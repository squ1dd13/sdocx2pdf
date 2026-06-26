use std::rc::Rc;

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

    pub fn create_egs(&self) -> lopdf::Dictionary {
        // fixme: Units
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
    /// `strokes` using this tool. The strokes are drawn in order.
    pub fn draw_events<'e>(
        &self,
        egs_name: &str,
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

        // todo: Draw translucent strokes by filling the stroke's bounding box and clipping
        // That should work perfectly if the colour is uniform along the stroke. It won't allow
        // us to vary the colour along the stroke (e.g., for the pencil with variable opacity),
        // but even then it's better than the current system.

        if self.is_straight_only() {
            for events in strokes {
                draw_events_straight(events, size, ops);
            }
        } else {
            let arc_mode = if self.is_like_highlighter() {
                // Straight ends for highlighter strokes.
                ArcMode::None
            } else if a == 255 {
                // Round ends and nice connections for opaque non-highlighter strokes.
                ArcMode::All
            } else {
                // Round ends only but no interior arcs for translucent non-highlighter strokes.
                ArcMode::FirstLastOnly
            };

            for events in strokes {
                draw_events_basic(events, size, arc_mode, ops);
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

    let bottom_to_top_left_points = [
        Normal(bottom_left),
        Control(cp_lower_mid_left),
        Control(cp_upper_mid_left),
        Normal(top_left),
    ];

    let top_to_bottom_right_points = [
        Normal(top_right),
        Control(cp_upper_mid_right),
        Control(cp_lower_mid_right),
        Normal(bottom_right),
    ];

    let bottom_arc_lowest = start_pos - scaled_tangent_start;
    let top_arc_highest = end_pos + scaled_tangent_end;

    let points = if let Some((
        top_left_to_arc_highest_cps,
        top_arc_highest_to_right_cps,
        bottom_right_to_arc_lowest_cps,
        bottom_arc_lowest_to_left_cps,
    )) = draw_arcs
        .then(|| {
            // If we can't calculate the control points for all four arcs, we won't draw any of
            // them. This is like a short-circuiting four-way zip that either gives us `None` or a
            // tuple containing all of the control points.
            bezier_arc_control_points(top_left, top_arc_highest, end_pos).and_then(|a| {
                bezier_arc_control_points(top_arc_highest, top_right, end_pos).and_then(|b| {
                    bezier_arc_control_points(bottom_right, bottom_arc_lowest, start_pos).and_then(
                        |c| {
                            bezier_arc_control_points(bottom_arc_lowest, bottom_left, start_pos)
                                .map(|d| (a, b, c, d))
                        },
                    )
                })
            })
        })
        .flatten()
    {
        // Points from bottom left to top left
        Either::Left(
            bottom_to_top_left_points
                .into_iter()
                // Control points for top left arc
                .chain(top_left_to_arc_highest_cps.map(Control))
                // Common point for top arcs
                .chain(std::iter::once(Normal(top_arc_highest)))
                // Control points for top right arc
                .chain(top_arc_highest_to_right_cps.map(Control))
                // Points from top right to bottom right
                .chain(top_to_bottom_right_points)
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
                            // Control points for bottom right arc
                            bottom_right_to_arc_lowest_cps
                                .map(Control)
                                .into_iter()
                                // Common point for bottom arcs
                                .chain(std::iter::once(Normal(bottom_arc_lowest)))
                                // Control points for bottom left arc
                                .chain(bottom_arc_lowest_to_left_cps.map(Control))
                                // Bottom left point (again - this time, to complete
                                // the curve)
                                .chain(std::iter::once(Normal(bottom_left))),
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
            bottom_to_top_left_points
                .into_iter()
                .chain(top_to_bottom_right_points),
        )
    };

    ops.extend(op_gen::draw_polygon(
        points,
        PolygonDrawMode::Fill(WindingRule::NonZero),
    ));

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

    ops.push(op_gen::save_graphics_state());

    ops.extend(op_gen::draw_polygon(
        points,
        PolygonDrawMode::Fill(WindingRule::NonZero),
    ));

    ops.push(op_gen::restore_graphics_state());

    Ok(())
}

fn draw_simple_line(
    [(a, radius_a), (b, radius_b)]: [(PdfPoint, f64); 2],
    round_ends: bool,
    ops: &mut Vec<lopdf::content::Operation>,
) {
    ops.extend([
        op_gen::save_graphics_state(),
        if round_ends {
            op_gen::set_line_cap_round()
        } else {
            op_gen::set_line_cap_butt()
        },
        op_gen::set_stroke_width((radius_a + radius_b) as f32),
    ]);

    ops.extend(op_gen::draw_line(a, b));
    ops.push(op_gen::restore_graphics_state());

    // todo: Draw line with width equal to the smaller radius and add a dot for the bigger radius?
}

#[derive(Clone, Copy)]
enum ArcMode {
    /// Do not draw any arcs.
    None,

    /// Draw arcs only on the first and last segments of the stroke.
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
    arc_mode: ArcMode,
    ops: &mut Vec<lopdf::content::Operation>,
) {
    let pen_size: f64 = pen_size.into();

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

        let is_quite_short = is_very_short || visual_length < start_spread + end_spread;

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
                ops,
            )
            .is_err()
        {
            last_segment_tan = Some(end_pos - start_pos);

            let points = [(start_pos, start_spread), (end_pos, end_spread)];

            // If the segment is quite short but not very short, try drawing a simple pulley.
            // If that doesn't work, or if the segment is very short, draw a simple line.
            if is_very_short || draw_simple_pulley(points, want_arcs, ops).is_err() {
                draw_simple_line(points, want_arcs, ops);
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
