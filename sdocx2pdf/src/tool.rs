use std::rc::Rc;

use euclid::{Point2D, Vector2D};
use itertools::Itertools;
use lerp::Lerp;
use ordered_float::OrderedFloat;
use printpdf::{BlendMode, ExtendedGraphicsState, ExtendedGraphicsStateId, LinePoint, Mm, Rgb};
use sdocx::page::object::stroke::{Event, Stroke};
use thiserror::Error;

use crate::stroke::{ContinuousStroke, StrokeOrDot};

struct PdfSpace;
type PdfPoint = Point2D<f64, PdfSpace>;
type PdfVector = Vector2D<f64, PdfSpace>;

/// Basic information used by all tools.
///
/// Though the `size` field is a float, this type implements `Hash` because the floats used are
/// expected to have been read directly from a document file. The size is calculated in the same
/// way for all tools, and thus two tools with logically equal sizes will have *exactly* equal
/// `size` fields.
#[derive(PartialEq, Eq, Hash, Clone)]
pub struct Basics {
    size: OrderedFloat<f32>,
    colour_bgra: [u8; 4],
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum Tool {
    FountainPen(Basics),
    CalligraphyPen(Basics),
    InkPen { basics: Basics, fixed_width: bool },
    Pencil { basics: Basics, fixed_opacity: bool },
    CalligraphyBrush(Basics),
    Highlighter(Basics),
    StraightHighlighter(Basics),
    Marker(Basics),
    StraightMarker(Basics),
}

#[derive(Error, Debug)]
enum ToolPropertyError {
    #[error("missing name")]
    NoName,

    #[error("unknown name '{0}'")]
    UnknownName(Rc<str>),

    #[error("missing size")]
    NoSize,

    #[error("missing colour")]
    NoColour,
}

impl Tool {
    fn try_for_stroke(stroke: &Stroke) -> Result<Tool, ToolPropertyError> {
        let name = stroke.pen_name().ok_or(ToolPropertyError::NoName)?;

        let basics = Basics {
            size: stroke.pen_size().ok_or(ToolPropertyError::NoSize)?.into(),
            colour_bgra: stroke.colour().ok_or(ToolPropertyError::NoColour)?,
        };

        Ok(match name.as_ref() {
            "com.samsung.android.sdk.pen.pen.preload.FountainPen" => Tool::FountainPen(basics),
            "com.samsung.android.sdk.pen.pen.preload.ObliquePen" => Tool::CalligraphyPen(basics),
            "com.samsung.android.sdk.pen.pen.preload.InkPen2" => Tool::InkPen {
                basics,
                fixed_width: stroke.is_fixed_width(),
            },
            "com.samsung.android.sdk.pen.pen.preload.Pencil2" => Tool::Pencil {
                basics,
                fixed_opacity: stroke.is_fixed_opacity(),
            },
            "com.samsung.android.sdk.pen.pen.preload.BrushPen" => Tool::CalligraphyBrush(basics),
            "com.samsung.android.sdk.pen.pen.preload.Marker4" => Tool::Highlighter(basics),
            "com.samsung.android.sdk.pen.pen.preload.StraightHighlighter" => {
                Tool::StraightHighlighter(basics)
            }
            "com.samsung.android.sdk.pen.pen.preload.Marker3" => Tool::Marker(basics),
            "com.samsung.android.sdk.pen.pen.preload.StraightMarker" => {
                Tool::StraightMarker(basics)
            }
            _ => return Err(ToolPropertyError::UnknownName(Rc::clone(name))),
        })
    }

    pub fn for_stroke(stroke: &Stroke) -> Tool {
        Self::try_for_stroke(stroke).unwrap()
    }

    fn basics(&self) -> &Basics {
        match self {
            Tool::FountainPen(basics)
            | Tool::CalligraphyPen(basics)
            | Tool::InkPen { basics, .. }
            | Tool::Pencil { basics, .. }
            | Tool::CalligraphyBrush(basics)
            | Tool::Highlighter(basics)
            | Tool::StraightHighlighter(basics)
            | Tool::Marker(basics)
            | Tool::StraightMarker(basics) => basics,
        }
    }

    /// Returns whether this tool is a highlighter or related tool which is typically used for
    /// drawing over existing text.
    fn is_like_highlighter(&self) -> bool {
        matches!(
            self,
            Tool::Highlighter(_)
                | Tool::StraightHighlighter(_)
                | Tool::Marker(_)
                | Tool::StraightMarker(_)
        )
    }

