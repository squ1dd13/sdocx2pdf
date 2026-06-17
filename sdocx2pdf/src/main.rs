use std::os::unix::fs::MetadataExt;

use euclid::{Point2D, Vector2D};
use itertools::Itertools;
use printpdf::{
    Color, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage, PdfSaveOptions, Point, Polygon,
    PolygonRing, Rgb, WindingOrder,
};

use crate::stroke::{FilteredStroke, InterpolatedStroke, StrokeOrDot};

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
        "/home/alex/projects/re/sdocx/sample_docs/TSI exam_260507_125853.sdocx",
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

                let pen_size = stroke.pen_size().map(f64::from).unwrap_or(1.0);

                // Convert from document space, with y=0 at the top, to PDF space, with y=0 at the
                // bottom.
                let tx = euclid::Transform2D::<f64, (), PdfSpace>::scale(1.0, -1.0)
                    .then_translate(PdfVector::new(0.0, h.into()));

                let smooth = match StrokeOrDot::from_events(stroke.events()) {
                    StrokeOrDot::Stroke(stroke) => FilteredStroke::new(
                        &InterpolatedStroke::from_split_stroke(&stroke),
                        5.5,
                        7.9,
                        stroke.event_count() * 2,
                    )
                    .unwrap(),

                    StrokeOrDot::Dot { x, y, pressure } => {
                        let pos = tx.transform_point((x, y).into());
                        let spread = pressure_to_circle_radius(pressure, pen_size);

                        // Draw a filled circle.
                        page_contents.extend([
                            Op::SetOutlineColor {
                                col: Color::Rgb(Rgb::new(0.0, 1.0, 0.0, None)),
                            },
                            Op::SetLineCapStyle {
                                cap: printpdf::LineCapStyle::Round,
                            },
                            Op::SetOutlineThickness {
                                pt: Mm(spread as f32 * 2.0).into(),
                            },
                            Op::DrawLine {
                                line: printpdf::Line {
                                    points: vec![
                                        pdf_point_to_line_point(pos),
                                        pdf_point_to_line_point(pos),
                                    ],
                                    is_closed: false,
                                },
                            },
                        ]);

                        continue;
                    }
                };

                let target_angle = f64::to_radians(15.0);
                let min_space_step = 2.0;
                let max_time_step = 50.0;

                let sample_times =
                    smooth.compute_sample_times(target_angle, min_space_step, max_time_step);

                // let rings = (0..derivs.t.len())
                for (t_start, t_end) in sample_times.tuple_windows() {
                    let start_pos = tx.transform_point(smooth.position(t_start).unwrap().into());
                    let end_pos = tx.transform_point(smooth.position(t_end).unwrap().into());

                    let start_pressure = smooth.pressure.evaluate(t_start).unwrap();
                    let end_pressure = smooth.pressure.evaluate(t_end).unwrap();

                    let (t_first_third, t_second_third) =
                        smooth.arc_length_third_times(t_start, t_end);

                    let (pressure_first_third, pressure_second_third) = (
                        smooth.pressure.evaluate(t_first_third).unwrap(),
                        smooth.pressure.evaluate(t_second_third).unwrap(),
                    );

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

                    let use_arcs = true;

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
                            bezier_arc_control_points(top_left, top_arc_highest, end_pos).and_then(
                                |a| {
                                    bezier_arc_control_points(top_arc_highest, top_right, end_pos)
                                        .and_then(|b| {
                                            bezier_arc_control_points(
                                                bottom_right,
                                                bottom_arc_lowest,
                                                start_pos,
                                            )
                                            .and_then(
                                                |c| {
                                                    bezier_arc_control_points(
                                                        bottom_arc_lowest,
                                                        bottom_left,
                                                        start_pos,
                                                    )
                                                    .map(|d| (a, b, c, d))
                                                },
                                            )
                                        })
                                },
                            )
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

                    // let true_angle = scaled_tangent_start
                    //     .angle_to(scaled_tangent_end)
                    //     .radians
                    //     .abs();

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
                                mode: PaintMode::Stroke,
                                winding_order: WindingOrder::NonZero,
                            },
                        },
                        // Set things up for drawing the points of interest.
                        Op::SetLineCapStyle {
                            cap: printpdf::LineCapStyle::Round,
                        },
                    ]);

                    page_contents.extend(smooth.key_times().flat_map(|key_time| {
                        let t = key_time.to_time();

                        let pos = tx.transform_point(smooth.position(t).unwrap().into());

                        [
                            Op::SetOutlineColor {
                                col: Color::Rgb(match key_time {
                                    stroke::KeyTime::Start(_) => Rgb::new(1.0, 0.5, 0.0, None),
                                    stroke::KeyTime::CurvatureExtremum(_) => {
                                        Rgb::new(0.0, 1.0, 0.0, None)
                                    }
                                    stroke::KeyTime::PressureExtremum(_) => {
                                        Rgb::new(1.0, 0.0, 0.0, None)
                                    }
                                    stroke::KeyTime::End(_) => Rgb::new(0.0, 0.5, 1.0, None),
                                }),
                            },
                            Op::SetOutlineThickness {
                                pt: Mm(pressure_to_circle_radius(
                                    smooth.pressure.evaluate(t).unwrap(),
                                    pen_size,
                                ) as f32
                                    * 2.0
                                    * 0.25)
                                .into(),
                            },
                            Op::DrawLine {
                                line: printpdf::Line {
                                    points: vec![
                                        pdf_point_to_line_point(pos),
                                        pdf_point_to_line_point(pos),
                                    ],
                                    is_closed: false,
                                },
                            },
                        ]
                    }));
                }
            }
        }

        pdf.pages
            .push(PdfPage::new(Mm(w as _), Mm(h as _), page_contents));
    }

    let mut warnings = vec![];
    let bytes = pdf.save(&PdfSaveOptions::default(), &mut warnings);

    eprintln!("Warnings: {warnings:#?}");

    std::fs::write("/tmp/doc.pdf", &bytes).unwrap();

    let mb = f64::from(std::fs::metadata("/tmp/doc.pdf").unwrap().size() as u32) / 1000000f64;

    eprintln!("{mb} MB");
}
