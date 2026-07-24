use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use euclid::{Point2D, Vector2D};
use itertools::{Either, Itertools};
use lerp::Lerp;
use lopdf::dictionary;
use ordered_float::OrderedFloat;
use sdocx::page::object::stroke::{Event, Stroke};
use thiserror::Error;

use crate::{
    op_gen::{self, PdfPoint, PdfVector, PolygonDrawMode, WindingRule},
    stroke::{ContinuousStroke, StrokeOrDot},
};

/// Counts strokes whose colour field is missing from the document (see `try_for_stroke`). These
/// are drawn in black; the total is reported once by `report_missing_colours` after conversion.
static MISSING_COLOUR_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Emits a single warning summarising how many strokes had no colour and were drawn in black.
/// Does nothing if every stroke had a colour. Call once after all strokes have been processed.
pub fn report_missing_colours() {
    let count = MISSING_COLOUR_COUNT.load(Ordering::Relaxed);
    if count > 0 {
        eprintln!(
            "Warning: {}; falling back to black. ({count})",
            ToolPropertyError::NoColour
        );
    }
}

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

        // Some strokes (e.g. those drawn with InkPen2) do not serialise a colour field at all,
        // so `stroke.colour()` returns `None`. Rather than aborting the whole conversion, count
        // the missing colour (preserving the original NoColour signal) and fall back to opaque
        // black, which is the correct colour for the vast majority of ink strokes. The count is
        // reported once via `report_missing_colours` instead of warning per stroke, since
        // documents can contain thousands of such strokes.
        let colour_bgra = match stroke.colour() {
            Some(colour) => colour,
            None => {
                MISSING_COLOUR_COUNT.fetch_add(1, Ordering::Relaxed);
                [0, 0, 0, 255]
            }
        };

        let basics = Basics {
            size: stroke.pen_size().ok_or(ToolPropertyError::NoSize)?.into(),
            colour_bgra,
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

    fn is_pressure_sensitive(&self) -> bool {
        matches!(
            self,
            Tool::FountainPen(_)
                | Tool::InkPen {
                    fixed_width: false,
                    ..
                }
                | Tool::Pencil {
                    fixed_opacity: false,
                    ..
                }
        )
    }

    /// Returns whether strokes drawn using this tool are always straight.
    fn is_straight_only(&self) -> bool {
        matches!(self, Tool::StraightHighlighter(_) | Tool::StraightMarker(_))
    }

    pub fn create_egs(&self) -> lopdf::Dictionary {
        let mut dict = lopdf::dictionary! {
            "Type" => "ExtGState",
            // Round line cap style
            "LC" => 1,
        };

        let alpha = self.basics().colour_bgra[3];

        if alpha != 255 {
            let alpha = (alpha as f32) / 255.0;

            // Stroke alpha
            dict.set("CA", alpha);
            // Fill alpha
            dict.set("ca", alpha);

            if self.is_like_highlighter() {
                // Multiply blend mode
                dict.set("BM", lopdf::Object::Name(b"Multiply".to_vec()));
            }
        }

        // todo: Soft masks for pencil and calligraphy brush

        dict
    }

    /// Extends `ops` with the necessary operations to draw each slice of stroke events in
    /// `strokes` using this tool. The strokes are drawn in order. Translucent strokes are drawn by
    /// filling a `page_size` rectangle and clipping it to the shape of the stroke.
    pub fn draw_events<'e>(
        &self,
        egs_name: &str,
        page_size: (f32, f32),
        strokes: impl IntoIterator<Item = impl IntoIterator<Item = &'e Event>>,
        ops: &mut Vec<lopdf::content::Operation>,
    ) -> Result<(), ()> {
        let &Basics {
            size: OrderedFloat(size),
            colour_bgra: [b, g, r, a],
        } = self.basics();

        ops.extend([
            op_gen::save_graphics_state(),
            op_gen::load_graphics_state(egs_name),
            op_gen::set_fill_colour(r, g, b),
            op_gen::set_stroke_colour(r, g, b),
        ]);

        if self.is_straight_only() {
            for events in strokes {
                draw_events_straight(events, size, ops);
            }
        } else {
            let specify_only = a != 255;

            let effective_size = if self.is_like_highlighter() {
                size * 2.5
            } else {
                size
            };

            let pressure_override = (!self.is_pressure_sensitive()).then_some(0.45);

            if specify_only {
                for events in strokes {
                    ops.push(op_gen::save_graphics_state());

                    draw_events_basic(
                        events,
                        effective_size,
                        pressure_override,
                        ArcMode::All,
                        specify_only,
                        ops,
                    );

                    ops.extend(op_gen::clip(WindingRule::NonZero));

                    ops.extend([
                        op_gen::specify_rectangle([0.0, 0.0, page_size.0, page_size.1]),
                        op_gen::fill(),
                        op_gen::restore_graphics_state(),
                    ]);
                }
            } else {
                for events in strokes {
                    draw_events_basic(
                        events,
                        effective_size,
                        pressure_override,
                        ArcMode::All,
                        specify_only,
                        ops,
                    );
                }
            }
        }

        ops.push(op_gen::restore_graphics_state());

        Ok(())
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

fn draw_bezier_pulley(
    points_tangents_radii: [(PdfPoint, PdfVector, f64); 4],
    draw_arcs: bool,
    last_segment_tan: &mut Option<PdfVector>,
    specify_only: bool,
    ops: &mut Vec<lopdf::content::Operation>,
) -> Result<(), ()> {
    let [
        (start_pos, start_tangent, start_spread),
        (pos_first_third, tangent_first_third, spread_first_third),
        (pos_second_third, tangent_second_third, spread_second_third),
        (end_pos, end_tangent, end_spread),
    ] = points_tangents_radii;

    let (
        scaled_tangent_start,
        scaled_tangent_first_third,
        scaled_tangent_second_third,
        scaled_tangent_end,
    ) = (
        start_tangent * start_spread,
        tangent_first_third * spread_first_third,
        tangent_second_third * spread_second_third,
        end_tangent * end_spread,
    );

    if !scaled_tangent_start.is_finite()
        || !scaled_tangent_first_third.is_finite()
        || !scaled_tangent_second_third.is_finite()
        || !scaled_tangent_end.is_finite()
    {
        return Err(());
    }

    let start_to_first_third = (pos_first_third - start_pos).normalize();

    // If the computed start direction is in opposition to the direction to the first third
    // position, use the latter for the tangent so we aren't going back on ourselves.
    // `start_to_first_third` should be finite, but we check the partial ordering so that if it is
    // not, we retain the current finite tangent.
    let scaled_tangent_start = if let Some(std::cmp::Ordering::Less) = scaled_tangent_start
        .dot(start_to_first_third)
        .partial_cmp(&0.0)
    {
        start_to_first_third * start_spread
    } else {
        scaled_tangent_start
    };

    let bottom_left = start_pos + Vector2D::new(-scaled_tangent_start.y, scaled_tangent_start.x);

    let bottom_right = start_pos + Vector2D::new(scaled_tangent_start.y, -scaled_tangent_start.x);

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

    let (cp_lower_mid_left, cp_upper_mid_left) =
        bezier_control_pts_for_intersections(bottom_left, top_left, lower_mid_left, upper_mid_left);

    let (cp_upper_mid_right, cp_lower_mid_right) = bezier_control_pts_for_intersections(
        top_right,
        bottom_right,
        upper_mid_right,
        lower_mid_right,
    );

    use op_gen::PolygonPoint::{Control, Normal};

    let top_to_bottom_left_points = [
        Normal(top_left),
        Control(cp_upper_mid_left),
        Control(cp_lower_mid_left),
        Normal(bottom_left),
    ];

    let bottom_to_top_right_points = [
        Normal(bottom_right),
        Control(cp_lower_mid_right),
        Control(cp_upper_mid_right),
        Normal(top_right),
    ];

    let bottom_arc_lowest = start_pos - scaled_tangent_start;
    let top_arc_highest = end_pos + scaled_tangent_end;

    let points = if draw_arcs {
        let (
            top_right_to_arc_highest_cps,
            top_arc_highest_to_left_cps,
            bottom_left_to_arc_lowest_cps,
            bottom_arc_lowest_to_right_cps,
        ) = (
            bezier_arc_control_points(top_right, top_arc_highest, end_pos).ok_or(())?,
            bezier_arc_control_points(top_arc_highest, top_left, end_pos).ok_or(())?,
            bezier_arc_control_points(bottom_left, bottom_arc_lowest, start_pos).ok_or(())?,
            bezier_arc_control_points(bottom_arc_lowest, bottom_right, start_pos).ok_or(())?,
        );

        // Points from bottom left to top left
        Either::Left(
            bottom_to_top_right_points
                .into_iter()
                .chain(top_right_to_arc_highest_cps.map(Control))
                .chain(std::iter::once(Normal(top_arc_highest)))
                .chain(top_arc_highest_to_left_cps.map(Control))
                .chain(top_to_bottom_left_points)
                .chain({
                    let it = if last_segment_tan.is_none_or(|tangent| {
                        tangent.angle_to(scaled_tangent_start).radians.abs() > f64::to_radians(20.0)
                    }) {
                        // Either this is the first segment, in which case we need to round the
                        // beginning, or there is a significant difference in tangent angle between
                        // the final third of the previous segment and the start of this one. In
                        // the latter case, we round the beginning to make the connection look
                        // cleaner.
                        Some(
                            bottom_left_to_arc_lowest_cps
                                .map(Control)
                                .into_iter()
                                .chain(std::iter::once(Normal(bottom_arc_lowest)))
                                .chain(bottom_arc_lowest_to_right_cps.map(Control))
                                .chain(std::iter::once(Normal(bottom_right))),
                        )
                    } else {
                        None
                    }
                    .into_iter()
                    .flatten();

                    *last_segment_tan = Some(end_pos - pos_second_third);

                    it
                }),
        )
    } else {
        Either::Right(
            bottom_to_top_right_points
                .into_iter()
                .chain(top_to_bottom_left_points),
        )
    };

    if specify_only {
        ops.extend(op_gen::specify_polygon(points));
    } else {
        ops.extend(op_gen::draw_polygon(
            points,
            PolygonDrawMode::Fill(WindingRule::NonZero),
        ));
    }

    Ok(())
}

fn calc_pulley_line_points_acw_from_lower_right(
    c1: PdfPoint,
    r1: f64,
    c2: PdfPoint,
    r2: f64,
) -> Option<[PdfPoint; 4]> {
    let d = c1.distance_to(c2);

    if d == 0.0 {
        return None;
    }

    let alpha = (c2 - c1).angle_from_x_axis().radians;
    let beta = ((r1 - r2) / d).acos();

    if !beta.is_finite() {
        // (r1-r2)/d is not in [-1,1], i.e. one circle is inside the other.
        return None;
    }

    let (apb_s, apb_c) = (alpha + beta).sin_cos();
    let (amb_s, amb_c) = (alpha - beta).sin_cos();

    let apb = PdfVector::new(apb_c, apb_s);
    let amb = PdfVector::new(amb_c, amb_s);

    let right_start = c1 + amb * r1;
    let right_end = c2 + amb * r2;

    let left_start = c2 + apb * r2;
    let left_end = c1 + apb * r1;

    Some([right_start, right_end, left_start, left_end])
}

fn draw_simple_pulley(
    [(a, radius_a), (b, radius_b)]: [(PdfPoint, f64); 2],
    use_arcs: bool,
    specify_only: bool,
    ops: &mut Vec<lopdf::content::Operation>,
) -> Result<(), ()> {
    let [a_right, b_right, b_left, a_left] =
        calc_pulley_line_points_acw_from_lower_right(a, radius_a, b, radius_b).ok_or(())?;

    let direction = (b - a).normalize();

    if !direction.is_finite() {
        return Err(());
    }

    use op_gen::PolygonPoint::{Control, Normal};

    let points = if use_arcs {
        let a_arc_midpoint = a - direction * radius_a;
        let b_arc_midpoint = b + direction * radius_b;

        let [b_right_arc_cp1, b_right_arc_cp2] =
            bezier_arc_control_points(b_right, b_arc_midpoint, b).ok_or(())?;
        let [b_left_arc_cp1, b_left_arc_cp2] =
            bezier_arc_control_points(b_arc_midpoint, b_left, b).ok_or(())?;
        let [a_left_arc_cp1, a_left_arc_cp2] =
            bezier_arc_control_points(a_left, a_arc_midpoint, a).ok_or(())?;
        let [a_right_arc_cp1, a_right_arc_cp2] =
            bezier_arc_control_points(a_arc_midpoint, a_right, a).ok_or(())?;

        Either::Left(
            [
                Normal(b_right),
                Control(b_right_arc_cp1),
                Control(b_right_arc_cp2),
                Normal(b_arc_midpoint),
                Control(b_left_arc_cp1),
                Control(b_left_arc_cp2),
                Normal(b_left),
                Normal(a_left),
                Control(a_left_arc_cp1),
                Control(a_left_arc_cp2),
                Normal(a_arc_midpoint),
                Control(a_right_arc_cp1),
                Control(a_right_arc_cp2),
                Normal(a_right),
            ]
            .into_iter(),
        )
    } else {
        Either::Right(
            [
                Normal(b_right),
                Normal(b_left),
                Normal(a_left),
                Normal(a_right),
            ]
            .into_iter(),
        )
    };

    if specify_only {
        ops.extend(op_gen::specify_polygon(points));
    } else {
        ops.extend(op_gen::draw_polygon(
            points,
            PolygonDrawMode::Fill(WindingRule::NonZero),
        ));
    }

    Ok(())
}

fn draw_simple_line(
    [(a, radius_a), (b, radius_b)]: [(PdfPoint, f64); 2],
    round_ends: bool,
    specify_only: bool,
    ops: &mut Vec<lopdf::content::Operation>,
) {
    if specify_only {
        use op_gen::PolygonPoint::{Control, Normal};

        let forwards = (b - a).normalize();

        let left: PdfVector = (-forwards.y, forwards.x).into();
        let right = -left;

        if !forwards.is_finite() {
            if !round_ends {
                // A zero-length line with butt caps is invisible.
                return;
            }

            // The points are equal. We need to draw a circle, but as we are specifying it as a
            // path, we can only approximate it using Bézier curves.
            let radius = (radius_a + radius_b) / 2.0;

            let left = a + PdfVector::new(-radius, 0.0);
            let right = a + PdfVector::new(radius, 0.0);
            let top = a + PdfVector::new(0.0, radius);
            let bottom = a + PdfVector::new(0.0, -radius);

            // Calculate the control points for the arc in each quadrant.
            // todo: Precompute these for the unit circle and translate as needed instead of
            // calculating them every time.
            let [q1_c1, q1_c2] = bezier_arc_control_points(right, top, a).unwrap();
            let [q2_c1, q2_c2] = bezier_arc_control_points(top, left, a).unwrap();
            let [q3_c1, q3_c2] = bezier_arc_control_points(left, bottom, a).unwrap();
            let [q4_c1, q4_c2] = bezier_arc_control_points(bottom, right, a).unwrap();

            ops.extend(op_gen::specify_polygon([
                Normal(right),
                Control(q1_c1),
                Control(q1_c2),
                Normal(top),
                Control(q2_c1),
                Control(q2_c2),
                Normal(left),
                Control(q3_c1),
                Control(q3_c2),
                Normal(bottom),
                Control(q4_c1),
                Control(q4_c2),
                Normal(right),
            ]));

            return;
        }

        let bottom_right = a + right * radius_a;
        let top_right = b + right * radius_b;
        let top_left = b + left * radius_b;
        let bottom_left = a + left * radius_a;

        if !round_ends {
            // A zero-length line with butt caps is invisible.
            if !forwards.is_finite() {
                return;
            }

            ops.extend(op_gen::specify_polygon([
                Normal(bottom_right),
                Normal(top_right),
                Normal(top_left),
                Normal(bottom_left),
            ]));

            return;
        }

        // Non-zero length line with round ends. We could make this look like the normal case where
        // we take the mean radius, but since we have to draw a path instead of a line, we might as
        // well take advantage of the fact we can use different widths at the start and end.
        let bottom_arc_lowest = a - forwards * radius_a;
        let top_arc_highest = b + forwards * radius_b;

        let (
            [top_right_to_arc_highest_cp1, top_right_to_arc_highest_cp2],
            [top_arc_highest_to_left_cp1, top_arc_highest_to_left_cp2],
            [bot_left_to_arc_lowest_cp1, bot_left_to_arc_lowest_cp2],
            [bot_arc_lowest_to_right_cp1, bot_arc_lowest_to_right_cp2],
        ) = (
            bezier_arc_control_points(top_right, top_arc_highest, b).unwrap(),
            bezier_arc_control_points(top_arc_highest, top_left, b).unwrap(),
            bezier_arc_control_points(bottom_left, bottom_arc_lowest, a).unwrap(),
            bezier_arc_control_points(bottom_arc_lowest, bottom_right, a).unwrap(),
        );

        ops.extend(op_gen::specify_polygon([
            Normal(top_right),
            Control(top_right_to_arc_highest_cp1),
            Control(top_right_to_arc_highest_cp2),
            Normal(top_arc_highest),
            Control(top_arc_highest_to_left_cp1),
            Control(top_arc_highest_to_left_cp2),
            Normal(top_left),
            Normal(bottom_left),
            Control(bot_left_to_arc_lowest_cp1),
            Control(bot_left_to_arc_lowest_cp2),
            Normal(bottom_arc_lowest),
            Control(bot_arc_lowest_to_right_cp1),
            Control(bot_arc_lowest_to_right_cp2),
            Normal(bottom_right),
        ]));

        return;
    }

    ops.extend([
        op_gen::save_graphics_state(),
        if round_ends {
            op_gen::set_line_cap_round()
        } else {
            op_gen::set_line_cap_butt()
        },
        // The effective radius is the mean of the radii at `a` and `b`, so the _width_ of the line
        // is the sum.
        op_gen::set_stroke_width((radius_a + radius_b) as f32),
    ]);

    ops.extend(op_gen::draw_line(a, b));
    ops.push(op_gen::restore_graphics_state());

    // todo: Draw line with width equal to the smaller radius and add a dot for the bigger radius?
}

#[derive(Clone, Copy)]
enum ArcMode {
    /// Do not draw any arcs.
    #[expect(dead_code)]
    None,

    /// Draw arcs only on the first and last segments of the stroke.
    #[expect(dead_code)]
    FirstLastOnly,

    /// Draw arcs on all segments.
    All,
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
    pressure_override: Option<f64>,
    arc_mode: ArcMode,
    specify_only: bool,
    ops: &mut Vec<lopdf::content::Operation>,
) {
    let pen_size: f64 = pen_size.into();

    let pressure_to_circle_radius =
        |p: f64, s: f64| -> f64 { pressure_to_circle_radius(pressure_override.unwrap_or(p), s) };

    let smooth = match StrokeOrDot::from_events(events) {
        StrokeOrDot::Stroke(stroke) => ContinuousStroke::new(&stroke),

        StrokeOrDot::Dot { x, y, pressure } => {
            let pos: PdfPoint = (x, y).into();
            let spread = pressure_to_circle_radius(pressure, pen_size);

            // Draw a filled circle.
            ops.push(op_gen::set_stroke_width(spread as f32 * 2.0));
            ops.extend(op_gen::draw_line(pos, pos));

            return;
        }
    };

    let target_angle = f64::to_radians(40.0);
    let sample_arc_lengths = smooth.sample_points(target_angle);

    let mut last_segment_tan: Option<PdfVector> = None;

    for (iter_pos, (start_s, end_s)) in sample_arc_lengths.tuple_windows().with_position() {
        let start_pos: PdfPoint = smooth.position(start_s).into();
        let end_pos: PdfPoint = smooth.position(end_s).into();

        let start_spread = pressure_to_circle_radius(smooth.pressure(start_s), pen_size);
        let end_spread = pressure_to_circle_radius(smooth.pressure(end_s), pen_size);

        let visual_length = 0.5 * start_spread + (end_pos - start_pos).length() + 0.5 * end_spread;

        let is_very_short =
            visual_length < start_spread.max(end_spread) + start_spread.min(end_spread) * 0.1;

        let is_quite_short = is_very_short || visual_length < 0.7 * (start_spread + end_spread);

        let want_arcs = match arc_mode {
            ArcMode::All => true,
            ArcMode::FirstLastOnly => !matches!(iter_pos, itertools::Position::Middle),
            ArcMode::None => false,
        };

        // If the segment is not short, try drawing a Bézier pulley. If that doesn't work, or
        // if the segment is short, use a simpler method.
        if is_quite_short
            || draw_bezier_pulley(
                {
                    let first_third_s = start_s.lerp(end_s, 1.0 / 3.0);
                    let second_third_s = start_s.lerp(end_s, 2.0 / 3.0);

                    let first_third_pos: PdfPoint = smooth.position(first_third_s).into();
                    let second_third_pos: PdfPoint = smooth.position(second_third_s).into();

                    [
                        (
                            start_pos,
                            Some(PdfVector::from(smooth.unit_tangent(start_s)).normalize())
                                .filter(|v| v.is_finite())
                                .unwrap_or_else(|| (first_third_pos - start_pos).normalize()),
                            start_spread,
                        ),
                        (
                            first_third_pos,
                            Some(PdfVector::from(smooth.unit_tangent(first_third_s)).normalize())
                                .filter(|v| v.is_finite())
                                // Note that we use the same fallback tangent for both middle
                                // thirds.
                                .unwrap_or_else(|| {
                                    (second_third_pos - first_third_pos).normalize()
                                }),
                            pressure_to_circle_radius(smooth.pressure(first_third_s), pen_size),
                        ),
                        (
                            second_third_pos,
                            Some(PdfVector::from(smooth.unit_tangent(second_third_s)).normalize())
                                .filter(|v| v.is_finite())
                                .unwrap_or_else(|| {
                                    (second_third_pos - first_third_pos).normalize()
                                }),
                            pressure_to_circle_radius(smooth.pressure(second_third_s), pen_size),
                        ),
                        (
                            end_pos,
                            Some(PdfVector::from(smooth.unit_tangent(end_s)).normalize())
                                .filter(|v| v.is_finite())
                                .unwrap_or_else(|| (end_pos - second_third_pos).normalize()),
                            end_spread,
                        ),
                    ]
                },
                want_arcs,
                &mut last_segment_tan,
                specify_only,
                ops,
            )
            .is_err()
        {
            last_segment_tan = Some(end_pos - start_pos);

            let points = [(start_pos, start_spread), (end_pos, end_spread)];

            // If the segment is quite short but not very short, try drawing a simple pulley.
            // If that doesn't work, or if the segment is very short, draw a simple line.
            if is_very_short || draw_simple_pulley(points, want_arcs, specify_only, ops).is_err() {
                draw_simple_line(points, want_arcs, specify_only, ops);
            }
        }
    }
}

fn draw_events_straight<'e>(
    events: impl IntoIterator<Item = &'e Event>,
    pen_size: f32,
    ops: &mut Vec<lopdf::content::Operation>,
) {
    let mut events = events.into_iter();

    let first = events.next().unwrap();
    let last = events.last().unwrap();

    ops.push(op_gen::set_stroke_width(pen_size));
    ops.extend(op_gen::draw_line(
        <(f64, f64)>::from(first.point).into(),
        <(f64, f64)>::from(last.point).into(),
    ));
}
