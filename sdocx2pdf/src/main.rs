use printpdf::{
    Color, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage, PdfSaveOptions, Polygon,
    PolygonRing, Rgb, WindingOrder,
};

struct PdfSpace;
type PdfPoint = euclid::Point2D<f64, PdfSpace>;
type PdfVector = euclid::Vector2D<f64, PdfSpace>;

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

    for page in document.pages() {
        // fixme: Document units are pixels, so we shouldn't be treating them as mm because it
        // creates huge dimensions.
        let (w, h) = page.width_height();

        let mut page_contents = vec![];

        for layer in page.layers() {
            for object in layer.objects() {
                let sdocx::DocObject::Stroke(stroke) = object else {
                    continue;
                };

                // eprintln!(
                //     "Stroke pen is {}",
                //     stroke.pen_name().unwrap_or("(unspecified)")
                // );

                event_count += stroke.events().len();

                // let mut total_pressure = 0_f32;

                let events = stroke.events();
                let pen_size = stroke.pen_size().unwrap_or(1.0);

                let event_points: Vec<PdfPoint> = stroke
                    .events()
                    .iter()
                    .map(|e| PdfPoint::new(e.point.x, f64::from(h) - e.point.y))
                    .collect();

                let mut boundary_points = vec![printpdf::LinePoint::default(); events.len() * 2];

                for i in 0..events.len() {
                    let pos_here = event_points[i];

                    // Calculate "forwards" direction by averaging the directions from the previous
                    // N points to here and the directions from here to the next M points.
                    let window_pre_first = i.saturating_sub(75);
                    let window_post_end = (i + 11).min(events.len());

                    let mut forwards = event_points[window_pre_first..i]
                        .iter()
                        .map(|&before| (pos_here - before).normalize())
                        .chain(
                            event_points[(i + 1)..window_post_end]
                                .iter()
                                .map(|&after| (after - pos_here).normalize()),
                        )
                        .filter(|v| v.is_finite())
                        .sum::<PdfVector>()
                        .normalize();

                    // If none of the directions were finite or all the finite ones were zero, the
                    // average will be zero.
                    if forwards.square_length() == 0.0 {
                        // Best guess for left-to-right writing systems.
                        forwards = PdfVector::new(1.0, 0.0);
                    }

                    // fixme: This system will never produce nice-looking text because inevitably
                    // we will create points that are contained within the polygon, which messes up
                    // the fill and leaves ugly gaps in places.

                    let left_dir = PdfVector::new(-forwards.y, forwards.x);

                    // Square-rooting the pressure prevents low pressure values making stuff
                    // stupidly small, while also keeping it in [0,1]. We halve the pen size so
                    // that the distance between the left and right points is equal to the pen
                    // size, not double.
                    let spread_left =
                        left_dir * f64::from(pen_size) * 0.5 * f64::from(events[i].pressure).sqrt();

                    let left_point = pos_here + spread_left;
                    let right_point = pos_here - spread_left;

                    boundary_points[i] = LinePoint {
                        p: printpdf::Point::new(Mm(left_point.x as f32), Mm(left_point.y as f32)),
                        bezier: false,
                    };

                    boundary_points[2 * events.len() - i - 1] = LinePoint {
                        p: printpdf::Point::new(Mm(right_point.x as f32), Mm(right_point.y as f32)),
                        bezier: false,
                    };
                }

                let [r, g, b, _a] = stroke.colour().map(|u| f32::from(u) / 255.0);

                page_contents.extend([
                    Op::SetFillColor {
                        col: Color::Rgb(Rgb::new(r, g, b, None)),
                    },
                    Op::SetOutlineColor {
                        col: Color::Rgb(Rgb::new(r, g, b, None)),
                    },
                    Op::SetOutlineThickness {
                        pt: Mm(stroke.pen_size().unwrap_or(1.0) * 0.05).into(),
                    },
                    Op::DrawPolygon {
                        polygon: Polygon {
                            rings: vec![PolygonRing {
                                points: boundary_points,
                            }],
                            mode: PaintMode::FillStroke,
                            winding_order: WindingOrder::default(),
                        },
                    },
                ]);
            }
        }

        pdf.pages
            .push(PdfPage::new(Mm(w as _), Mm(h as _), page_contents));
    }

    eprintln!("Document contains {event_count} stroke events");

    let mut warnings = vec![];
    let bytes = pdf.save(&PdfSaveOptions::default(), &mut warnings);

    eprintln!("Warnings: {warnings:#?}");

    std::fs::write("/tmp/doc.pdf", &bytes).unwrap();
}
