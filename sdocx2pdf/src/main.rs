use printpdf::{Color, Line, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, Rgb};

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

                let mut total_pressure = 0_f32;

                let pdf_points = stroke
                    .events()
                    .iter()
                    .map(|event| {
                        total_pressure += event.pressure;

                        printpdf::LinePoint {
                            p: printpdf::Point::new(
                                Mm(event.point.x as _),
                                // PDF has y=0 at the bottom, but `Document` has it at the top.
                                Mm((f64::from(h) - event.point.y) as _),
                            ),
                            bezier: false,
                        }
                    })
                    .collect();

                let mean_pressure = total_pressure / (stroke.events().len() as f32);

                let [r, g, b, _a] = stroke.colour().map(|u| f32::from(u) / 255.0);

                page_contents.extend([
                    Op::SetOutlineColor {
                        col: Color::Rgb(Rgb::new(r, g, b, None)),
                    },
                    Op::SetOutlineThickness {
                        pt: Mm(stroke.pen_size().unwrap_or(1.0) * mean_pressure).into(),
                    },
                    Op::SetLineCapStyle {
                        cap: printpdf::LineCapStyle::Round,
                    },
                    Op::DrawLine {
                        line: Line {
                            points: pdf_points,
                            is_closed: false,
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
