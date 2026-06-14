use std::{ops::Div, os::unix::fs::MetadataExt};

use euclid::{Angle, Point2D, Vector2D, Vector3D};
use itertools::{Either, Itertools, Position};
use lerp::Lerp;
use printpdf::{
    Color, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage, PdfSaveOptions, Polygon,
    PolygonRing, Rgb, WindingOrder,
};
use sdocx::page::object::stroke::{Event, Stroke};

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

struct DocumentSpace;
type DocPoint = Point2D<f64, DocumentSpace>;
type DocVec = Vector2D<f64, DocumentSpace>;

#[derive(Debug, Clone, Copy)]
enum StrokeFnPoint {
    First {
        pos: DocPoint,
        pressure: f32,
        time: u32,
    },

    Last {
        pos: DocPoint,
        pressure: f32,
        time: u32,
        path_dist: f64,
    },

    Middle {
        pos: DocPoint,
        pressure: f32,
        time: u32,
        path_dist: f64,

        /// Second derivative of position with respect to time.
        accn: DocVec,

        /// Second derivative of pressure with respect to path distance.
        pressure_2nd: f64,
    },
}

impl StrokeFnPoint {
    fn position(&self) -> DocPoint {
        match self {
            StrokeFnPoint::First { pos, .. } => *pos,
            StrokeFnPoint::Last { pos, .. } => *pos,
            StrokeFnPoint::Middle { pos, .. } => *pos,
        }
    }

    fn path_dist(&self) -> f64 {
        match self {
            StrokeFnPoint::First { .. } => 0.0,
            StrokeFnPoint::Last {
                path_dist: dist, ..
            } => *dist,
            StrokeFnPoint::Middle {
                path_dist: dist, ..
            } => *dist,
        }
    }

    fn time(&self) -> u32 {
        match self {
            StrokeFnPoint::First { time, .. } => *time,
            StrokeFnPoint::Last { time, .. } => *time,
            StrokeFnPoint::Middle { time, .. } => *time,
        }
    }

    fn pressure(&self) -> f32 {
        match self {
            StrokeFnPoint::First { pressure, .. } => *pressure,
            StrokeFnPoint::Last { pressure, .. } => *pressure,
            StrokeFnPoint::Middle { pressure, .. } => *pressure,
        }
    }

    fn acceleration(&self) -> Option<&DocVec> {
        match self {
            StrokeFnPoint::Middle { accn, .. } => Some(accn),
            _ => None,
        }
    }

    fn pressure_2nd(&self) -> Option<f64> {
        match self {
            StrokeFnPoint::Middle { pressure_2nd, .. } => Some(*pressure_2nd),
            _ => None,
        }
    }

