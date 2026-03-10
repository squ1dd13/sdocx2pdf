use itertools::{Itertools, Position};
use lerp::Lerp;
use printpdf::{
    Color, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage, PdfSaveOptions, Polygon,
    PolygonRing, Rgb, WindingOrder,
};

struct PdfSpace;
type PdfPoint = euclid::Point2D<f64, PdfSpace>;
type PdfVector = euclid::Vector2D<f64, PdfSpace>;

fn pdf_point_into_line_point(point: PdfPoint) -> LinePoint {
    LinePoint {
        p: printpdf::Point {
            x: Mm(point.x as f32).into(),
            y: Mm(point.y as f32).into(),
        },
        bezier: false,
    }
}

fn main() {
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
    let mut discarded_monotone_event_count = 0_usize;

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
                    .tuple_windows()
                    .coalesce(|(a, b), (_, c)| {
                        let ab = b.0 - a.0;
                        let ac = c.0 - a.0;

                        let ac_sql = ac.square_length();

                        // If the pressure at `b` is roughly equal to what it would be if we just
                        // linearly interpolated between the pressures at `a` and `b` by distance,
                        // and if `b` is roughly on the line between `a` and `c`, then we can
                        // discard `b` with minimal visual effect.
                        if ac_sql != 0.0
                            && ((a.1.lerp(c.1, ab.length() as f32 / (ac_sql as f32).sqrt())) - b.1)
                                / b.1
                                <= 0.0001
                            && ab.angle_to(ac).to_degrees().abs() <= 0.5
                        {
                            discarded_monotone_event_count += 1;
                            Ok((a, c))
                        } else {
                            Err(((a, b), (b, c)))
                        }
                    })
                    .with_position()
                    .flat_map(|(p, (x1, x2))| match p {
                        Position::First => [Some(x1), None],
                        Position::Middle | Position::Last => [Some(x2), None],
                        Position::Only => [Some(x1), Some(x2)],
                    })
                    .flatten()
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
                let rings = deduped_events.tuple_windows().map(
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

                        // Approximates an illustration of a belt drive, where one pulley is
                        // centred on the first point and has radius based on the first pressure,
                        // and the other is centred on the second point and has radius based on the
                        // second pressure.
                        PolygonRing {
                            points: vec![
                                pdf_point_into_line_point(start_pos - left * start_spread),
                                pdf_point_into_line_point(
                                    start_pos - left.lerp(forwards, 0.5).normalize() * start_spread,
                                ),
                                pdf_point_into_line_point(start_pos - forwards * start_spread),
                                pdf_point_into_line_point(
                                    start_pos
                                        + (-forwards).lerp(left, 0.5).normalize() * start_spread,
                                ),
                                pdf_point_into_line_point(start_pos + left * start_spread),
                                pdf_point_into_line_point(end_pos + left * end_spread),
                                pdf_point_into_line_point(
                                    end_pos + left.lerp(forwards, 0.5).normalize() * end_spread,
                                ),
                                pdf_point_into_line_point(end_pos + forwards * end_spread),
                                pdf_point_into_line_point(
                                    end_pos + forwards.lerp(-left, 0.5).normalize() * end_spread,
                                ),
                                pdf_point_into_line_point(end_pos - left * end_spread),
                            ],
                        }
                    },
                );

                // fixme: Alpha is ignored here.
                let [b, g, r, _a] = stroke.colour().map(|u| f32::from(u) / 255.0);

                page_contents.extend([
                    Op::SetFillColor {
                        col: Color::Rgb(Rgb::new(r, g, b, None)),
                    },
                    // Op::SetOutlineColor {
                    //     col: Color::Rgb(Rgb::new(r, g, b, None)),
                    // },
                    // Op::SetOutlineThickness {
                    //     pt: Mm(0.01).into(),
                    // },
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

    let discarded_event_count = discarded_duplicate_event_count + discarded_monotone_event_count;

    eprintln!(
        "Discarded {discarded_event_count} ({} + {}) of {event_count} stroke events ({:.1}%).",
        discarded_duplicate_event_count,
        discarded_monotone_event_count,
        100. * (discarded_event_count as f64) / (event_count as f64)
    );

    eprintln!("Creating PDF with {polygon_count} polygons");

    let mut warnings = vec![];
    let bytes = pdf.save(&PdfSaveOptions::default(), &mut warnings);

    eprintln!("Warnings: {warnings:#?}");

    std::fs::write("/tmp/doc.pdf", &bytes).unwrap();
}
