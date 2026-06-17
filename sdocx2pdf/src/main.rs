use std::{ops::Div, os::unix::fs::MetadataExt};

use euclid::{Angle, Point2D, Vector2D, Vector3D};
use itertools::{Either, Itertools, Position};
use lerp::Lerp;
use printpdf::{
    Color, Line, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage, PdfSaveOptions, Polygon,
    PolygonRing, Rgb, WindingOrder,
};
use sdocx::page::object::stroke::{Event, Stroke};

use crate::stroke::{FilteredStroke, InterpolatedStroke};

struct PdfSpace;
type PdfPoint = Point2D<f64, PdfSpace>;
type PdfVector = Vector2D<f64, PdfSpace>;

mod stroke;

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

fn calc_pulley_line_points_acw<T: num::Float + euclid::Trig, U>(
    c1: Point2D<T, U>,
    r1: T,
    c2: Point2D<T, U>,
    r2: T,
) -> Option<[Point2D<T, U>; 4]> {
    let d = c1.distance_to(c2);

    if d.is_zero() {
        return None;
    }

    let alpha = (c2 - c1).angle_from_x_axis();
    let beta = Angle::radians(((r1 - r2) / d).acos());

    if !beta.is_finite() {
        // (r1-r2)/d is not in [-1,1], i.e. one circle is inside the other.
        // todo: Draw such events as single circles.
        return None;
    }

    let (apb_s, apb_c) = (alpha + beta).sin_cos();
    let (amb_s, amb_c) = (alpha - beta).sin_cos();

    let apb = Vector2D::<T, U>::new(apb_c, apb_s);
    let amb = Vector2D::<T, U>::new(amb_c, amb_s);

    let right_start = c1 + amb * r1;
    let right_end = c2 + amb * r2;

    let left_start = c2 + apb * r2;
    let left_end = c1 + apb * r1;

    Some([right_start, right_end, left_start, left_end])
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

fn main() {
    // sdocx::test_all();

    let document = sdocx::Document::from_zip(
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010.sdocx",
    )
    .unwrap();

    let name = document
        .title_text()
        .raw_string()
        .unwrap_or("Unnamed document");

    eprintln!("Name is '{name}'");

    let mut pdf = PdfDocument::new(name);

    match document.page_model() {
        sdocx::PageModel::Paged => eprintln!("This is a paged document"),
        sdocx::PageModel::Pageless => eprintln!("This is a pageless document"),
    };

    let (w, h) = document.width_height();
    eprintln!("w = {w}, h = {h}");

    let mut event_count = 0_usize;
    let mut polygon_count = 0_usize;
    let mut used_event_count = 0_usize;

    for page in document.pages() {
        // fixme: Document units are pixels, so we shouldn't be treating them as mm because it
        // creates huge dimensions.
        let (w, h) = page.width_height();

        let mut page_contents = vec![];

        for layer in page.layers() {
            let objects = layer.objects();
            let obj_count = objects.len() as f64;

            // todo: Filter for strokes only, then group by pen properties so we can create
            // an ExtendedGraphicsState for each pen and use that rather than writing out explicit
            // properties each time.
            for (obj_i, object) in layer.objects().iter().enumerate() {
                eprintln!(
                    "Processing objects: {:.1}% ({} of {})",
                    ((obj_i + 1) as f64 / obj_count) * 100.0,
                    obj_i + 1,
                    obj_count
                );

                let sdocx::DocObject::Stroke(stroke) = object else {
                    continue;
                };

                let interpolated = InterpolatedStroke::from_events(stroke.events());

                // fixme: Think carefully about how many samples to take here
                let smooth =
                    FilteredStroke::new(&interpolated, 5.5, 7.9, stroke.events().len() * 2)
                        .unwrap();

                // let (min_curvature, max_curvature) = derivs
                //     .curvature
                //     .iter()
                //     .minmax_by(|a, b| a.total_cmp(b))
                //     .into_option()
                //     .unwrap();

                // let curvature_span = max_curvature - min_curvature;

                event_count += stroke.events().len();

                let pen_size = stroke.pen_size().map(f64::from).unwrap_or(1.0);

                // Convert from document space, with y=0 at the top, to PDF space, with y=0 at the
                // bottom.
                let tx = euclid::Transform2D::<f64, (), PdfSpace>::scale(1.0, -1.0)
                    .then_translate(PdfVector::new(0.0, h.into()));

                let target_angle = f64::to_radians(15.0);
                let min_space_step = 2.0;
                let max_time_step = 50.0;

                let sample_times =
                    smooth.compute_sample_times(target_angle, min_space_step, max_time_step);

                // let rings = (0..derivs.t.len())
                for (t_start, t_end) in sample_times.tuple_windows() {
                    // eprintln!("dt = {}", t_end - t_start);

                    // .flat_map(|(i_start, i_end)| {
                    // A single event, ish
                    used_event_count += 1;

                    let start_pos = tx.transform_point(
                        (
                            smooth.x.evaluate(t_start).unwrap(),
                            smooth.y.evaluate(t_start).unwrap(),
                        )
                            .into(),
                    );

                    let end_pos = tx.transform_point(
                        (
                            smooth.x.evaluate(t_end).unwrap(),
                            smooth.y.evaluate(t_end).unwrap(),
                        )
                            .into(),
                    );

                    let start_pressure = smooth.pressure.evaluate(t_start).unwrap();
                    let end_pressure = smooth.pressure.evaluate(t_end).unwrap();

                    let (t_first_third, t_second_third) =
                        smooth.arc_length_third_times(t_start, t_end);

                    let (pressure_first_third, pressure_second_third) = (
                        smooth.pressure.evaluate(t_first_third).unwrap(),
                        smooth.pressure.evaluate(t_second_third).unwrap(),
                    );

                    // let forwards = (end_pos - start_pos).normalize();

                    // if !forwards.is_finite() {
                    //     continue;
                    //     // return None;
                    // }

                    let start_spread = pressure_to_circle_radius(start_pressure, pen_size);
                    let spread_first_third =
                        pressure_to_circle_radius(pressure_first_third, pen_size);
                    let spread_second_third =
                        pressure_to_circle_radius(pressure_second_third, pen_size);
                    let end_spread = pressure_to_circle_radius(end_pressure, pen_size);

                    let mut used_fallback_start_tangent = false;
                    let mut used_fallback_end_tangent = false;

                    let (
                        scaled_tangent_start,
                        scaled_tangent_first_third,
                        scaled_tangent_second_third,
                        scaled_tangent_end,
                    ) = (
                        {
                            let tangent = tx
                                .transform_vector(Vector2D::<_, ()>::from(
                                    smooth.velocity(t_start).unwrap(),
                                ))
                                .normalize();

                            if tangent.is_finite() {
                                tangent
                            } else {
                                used_fallback_start_tangent = true;
                                (end_pos - start_pos).normalize()
                            }
                        } * start_spread,
                        Vector2D::<_, ()>::from(smooth.velocity(t_first_third).unwrap())
                            .normalize()
                            * spread_first_third,
                        Vector2D::<_, ()>::from(smooth.velocity(t_second_third).unwrap())
                            .normalize()
                            * spread_second_third,
                        {
                            let tangent = tx
                                .transform_vector(Vector2D::<_, ()>::from(
                                    smooth.velocity(t_end).unwrap(),
                                ))
                                .normalize();

                            if tangent.is_finite() {
                                tangent
                            } else {
                                used_fallback_end_tangent = true;
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
                        Point2D::<f64, ()>::from(smooth.position(t_first_third).unwrap()),
                        Point2D::<f64, ()>::from(smooth.position(t_second_third).unwrap()),
                    );

                    let scaled_tangent_first_third =
                        tx.transform_vector(scaled_tangent_first_third);

                    let scaled_tangent_second_third =
                        tx.transform_vector(scaled_tangent_second_third);

                    let pos_first_third = tx.transform_point(pos_first_third);
                    let pos_second_third = tx.transform_point(pos_second_third);

                    let start_to_first_third = (pos_first_third - start_pos).normalize();

                    // If the computed start direction is in opposition to the direction to the
                    // first third position, use the latter for the tangent so we aren't going back
                    // on ourselves. `start_to_first_third` should be finite, but we check the
                    // partial ordering so that if it is not, we retain the current finite tangent.
                    let scaled_tangent_start = if let Some(std::cmp::Ordering::Less) =
                        scaled_tangent_start
                            .dot(start_to_first_third)
                            .partial_cmp(&0.0)
                    {
                        start_to_first_third * start_spread
                    } else {
                        scaled_tangent_start
                    };

                    // page_contents.extend([
                    //     Op::SetOutlineColor {
                    //         col: Color::Rgb(Rgb::new(1.0, 0.5, 0.0, None)),
                    //     },
                    //     Op::DrawLine {
                    //         line: Line {
                    //             points: vec![
                    //                 pdf_point_to_line_point(start_pos),
                    //                 pdf_point_to_line_point(start_pos + scaled_tangent_start),
                    //             ],
                    //             is_closed: false,
                    //         },
                    //     },
                    //     Op::SetOutlineColor {
                    //         col: Color::Rgb(Rgb::new(0.0, 0.5, 1.0, None)),
                    //     },
                    //     Op::DrawLine {
                    //         line: Line {
                    //             points: vec![
                    //                 pdf_point_to_line_point(pos_first_third),
                    //                 pdf_point_to_line_point(
                    //                     pos_first_third + scaled_tangent_first_third,
                    //                 ),
                    //             ],
                    //             is_closed: false,
                    //         },
                    //     },
                    // ]);

                    let bottom_left =
                        start_pos + Vector2D::new(-scaled_tangent_start.y, scaled_tangent_start.x);

                    let bottom_right =
                        start_pos + Vector2D::new(scaled_tangent_start.y, -scaled_tangent_start.x);

                    let lower_mid_left = pos_first_third
                        + Vector2D::new(
                            -scaled_tangent_first_third.y,
                            scaled_tangent_first_third.x,
                        );

                    let lower_mid_right = pos_first_third
                        + Vector2D::new(
                            scaled_tangent_first_third.y,
                            -scaled_tangent_first_third.x,
                        );

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

                    let top_left =
                        end_pos + Vector2D::new(-scaled_tangent_end.y, scaled_tangent_end.x);

                    let top_right =
                        end_pos + Vector2D::new(scaled_tangent_end.y, -scaled_tangent_end.x);

                    let (cp_lower_mid_left, cp_upper_mid_left) =
                        bezier_control_pts_for_intersections(
                            bottom_left,
                            top_left,
                            lower_mid_left,
                            upper_mid_left,
                        );

                    let (cp_upper_mid_right, cp_lower_mid_right) =
                        bezier_control_pts_for_intersections(
                            top_right,
                            bottom_right,
                            upper_mid_right,
                            lower_mid_right,
                        );

                    // eprintln!(
                    //     "fitting {right_first_third:?} and {right_second_third:?} into curve from {right_start:?} to {right_end:?}"
                    // );

                    // eprintln!(
                    //     "ls = {left_start:?}, le = {left_end:?}, lp1 = {left_p1:?}, lp2 = {left_p2:?}, rp1 = {right_p1:?}, rp2 = {right_p2:?}"
                    // );

                    // let Some([r2_top_c1, r2_top_c2]) = bezier_arc_control_points(r2, top, end_pos)
                    // else {
                    //     continue;
                    // };
                    // let Some([top_l1_c1, top_l1_c2]) = bezier_arc_control_points(top, l1, end_pos)
                    // else {
                    //     continue;
                    // };
                    // let Some([l2_bot_c1, l2_bot_c2]) =
                    //     bezier_arc_control_points(l2, bot, start_pos)
                    // else {
                    //     continue;
                    // };
                    // let Some([bot_r1_c1, bot_r1_c2]) =
                    //     bezier_arc_control_points(bot, r1, start_pos)
                    // else {
                    //     continue;
                    // };

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

                    let points = if let (
                        Some(top_left_to_arc_highest_cps),
                        Some(top_arc_highest_to_right_cps),
                        Some(bottom_right_to_arc_lowest_cps),
                        Some(bottom_arc_lowest_to_left_cps),
                    ) = (
                        bezier_arc_control_points(top_left, top_arc_highest, end_pos)
                            .map(IntoIterator::into_iter),
                        bezier_arc_control_points(top_arc_highest, top_right, end_pos)
                            .map(IntoIterator::into_iter),
                        bezier_arc_control_points(bottom_right, bottom_arc_lowest, start_pos)
                            .map(IntoIterator::into_iter),
                        bezier_arc_control_points(bottom_arc_lowest, bottom_left, start_pos)
                            .map(IntoIterator::into_iter),
                    ) {
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
                            // Control points for bottom right arc
                            .chain(bottom_right_to_arc_lowest_cps.map(pdf_point_to_control_point))
                            // Common point for bottom arcs
                            .chain(std::iter::once(pdf_point_to_line_point(bottom_arc_lowest)))
                            // Control points for bottom left arc
                            .chain(bottom_arc_lowest_to_left_cps.map(pdf_point_to_control_point))
                            // Bottom left point (again - this time, to complete the curve)
                            .chain(std::iter::once(pdf_point_to_line_point(bottom_left)))
                            .collect_vec()
                    } else {
                        bottom_to_top_left_points
                            .into_iter()
                            .chain(top_to_bottom_right_points)
                            .collect_vec()
                    };

                    // let points = vec![
                    //     pdf_point_into_line_point(left_start),
                    //     // pdf_point_into_line_point(left_first_third),
                    //     // pdf_point_into_line_point(left_second_third),
                    //     pdf_point_into_line_point(left_end),
                    //     pdf_point_into_line_point(right_end),
                    //     // pdf_point_into_line_point(right_second_third),
                    //     // pdf_point_into_line_point(right_first_third),
                    //     pdf_point_into_line_point(right_start),
                    // ];

                    // page_contents.extend([
                    //     Op::SetOutlineColor {
                    //         col: Color::Rgb(Rgb::new(1.0, 0.0, 0.0, None)),
                    //     },
                    //     Op::DrawLine {
                    //         line: Line {
                    //             points: vec![
                    //                 pdf_point_into_line_point(left_start),
                    //                 pdf_point_into_line_point(left_end),
                    //             ],
                    //             is_closed: false,
                    //         },
                    //     },
                    //     Op::SetOutlineColor {
                    //         col: Color::Rgb(Rgb::new(0.0, 1.0, 0.0, None)),
                    //     },
                    //     Op::DrawLine {
                    //         line: Line {
                    //             points: vec![
                    //                 pdf_point_into_line_point(left_end),
                    //                 pdf_point_into_line_point(right_end),
                    //             ],
                    //             is_closed: false,
                    //         },
                    //     },
                    //     Op::SetOutlineColor {
                    //         col: Color::Rgb(Rgb::new(0.0, 0.0, 1.0, None)),
                    //     },
                    //     Op::DrawLine {
                    //         line: Line {
                    //             points: vec![
                    //                 pdf_point_into_line_point(right_end),
                    //                 pdf_point_into_line_point(right_start),
                    //             ],
                    //             is_closed: false,
                    //         },
                    //     },
                    //     Op::SetOutlineColor {
                    //         col: Color::Rgb(Rgb::new(1.0, 0.0, 1.0, None)),
                    //     },
                    //     Op::DrawLine {
                    //         line: Line {
                    //             points: vec![
                    //                 pdf_point_into_line_point(right_start),
                    //                 pdf_point_into_line_point(left_start),
                    //             ],
                    //             is_closed: false,
                    //         },
                    //     },
                    // ]);

                    // // fixme: Lots of rejections here...
                    // let Some([r1, r2, l1, l2]) =
                    //     calc_pulley_line_points_acw(start_pos, start_spread, end_pos, end_spread)
                    // else {
                    //     continue;
                    // };

                    // let top = end_pos + forwards * end_spread;
                    // let bot = start_pos - forwards * end_spread;

                    // // fixme: ...and here.
                    // let Some([r2_top_c1, r2_top_c2]) = bezier_arc_control_points(r2, top, end_pos)
                    // else {
                    //     continue;
                    // };
                    // let Some([top_l1_c1, top_l1_c2]) = bezier_arc_control_points(top, l1, end_pos)
                    // else {
                    //     continue;
                    // };
                    // let Some([l2_bot_c1, l2_bot_c2]) =
                    //     bezier_arc_control_points(l2, bot, start_pos)
                    // else {
                    //     continue;
                    // };
                    // let Some([bot_r1_c1, bot_r1_c2]) =
                    //     bezier_arc_control_points(bot, r1, start_pos)
                    // else {
                    //     continue;
                    // };

                    // let points = vec![
                    //     pdf_point_into_line_point(r2),
                    //     pdf_point_into_control_point(r2_top_c1),
                    //     pdf_point_into_control_point(r2_top_c2),
                    //     pdf_point_into_line_point(top),
                    //     pdf_point_into_control_point(top_l1_c1),
                    //     pdf_point_into_control_point(top_l1_c2),
                    //     pdf_point_into_line_point(l1),
                    //     pdf_point_into_line_point(l2),
                    //     pdf_point_into_control_point(l2_bot_c1),
                    //     pdf_point_into_control_point(l2_bot_c2),
                    //     pdf_point_into_line_point(bot),
                    //     pdf_point_into_control_point(bot_r1_c1),
                    //     pdf_point_into_control_point(bot_r1_c2),
                    //     pdf_point_into_line_point(r1),
                    // ];

                    // let curvature_ratio = ((derivs.curvature[i_start] + derivs.curvature[i_end]
                    //     - min_curvature * 2.0)
                    //     / (curvature_span * 2.0)) as f32;

                    // assert!(curvature_ratio.is_finite());

                    let true_angle = scaled_tangent_start
                        .angle_to(scaled_tangent_end)
                        .radians
                        .abs();

                    let col = Color::Rgb(Rgb::new(
                        0.0, 0.0, 0.0,
                        // if used_fallback_start_tangent {
                        //     1.0
                        // } else {
                        //     0.0
                        // },
                        // if true_angle > 1.5 * target_angle {
                        //     0.5
                        // } else {
                        //     0.0
                        // },
                        // if true_angle < 0.5 * target_angle {
                        //     1.0
                        // } else {
                        //     0.0
                        // },
                        None,
                    ));

                    page_contents.extend([
                        Op::SetFillColor { col: col.clone() },
                        Op::SetOutlineColor { col },
                        Op::SetOutlineThickness {
                            pt: Mm(0.05).into(),
                        },
                        Op::DrawPolygon {
                            polygon: Polygon {
                                rings: vec![PolygonRing { points }],
                                mode: PaintMode::Fill,
                                winding_order: WindingOrder::NonZero,
                            },
                        },
                    ]);

                    // if let Ok(max_end_point) = smooth
                    //     .position(t_start + smooth.space_step_to_time_step(t_start, max_space_step))
                    // {
                    //     page_contents.extend([
                    //         Op::SetOutlineColor {
                    //             col: Color::Rgb(Rgb::new(1.0, 0.0, 0.0, None)),
                    //         },
                    //         Op::DrawLine {
                    //             line: Line {
                    //                 points: vec![
                    //                     pdf_point_to_line_point(start_pos),
                    //                     pdf_point_to_line_point(
                    //                         tx.transform_point(max_end_point.into()),
                    //                     ),
                    //                 ],
                    //                 is_closed: false,
                    //             },
                    //         },
                    //     ])
                    // }

                    // if let Ok(min_end_point) = smooth
                    //     .position(t_start + smooth.space_step_to_time_step(t_start, min_space_step))
                    // {
                    //     page_contents.extend([
                    //         Op::SetOutlineColor {
                    //             col: Color::Rgb(Rgb::new(0.0, 1.0, 0.0, None)),
                    //         },
                    //         Op::DrawLine {
                    //             line: Line {
                    //                 points: vec![
                    //                     pdf_point_to_line_point(start_pos),
                    //                     pdf_point_to_line_point(
                    //                         tx.transform_point(min_end_point.into()),
                    //                     ),
                    //                 ],
                    //                 is_closed: false,
                    //             },
                    //         },
                    //     ])
                    // }

                    // Some(PolygonRing { points })
                }

                // // fixme: Alpha is ignored here.
                // let [b, g, r, _a] = stroke.colour().map(|u| f32::from(u) / 255.0);

                // page_contents.extend([
                //     Op::SetFillColor {
                //         col: Color::Rgb(Rgb::new(r, g, b, None)),
                //     },
                //     Op::SetOutlineColor {
                //         col: Color::Rgb(Rgb::new(r, g, b, None)),
                //     },
                //     Op::SetOutlineThickness {
                //         pt: Mm(0.05).into(),
                //     },
                // ]);

                // let len_before = page_contents.len();

                // page_contents.extend(rings.map(|ring| Op::DrawPolygon {
                //     polygon: Polygon {
                //         rings: vec![ring],
                //         mode: PaintMode::Fill,
                //         winding_order: WindingOrder::default(),
                //     },
                // }));

                // polygon_count += page_contents.len() - len_before;
            }
        }

        pdf.pages
            .push(PdfPage::new(Mm(w as _), Mm(h as _), page_contents));
    }

    // let discarded_event_count = event_count - used_event_count;

    // eprintln!(
    //     "Discarded {discarded_event_count} of {event_count} stroke events ({:.1}%).",
    //     100. * (discarded_event_count as f64) / (event_count as f64)
    // );

    eprintln!("Creating PDF with {polygon_count} polygons");

    let mut warnings = vec![];
    let bytes = pdf.save(&PdfSaveOptions::default(), &mut warnings);

    eprintln!("Warnings: {warnings:#?}");

    std::fs::write("/tmp/doc.pdf", &bytes).unwrap();

    let mb = f64::from(std::fs::metadata("/tmp/doc.pdf").unwrap().size() as u32) / 1000000f64;

    eprintln!("{mb} MB");
}