    fn points_from(stroke: &Stroke) -> Vec<StrokeFnPoint> {
        let events = stroke.events();

        if events.is_empty() {
            return Vec::new();
        }

        let mut last_pos: DocPoint = (events[0].point.x, events[0].point.y).into();
        let mut path_dist = 0.0;
        let mut last_time = None;

        let mut fn_pts = stroke
            .events()
            .iter()
            .with_position()
            .flat_map(|(it_pos, event)| {
                let pos: DocPoint = (event.point.x, event.point.y).into();
                let pressure = event.pressure;
                let time = event.timestamp;

                if last_time == Some(time) {
                    // Two events at once. Often, the second event is an exact duplicate of the
                    // first.
                    // todo: This could cause issues if the second event is different.
                    return None;
                }

                last_time = Some(time);

                let dist_to_last = pos.distance_to(last_pos);

                // Ignore this event if it is too close in position to the previous one.
                // todo: Pick a sensible epsilon here.
                if dist_to_last < 0.01 {
                    return None;
                }

                path_dist += dist_to_last;
                last_pos = pos;

                Some(match it_pos {
                    Position::First | Position::Only => StrokeFnPoint::First {
                        pos,
                        pressure,
                        time,
                    },

                    Position::Middle => StrokeFnPoint::Middle {
                        pos,
                        pressure,
                        time,
                        path_dist,
                        accn: DocVec::zero(),
                        pressure_2nd: 0.0,
                    },

                    Position::Last => StrokeFnPoint::Last {
                        pos,
                        pressure,
                        time,
                        path_dist,
                    },
                })
            })
            .collect_vec();

        for i in 1..(fn_pts.len() - 1) {
            let time_bwd = fn_pts[i].time() - fn_pts[i - 1].time();
            let time_fwd = fn_pts[i + 1].time() - fn_pts[i].time();

            let pos_weight_bwd = 2.0 / f64::from(time_bwd * (time_bwd + time_fwd));
            let pos_weight_cur = -2.0 / f64::from(time_bwd * time_fwd);
            let pos_weight_fwd = 2.0 / f64::from(time_fwd * (time_bwd + time_fwd));

            // Central finite difference approximation to the second time derivative of position.
            // For non-equal forward/backward time deltas, this is only first-order accurate.
            // If the two are equal, this is second order.
            let accn_approx = fn_pts[i - 1].position().to_vector() * pos_weight_bwd
                + fn_pts[i].position().to_vector() * pos_weight_cur
                + fn_pts[i + 1].position().to_vector() * pos_weight_fwd;

            let dist_bwd = fn_pts[i].path_dist() - fn_pts[i - 1].path_dist();
            let dist_fwd = fn_pts[i + 1].path_dist() - fn_pts[i].path_dist();

            let pres_weight_bwd = 2.0 / (dist_bwd * (dist_bwd + dist_fwd));
            let pres_weight_cur = -2.0 / (dist_bwd * dist_fwd);
            let pres_weight_fwd = 2.0 / (dist_fwd * (dist_bwd + dist_fwd));

            // Same scheme, but now for the second path distance derivative of pressure.
            let pres_2nd_approx = f64::from(fn_pts[i - 1].pressure()) * pres_weight_bwd
                + f64::from(fn_pts[i].pressure()) * pres_weight_cur
                + f64::from(fn_pts[i + 1].pressure()) * pres_weight_fwd;

            let StrokeFnPoint::Middle {
                accn, pressure_2nd, ..
            } = &mut fn_pts[i]
            else {
                unreachable!()
            };

            assert!(
                accn_approx.is_finite() && pres_2nd_approx.is_finite(),
                "time bwd = {time_bwd}; time fwd = {time_fwd}; acceleration = {accn_approx:?}; pressure 2nd = {pres_2nd_approx}"
            );

            *accn = accn_approx;
            *pressure_2nd = pres_2nd_approx;
        }

        fn_pts
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

fn clean_events(events: &mut Vec<(PdfPoint, f32)>) {
    while events.len() >= 3 {
        let mut any_removed = false;

        let mut i = 1;

        while i + 1 < events.len() {
            let (last_pt, last_pres) = events[i - 1];
            let (this_pt, this_pres) = events[i];
            let (next_pt, next_pres) = events[i + 1];

            let this_i = i;
            i += 2;

            let to_here = this_pt.to_vector() - last_pt.to_vector();
            let from_here = next_pt.to_vector() - this_pt.to_vector();

            let abs_angle = Angle::radians(to_here.angle_to(from_here).get().abs());

            if abs_angle > Angle::frac_pi_4() / 2.5 {
                continue;
            }

            let length_ratio = to_here.length() / (to_here.length() + from_here.length());
            let pres_guess = last_pres.lerp(next_pres, length_ratio as f32);

            if (pres_guess - this_pres).abs() / this_pres > 0.1 {
                // Actual pressure is not close to what we might guess.
                continue;
            }

            any_removed = true;
            events.remove(this_i);
            i -= 1;
        }

        if !any_removed {
            break;
        }
    }
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
            // todo: Filter for strokes only, then group by pen properties so we can create
            // an ExtendedGraphicsState for each pen and use that rather than writing out explicit
            // properties each time.
            for object in layer.objects() {
                let sdocx::DocObject::Stroke(stroke) = object else {
                    continue;
                };

                let fn_pts = StrokeFnPoint::points_from(stroke);

                // eprintln!(
                //     "Stroke pen is {}",
                //     stroke.pen_name().unwrap_or("(unspecified)")
                // );

                event_count += stroke.events().len();

                let pen_size = stroke.pen_size().map(f64::from).unwrap_or(1.0);

                // Convert from document space, with y=0 at the top, to PDF space, with y=0 at the
                // bottom.
                let tx = euclid::Transform2D::<f64, DocumentSpace, PdfSpace>::scale(1.0, -1.0)
                    .then_translate(PdfVector::new(0.0, h.into()));

                let (max_accel_mag, max_pres_2nd_mag) =
                    fn_pts.iter().fold((None, None), |(ma, mp), pt| match pt {
                        StrokeFnPoint::Middle {
                            accn, pressure_2nd, ..
                        } => {
                            let accn_mag = accn.length();
                            let pres_2nd_mag = pressure_2nd.abs();

                            (
                                match ma {
                                    None => Some(accn_mag),
                                    Some(max_accn) => Some(max_accn.max(accn_mag)),
                                },
                                match mp {
                                    None => Some(pres_2nd_mag),
                                    Some(max_pres_2nd) => Some(max_pres_2nd.max(pres_2nd_mag)),
                                },
                            )
                        }

                        _ => (ma, mp),
                    });

                let rings = fn_pts
                    .into_iter()
                    .filter(|pt| {
                        pt.acceleration().is_none_or(|accn| {
                            max_accel_mag.is_none_or(|mam| accn.length() < 0.025 * mam)
                        }) || pt.pressure_2nd().is_none_or(|pres_2nd| {
                            max_pres_2nd_mag.is_none_or(|mp2m| pres_2nd.abs() < 0.001 * mp2m)
                        })
                    })
                    .tuple_windows()
                    .flat_map(|(start, end)| {
                        // A single event, ish
                        used_event_count += 1;

                        let start_pos = tx.transform_point(start.position());
                        let end_pos = tx.transform_point(end.position());
                        let start_pressure = start.pressure();
                        let end_pressure = end.pressure();

                        let forwards = (end_pos - start_pos).normalize();

                        if !forwards.is_finite() {
                            return None;
                        }

                        let start_spread =
                            0.5 * pen_size * f64::from(start_pressure.powf(0.7)).clamp(0.05, 0.3);
                        let end_spread =
                            0.5 * pen_size * f64::from(end_pressure.powf(0.7)).clamp(0.05, 0.3);

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

                        Some(PolygonRing { points })
                    });

                // fixme: Alpha is ignored here.
                let [b, g, r, _a] = stroke.colour().map(|u| f32::from(u) / 255.0);

                page_contents.extend([
                    Op::SetFillColor {
                        col: Color::Rgb(Rgb::new(r, g, b, None)),
                    },
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
                        mode: PaintMode::Fill,
                        winding_order: WindingOrder::default(),
                    },
                }));

                polygon_count += page_contents.len() - len_before;
            }
        }

        pdf.pages
            .push(PdfPage::new(Mm(w as _), Mm(h as _), page_contents));
    }

    let discarded_event_count = event_count - used_event_count;

    eprintln!(
        "Discarded {discarded_event_count} of {event_count} stroke events ({:.1}%).",
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