    /// Returns whether strokes drawn using this tool are always straight.
    fn is_straight_only(&self) -> bool {
        matches!(self, Tool::StraightHighlighter(_) | Tool::StraightMarker(_))
    }

    pub fn create_egs(&self) -> ExtendedGraphicsState {
        let mut egs = ExtendedGraphicsState::default().with_line_cap(printpdf::LineCapStyle::Round);

        let alpha = self.basics().colour_bgra[3];

        if alpha != 255 {
            let alpha = (alpha as f32) / 255.0;

            egs.set_current_fill_alpha(alpha);
            egs.set_current_stroke_alpha(alpha);

            if self.is_like_highlighter() {
                egs.set_blend_mode(BlendMode::multiply());
            }
        }

        // todo: Soft mask for pencil
        // (I don't think it can work with `printpdf` because the soft mask in EGS doesn't let us
        // provide dimensions for the mask we're using, which doesn't make much sense)

        egs
    }

    /// Extends `ops` with the necessary operations to draw each slice of stroke events in
    /// `strokes` using this tool. The strokes are drawn in order.
    pub fn draw_events<'e>(
        &self,
        egs_id: &ExtendedGraphicsStateId,
        strokes: impl IntoIterator<Item = impl IntoIterator<Item = &'e Event>>,
        ops: &mut Vec<printpdf::Op>,
    ) -> Result<(), ()> {
        let &Basics {
            size: OrderedFloat(size),
            colour_bgra: [b, g, r, a],
        } = self.basics();

        let colour = printpdf::Color::Rgb(Rgb::new(
            (r as f32) / 255.0,
            (g as f32) / 255.0,
            (b as f32) / 255.0,
            None,
        ));

        ops.extend([
            printpdf::Op::SaveGraphicsState,
            printpdf::Op::LoadGraphicsState { gs: egs_id.clone() },
            printpdf::Op::SetFillColor {
                col: colour.clone(),
            },
            printpdf::Op::SetOutlineColor { col: colour },
        ]);

        if self.is_straight_only() {
            for events in strokes {
                draw_events_straight(events, size, ops);
            }
        } else {
            let use_arcs = a == 255;

            for events in strokes {
                draw_events_basic(events, size, use_arcs, ops);
            }
        }

        ops.push(printpdf::Op::RestoreGraphicsState);

        Ok(())
    }
}

fn pdf_point_to_line_point(point: PdfPoint) -> LinePoint {
    LinePoint {
        p: printpdf::Point {
            x: Mm(point.x as f32).into(),
            y: Mm(point.y as f32).into(),
        },
        bezier: false,
    }
}

fn pdf_point_to_control_point(point: PdfPoint) -> LinePoint {
    LinePoint {
        p: printpdf::Point {
            x: Mm(point.x as f32).into(),
            y: Mm(point.y as f32).into(),
        },
        bezier: true,
    }
}

fn bezier_arc_control_points<T: num::Float, U>(
    a: Point2D<T, U>,
    b: Point2D<T, U>,
    centre: Point2D<T, U>,
) -> Option<[Point2D<T, U>; 2]> {
    let x1 = a.x;
    let y1 = a.y;
    let x4 = b.x;
    let y4 = b.y;
    let xc = centre.x;
    let yc = centre.y;
    let ax = x1 - xc;
    let ay = y1 - yc;
    let bx = x4 - xc;
    let by = y4 - yc;
    let q1 = ax * ax + ay * ay;
    let q2 = q1 + ax * bx + ay * by;
    let k2 = (((q1 * q2 * T::from(2).unwrap()).sqrt() - q2) * T::from(4).unwrap())
        / ((ax * by - ay * bx) * T::from(3).unwrap());
    let x2 = xc + ax - k2 * ay;
    let y2 = yc + ay + k2 * ax;
    let x3 = xc + bx + k2 * by;
    let y3 = yc + by - k2 * bx;

    if !x2.is_finite() || !y2.is_finite() || !x3.is_finite() || !y3.is_finite() {
        return None;
    }

    Some([(x2, y2).into(), (x3, y3).into()])
}

/// Returns the control points `(p1, p2)` for a cubic Bézier from `p0` to `p3` that passes through
/// `a` at `t = 1/3` and `b` at `t = 2/3`.
fn bezier_control_pts_for_intersections<U>(
    p0: Point2D<f64, U>,
    p3: Point2D<f64, U>,
    a: Point2D<f64, U>,
    b: Point2D<f64, U>,
) -> (Point2D<f64, U>, Point2D<f64, U>) {
    let (p0, p3, a, b) = (p0.to_vector(), p3.to_vector(), a.to_vector(), b.to_vector());

    // Solution to the linear system formed by `f(1/3) = a` and `f(2/3) = b` where f is the
    // function describing the cubic Bézier from `p0` to `p3`.
    let p2 = (b * 18.0 - a * 9.0 + p0 * 2.0 - p3 * 5.0) / 6.0;
    let p1 = (a * 27.0 - p0 * 8.0 - p3 - p2 * 6.0) / 12.0;

    (p1.to_point(), p2.to_point())
}

