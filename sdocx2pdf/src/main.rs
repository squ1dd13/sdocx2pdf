use std::os::unix::fs::MetadataExt;

use euclid::{Angle, Point2D, Vector2D};
use itertools::{Itertools, Position};
use lerp::Lerp;
use printpdf::{
    Color, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage, PdfSaveOptions, Polygon,
    PolygonRing, Rgb, WindingOrder,
};

struct PdfSpace;
type PdfPoint = Point2D<f64, PdfSpace>;
type PdfVector = Vector2D<f64, PdfSpace>;

fn pdf_point_into_line_point(point: PdfPoint) -> LinePoint {
    LinePoint {
        p: printpdf::Point {
            x: Mm(point.x as f32).into(),
            y: Mm(point.y as f32).into(),
        },
        bezier: false,
    }
}

fn pdf_point_into_control_point(point: PdfPoint) -> LinePoint {
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
    let mut discarded_duplicate_event_count = 0_usize;
    let mut discarded_middle_event_count = 0_usize;

    for page in document.pages() {
        // fixme: Document units are pixels, so we shouldn't be treating them as mm because it
        // creates huge dimensions.
        let (w, h) = page.width_height();

        let mut page_contents = vec![];

        for layer in page.layers() {
            // todo: Filter for strokes only, then group by pen properties so we can create
            // an ExtendedGraphicsState for each pen and use that rather than writing out explicit
            // properties each time.
            for object in layer.objects() {
                let sdocx::DocObject::Stroke(stroke) = object else {
                    continue;
                };

                // eprintln!(
                //     "Stroke pen is {}",
                //     stroke.pen_name().unwrap_or("(unspecified)")
                // );

                event_count += stroke.events().len();

                let pen_size = stroke.pen_size().map(f64::from).unwrap_or(1.0);

                let deduped_events = stroke
                    .events()
                    .iter()
                    .map(|e| {
                        (
                            // Convert from `Document`-space, with y=0 at the top, to PDF space,
                            // with y=0 at the bottom.
                            PdfPoint::new(e.point.x, f64::from(h) - e.point.y),
                            e.pressure,
                        )
                    })
                    // .tuple_windows()
                    // .coalesce(|(a, b), (_, c)| {
                    //     let ab = b.0 - a.0;
                    //     let ac = c.0 - a.0;
                    //     let ac_sql = ac.square_length();
                    //     // If the pressure at `b` is roughly equal to what it would be if we just
                    //     // linearly interpolated between the pressures at `a` and `b` by distance,
                    //     // and if `b` is roughly on the line between `a` and `c`, then we can
                    //     // discard `b` with minimal visual effect.
                    //     if ac_sql != 0.0
                    //         && ((a.1.lerp(c.1, ab.length() as f32 / (ac_sql as f32).sqrt())) - b.1)
                    //             / b.1
                    //             <= 10.0
                    //         && ab.angle_to(ac).to_degrees().abs() <= 0.5
                    //     {
                    //         discarded_middle_event_count += 1;
                    //         Ok((a, c))
                    //     } else {
                    //         Err(((a, b), (b, c)))
                    //     }
                    // })
                    // .with_position()
                    // .flat_map(|(p, (x1, x2))| match p {
                    //     Position::First => [Some(x1), None],
                    //     Position::Middle | Position::Last => [Some(x2), None],
                    //     Position::Only => [Some(x1), Some(x2)],
                    // })
                    // .flatten()
                    // Merge consecutive events with the same position. We use the largest pressure
                    // value when merging events because when several events cover the same
                    // position, only the one with the largest pressure needs to be drawn, since it
                    // will cover the others. (At least, in the theoretical model where each event
                    // is drawn as a disc.)
                    .coalesce(|a, b| {
                        if a.0 == b.0 {
                            discarded_duplicate_event_count += 1;
                            Ok((a.0, a.1.max(b.1)))
                        } else {
                            Err((a, b))
                        }
                    });

                // todo: We should be able to avoid self-intersection by taking points while
                // the distance from the start is increasing. This would be more efficient than
                // making a polygon for every pair of points.
                let rings = deduped_events.tuple_windows().flat_map(
                    |((start_pos, start_pressure), (end_pos, end_pressure))| {
                        let forwards = (end_pos - start_pos).normalize();

                        // Should not fail, because no pair of consecutive events have a common
                        // position.
                        debug_assert!(forwards.is_finite());

                        let left = PdfVector::new(-forwards.y, forwards.x);

                        let start_spread = 0.5
                            * pen_size
                            * f64::from(start_pressure.sqrt().lerp(start_pressure, start_pressure))
                                .max(0.05);

                        let end_spread = 0.5
                            * pen_size
                            * f64::from(end_pressure.sqrt().lerp(end_pressure, end_pressure))
                                .max(0.05);

                        // fixme: Lots of rejections here...
                        let [r1, r2, l1, l2] = calc_pulley_line_points_acw(
                            start_pos,
                            start_spread,
                            end_pos,
                            end_spread,
                        )?;

                        let top = end_pos + forwards * end_spread;
                        let bot = start_pos - forwards * end_spread;

                        // fixme: ...and here.
                        let [r2_top_c1, r2_top_c2] = bezier_arc_control_points(r2, top, end_pos)?;
                        let [top_l1_c1, top_l1_c2] = bezier_arc_control_points(top, l1, end_pos)?;
                        let [l2_bot_c1, l2_bot_c2] = bezier_arc_control_points(l2, bot, start_pos)?;
                        let [bot_r1_c1, bot_r1_c2] = bezier_arc_control_points(bot, r1, start_pos)?;

                        let points = vec![
                            pdf_point_into_line_point(r2),
                            pdf_point_into_control_point(r2_top_c1),
                            pdf_point_into_control_point(r2_top_c2),
                            pdf_point_into_line_point(top),
                            pdf_point_into_control_point(top_l1_c1),
                            pdf_point_into_control_point(top_l1_c2),
                            pdf_point_into_line_point(l1),
                            pdf_point_into_line_point(l2),
                            pdf_point_into_control_point(l2_bot_c1),
                            pdf_point_into_control_point(l2_bot_c2),
                            pdf_point_into_line_point(bot),
                            pdf_point_into_control_point(bot_r1_c1),
                            pdf_point_into_control_point(bot_r1_c2),
                            pdf_point_into_line_point(r1),
                        ];

                        // Approximates an illustration of a belt drive, where one pulley is
                        // centred on the first point and has radius based on the first pressure,
                        // and the other is centred on the second point and has radius based on the
                        // second pressure.
                        Some(PolygonRing {
                            points,
                            // points: std::iter::once(start_pos - forwards * start_spread)
                            //     .chain(pts[..2].iter().copied())
                            //     .chain(std::iter::once(end_pos + forwards * end_spread))
                            //     .chain(pts[2..].iter().copied())
                            //     .map(|p| {
                            //         if !p.is_finite() {
                            //             Default::default()
                            //         } else {
                            //             p
                            //         }
                            //     })
                            //     .map(pdf_point_into_line_point)
                            //     .collect(),
                            // points: vec![
                            //     pdf_point_into_line_point(start_pos - left * start_spread),
                            //     pdf_point_into_line_point(
                            //         start_pos - left.lerp(forwards, 0.5).normalize() * start_spread,
                            //     ),
                            //     pdf_point_into_line_point(start_pos - forwards * start_spread),
                            //     pdf_point_into_line_point(
                            //         start_pos
                            //             + (-forwards).lerp(left, 0.5).normalize() * start_spread,
                            //     ),
                            //     pdf_point_into_line_point(start_pos + left * start_spread),
                            //     pdf_point_into_line_point(end_pos + left * end_spread),
                            //     pdf_point_into_line_point(
                            //         end_pos + left.lerp(forwards, 0.5).normalize() * end_spread,
                            //     ),
                            //     pdf_point_into_line_point(end_pos + forwards * end_spread),
                            //     pdf_point_into_line_point(
                            //         end_pos + forwards.lerp(-left, 0.5).normalize() * end_spread,
                            //     ),
                            //     pdf_point_into_line_point(end_pos - left * end_spread),
                            // ],
                        })
                    },
                );

                // fixme: Alpha is ignored here.
                let [b, g, r, _a] = stroke.colour().map(|u| f32::from(u) / 255.0);

                page_contents.extend([
                    // Op::SetFillColor {
                    //     col: Color::Rgb(Rgb::new(r, g, b, None)),
                    // },
                    Op::SetOutlineColor {
                        col: Color::Rgb(Rgb::new(r, g, b, None)),
                    },
                    Op::SetOutlineThickness {
                        pt: Mm(0.05).into(),
                    },
                ]);

                let len_before = page_contents.len();

                page_contents.extend(rings.map(|ring| Op::DrawPolygon {
                    polygon: Polygon {
                        rings: vec![ring],
                        mode: PaintMode::Stroke,
                        winding_order: WindingOrder::default(),
                    },
                }));

                polygon_count += page_contents.len() - len_before;
            }
        }

        pdf.pages
            .push(PdfPage::new(Mm(w as _), Mm(h as _), page_contents));
    }

    let discarded_event_count = discarded_duplicate_event_count + discarded_middle_event_count;

    eprintln!(
        "Discarded {discarded_event_count} ({} + {}) of {event_count} stroke events ({:.1}%).",
        discarded_duplicate_event_count,
        discarded_middle_event_count,
        100. * (discarded_event_count as f64) / (event_count as f64)
    );

    eprintln!("Creating PDF with {polygon_count} polygons");

    let mut warnings = vec![];
    let bytes = pdf.save(&PdfSaveOptions::default(), &mut warnings);

    eprintln!("Warnings: {warnings:#?}");

    std::fs::write("/tmp/doc.pdf", &bytes).unwrap();

    let mb = f64::from(std::fs::metadata("/tmp/doc.pdf").unwrap().size() as u32) / 1000000f64;

    eprintln!("{mb} MB");
}