fn pressure_to_circle_radius(pressure: f64, pen_size: f64) -> f64 {
    0.5 * pen_size * pressure.clamp(0.4, 0.7)
}

/// Draws `events` into `ops` using the basic unmodified stroke segmentation algorithm and a simple
/// pressure-to-width conversion based on `pen_size`.
///
/// The event positions must be in PDF space. It is assumed that the outline colour and line cap
/// style are set up for drawing dots, and that the fill colour is set up for drawing segments.
/// This function does not change the colours at any point, and is therefore unsuitable for drawing
/// strokes with varying opacity. Tilt data, if present, is ignored.
///
/// `use_arcs` determines whether stroke segments are rounded at the end(s). Arcs allow for
/// smoother connections, but increase the file size. They also necessarily overlap the next
/// segment, making them inappropriate for strokes with transparency (because you can see the
/// arcs).
fn draw_events_basic<'e>(
    events: impl IntoIterator<Item = &'e Event>,
    pen_size: f32,
    use_arcs: bool,
    ops: &mut Vec<printpdf::Op>,
) {
    let pen_size: f64 = pen_size.into();

    let smooth = match StrokeOrDot::from_events(events) {
        StrokeOrDot::Stroke(stroke) => ContinuousStroke::new(&stroke),

        StrokeOrDot::Dot { x, y, pressure } => {
            let pos: PdfPoint = (x, y).into();
            let spread = pressure_to_circle_radius(pressure, pen_size);

            // Draw a filled circle.
            ops.extend([
                // Op::SetOutlineColor {
                //     col: Color::Rgb(Rgb::new(0.0, 1.0, 0.0, None)),
                // },
                // Op::SetLineCapStyle {
                //     cap: printpdf::LineCapStyle::Round,
                // },
                printpdf::Op::SetOutlineThickness {
                    pt: Mm(spread as f32 * 2.0).into(),
                },
                printpdf::Op::DrawLine {
                    line: printpdf::Line {
                        points: vec![pdf_point_to_line_point(pos), pdf_point_to_line_point(pos)],
                        is_closed: false,
                    },
                },
            ]);

            return;
        }
    };

    let target_angle = f64::to_radians(40.0);
    let sample_arc_lengths = smooth.sample_points(target_angle);

    let mut tangent_at_connection: Option<PdfVector> = None;

    for (s_start, s_end) in sample_arc_lengths.tuple_windows() {
        let start_pos: PdfPoint = smooth.position(s_start).into();
        let end_pos: PdfPoint = smooth.position(s_end).into();

        let start_pressure = smooth.pressure(s_start);
        let end_pressure = smooth.pressure(s_end);

        let s_first_third = s_start.lerp(s_end, 1.0 / 3.0);
        let s_second_third = s_start.lerp(s_end, 2.0 / 3.0);

        let (pressure_first_third, pressure_second_third) = (
            smooth.pressure(s_first_third),
            smooth.pressure(s_second_third),
        );

        let start_spread = pressure_to_circle_radius(start_pressure, pen_size);
        let spread_first_third = pressure_to_circle_radius(pressure_first_third, pen_size);
        let spread_second_third = pressure_to_circle_radius(pressure_second_third, pen_size);
        let end_spread = pressure_to_circle_radius(end_pressure, pen_size);

        let (
            scaled_tangent_start,
            scaled_tangent_first_third,
            scaled_tangent_second_third,
            scaled_tangent_end,
        ) = (
            {
                let tangent = PdfVector::from(smooth.unit_tangent(s_start)).normalize();

                if tangent.is_finite() {
                    tangent
                } else {
                    (end_pos - start_pos).normalize()
                }
            } * start_spread,
            PdfVector::from(smooth.unit_tangent(s_first_third)).normalize() * spread_first_third,
            PdfVector::from(smooth.unit_tangent(s_second_third)).normalize() * spread_second_third,
            {
                let tangent = PdfVector::from(smooth.unit_tangent(s_end)).normalize();

                if tangent.is_finite() {
                    tangent
                } else {
                    (end_pos - start_pos).normalize()
                }
            } * end_spread,
        );

        // todo: Handle the case where this assertion fails.
        assert!(
            scaled_tangent_start.is_finite()
                && scaled_tangent_first_third.is_finite()
                && scaled_tangent_second_third.is_finite()
                && scaled_tangent_end.is_finite()
        );

        let (pos_first_third, pos_second_third) = (
            PdfPoint::from(smooth.position(s_first_third)),
            PdfPoint::from(smooth.position(s_second_third)),
        );

        let start_to_first_third = (pos_first_third - start_pos).normalize();

        // If the computed start direction is in opposition to the direction to the
        // first third position, use the latter for the tangent so we aren't going back
        // on ourselves. `start_to_first_third` should be finite, but we check the
        // partial ordering so that if it is not, we retain the current finite tangent.
        let scaled_tangent_start = if let Some(std::cmp::Ordering::Less) = scaled_tangent_start
            .dot(start_to_first_third)
            .partial_cmp(&0.0)
        {
            start_to_first_third * start_spread
        } else {
            scaled_tangent_start
        };

        let bottom_left =
            start_pos + Vector2D::new(-scaled_tangent_start.y, scaled_tangent_start.x);

        let bottom_right =
            start_pos + Vector2D::new(scaled_tangent_start.y, -scaled_tangent_start.x);

        let lower_mid_left = pos_first_third
            + Vector2D::new(-scaled_tangent_first_third.y, scaled_tangent_first_third.x);

        let lower_mid_right = pos_first_third
            + Vector2D::new(scaled_tangent_first_third.y, -scaled_tangent_first_third.x);

        let upper_mid_left = pos_second_third
            + Vector2D::new(
                -scaled_tangent_second_third.y,
                scaled_tangent_second_third.x,
            );

        let upper_mid_right = pos_second_third
            + Vector2D::new(
                scaled_tangent_second_third.y,
                -scaled_tangent_second_third.x,
            );

        let top_left = end_pos + Vector2D::new(-scaled_tangent_end.y, scaled_tangent_end.x);

        let top_right = end_pos + Vector2D::new(scaled_tangent_end.y, -scaled_tangent_end.x);

        let (cp_lower_mid_left, cp_upper_mid_left) = bezier_control_pts_for_intersections(
            bottom_left,
            top_left,
            lower_mid_left,
            upper_mid_left,
        );

        let (cp_upper_mid_right, cp_lower_mid_right) = bezier_control_pts_for_intersections(
            top_right,
            bottom_right,
            upper_mid_right,
            lower_mid_right,
        );

        let bottom_to_top_left_points = [
            pdf_point_to_line_point(bottom_left),
            pdf_point_to_control_point(cp_lower_mid_left),
            pdf_point_to_control_point(cp_upper_mid_left),
            pdf_point_to_line_point(top_left),
        ];

        let top_to_bottom_right_points = [
            pdf_point_to_line_point(top_right),
            pdf_point_to_control_point(cp_upper_mid_right),
            pdf_point_to_control_point(cp_lower_mid_right),
            pdf_point_to_line_point(bottom_right),
        ];

        let bottom_arc_lowest = start_pos - scaled_tangent_start;
        let top_arc_highest = end_pos + scaled_tangent_end;

        let points = if let Some((
            top_left_to_arc_highest_cps,
            top_arc_highest_to_right_cps,
            bottom_right_to_arc_lowest_cps,
            bottom_arc_lowest_to_left_cps,
        )) = use_arcs
            .then(|| {
                // If we can't calculate the control points for all four arcs, we won't
                // draw any of them. This is like a short-circuiting four-way zip that
                // either gives us `None` or a tuple containing all of the control
                // points.
                bezier_arc_control_points(top_left, top_arc_highest, end_pos).and_then(|a| {
                    bezier_arc_control_points(top_arc_highest, top_right, end_pos).and_then(|b| {
                        bezier_arc_control_points(bottom_right, bottom_arc_lowest, start_pos)
                            .and_then(|c| {
                                bezier_arc_control_points(bottom_arc_lowest, bottom_left, start_pos)
                                    .map(|d| (a, b, c, d))
                            })
                    })
                })
            })
            .flatten()
        {
            // Points from bottom left to top left
            bottom_to_top_left_points
                .into_iter()
                // Control points for top left arc
                .chain(top_left_to_arc_highest_cps.map(pdf_point_to_control_point))
                // Common point for top arcs
                .chain(std::iter::once(pdf_point_to_line_point(top_arc_highest)))
                // Control points for top right arc
                .chain(top_arc_highest_to_right_cps.map(pdf_point_to_control_point))
                // Points from top right to bottom right
                .chain(top_to_bottom_right_points)
                .chain({
                    let it = if tangent_at_connection.is_none_or(|tangent| {
                        tangent.angle_to(scaled_tangent_start).radians.abs() > f64::to_radians(20.0)
                    }) {
                        // Either this is the first segment, in which case we need to
                        // round the beginning, or there is a significant difference in
                        // tangent angle between the final third of the previous segment
                        // and the start of this one. In the latter case, we round the
                        // beginning to make the connection look cleaner.
                        Some(
                            // Control points for bottom right arc
                            bottom_right_to_arc_lowest_cps
                                .map(pdf_point_to_control_point)
                                .into_iter()
                                // Common point for bottom arcs
                                .chain(std::iter::once(pdf_point_to_line_point(bottom_arc_lowest)))
                                // Control points for bottom left arc
                                .chain(
                                    bottom_arc_lowest_to_left_cps.map(pdf_point_to_control_point),
                                )
                                // Bottom left point (again - this time, to complete
                                // the curve)
                                .chain(std::iter::once(pdf_point_to_line_point(bottom_left))),
                        )
                    } else {
                        None
                    }
                    .into_iter()
                    .flatten();

                    tangent_at_connection = Some(end_pos - pos_second_third);

                    it
                })
                .collect_vec()
        } else {
            bottom_to_top_left_points
                .into_iter()
                .chain(top_to_bottom_right_points)
                .collect_vec()
        };

        // let true_angle = scaled_tangent_start
        //     .angle_to(scaled_tangent_end)
        //     .radians
        //     .abs();

        // let col = Color::Rgb(Rgb::new(
        //     0.0, 0.0, 0.0,
        //     // if used_fallback_start_tangent {
        //     //     1.0
        //     // } else {
        //     //     0.0
        //     // },
        //     // 0.0,
        //     // if true_angle > 1.5 * target_angle {
        //     //     1.0
        //     // } else {
        //     //     0.0
        //     // },
        //     None,
        // ));

        ops.extend([
            // Op::SetFillColor { col: col.clone() },
            // Op::SetOutlineColor { col },
            // Op::SetOutlineThickness {
            //     pt: Mm(0.05).into(),
            // },
            printpdf::Op::DrawPolygon {
                polygon: printpdf::Polygon {
                    rings: vec![printpdf::PolygonRing { points }],
                    mode: printpdf::PaintMode::Fill,
                    winding_order: printpdf::WindingOrder::NonZero,
                },
            },
            // // Set things up for drawing the points of interest.
            // Op::SetLineCapStyle {
            //     cap: printpdf::LineCapStyle::Round,
            // },
        ]);

        // page_contents.extend(smooth.features().into_iter().flat_map(|feature| {
        //     let s = feature.arc_length();

        //     let pos = tx.transform_point(smooth.position(s).into());

        //     [
        //         Op::SetOutlineColor {
        //             col: Color::Rgb(match feature {
        //                 Feature::Start => Rgb::new(1.0, 0.5, 0.0, None),
        //                 Feature::Vertex(_) => Rgb::new(0.0, 1.0, 0.0, None),
        //                 Feature::Inflection(_) => Rgb::new(1.0, 1.0, 0.0, None),
        //                 Feature::End(_) => Rgb::new(0.0, 0.5, 1.0, None),
        //             }),
        //         },
        //         Op::SetOutlineThickness {
        //             pt: Mm(pressure_to_circle_radius(smooth.pressure(s), pen_size)
        //                 as f32
        //                 * 2.0
        //                 * 0.25)
        //             .into(),
        //         },
        //         Op::DrawLine {
        //             line: printpdf::Line {
        //                 points: vec![
        //                     pdf_point_to_line_point(pos),
        //                     pdf_point_to_line_point(pos),
        //                 ],
        //                 is_closed: false,
        //             },
        //         },
        //     ]
        // }));
    }
}

fn draw_events_straight<'e>(
    events: impl IntoIterator<Item = &'e Event>,
    pen_size: f32,
    ops: &mut Vec<printpdf::Op>,
) {
    let mut events = events.into_iter();

    let first = events.next().unwrap();
    let last = events.last().unwrap();

    let a = pdf_point_to_line_point(<(f64, f64)>::from(first.point).into());
    let b = pdf_point_to_line_point(<(f64, f64)>::from(last.point).into());

    ops.extend([
        printpdf::Op::SetOutlineThickness {
            pt: Mm(pen_size).into(),
        },
        printpdf::Op::DrawLine {
            line: printpdf::Line {
                points: vec![a, b],
                is_closed: false,
            },
        },
    ]);
}
