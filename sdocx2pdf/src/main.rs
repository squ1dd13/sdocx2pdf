use std::{collections::HashMap, os::unix::fs::MetadataExt};

use itertools::Itertools;
use num::ToPrimitive;
use printpdf::{Mm, PdfDocument, PdfPage, PdfSaveOptions};

use crate::tool::Tool;

mod stroke;
mod tool;

fn main() {
    // sdocx::test_all();

    let document = sdocx::Document::from_zip(
        // "/home/alex/projects/re/sdocx/sample_docs/TSI exam_260507_125853.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/FM C1_260525_134723.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Different pen types_260623_171841.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Paged with handwriting_260624_142215.sdocx",
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

    for page in document.pages() {
        // fixme: Document units are pixels, so we shouldn't be treating them as mm because it
        // creates huge dimensions.
        let (page_w, page_h) = page.width_height();

        let page_h_f = page_h.to_f64().unwrap();

        let mut page_contents = vec![];
        let mut tool_graphics_state_ids = HashMap::new();

        for layer in page.layers() {
            let objects = layer.objects();
            let obj_count = objects.len() as f64;
            let mut strokes_handled = 0;

            let strokes = objects.iter().filter_map(|obj| match obj {
                sdocx::DocObject::Stroke(stroke) => Some(stroke),
                _ => None,
            });

            let strokes_by_tool = strokes.chunk_by(|stroke| Tool::for_stroke(stroke));

            // Document space has y = 0 at the top; PDF space has y = 0 at the bottom.
            let event_position_map = |(x, y)| (x, page_h_f - y);

            for (tool, strokes) in &strokes_by_tool {
                // Get the extended graphics state required by this tool, creating it if it does
                // not yet exist.
                let egs_id = tool_graphics_state_ids
                    .entry(tool.clone())
                    .or_insert_with(|| pdf.add_graphics_state(tool.create_egs()));

                tool.draw_events(
                    egs_id,
                    strokes.inspect(|_| strokes_handled += 1).map(|s| {
                        s.events()
                            .iter()
                            .map(|e| e.map_position(event_position_map))
                    }),
                    &mut page_contents,
                )
                .unwrap();

                // Number of strokes handled over the number of objects in total isn't an ideal
                // progress indicator, but it will do for now.
                eprintln!(
                    "Processing strokes: {:.1}% complete ({} strokes of {} objects)",
                    (strokes_handled as f64 / obj_count) * 100.0,
                    strokes_handled,
                    obj_count
                );
            }
        }

        pdf.pages.push(PdfPage::new(
            Mm(page_w as _),
            Mm(page_h as _),
            page_contents,
        ));
    }

    let mut warnings = vec![];
    let bytes = pdf.save(&PdfSaveOptions::default(), &mut warnings);

    eprintln!("Warnings: {warnings:#?}");

    std::fs::write("/tmp/doc.pdf", &bytes).unwrap();

    let mb = f64::from(std::fs::metadata("/tmp/doc.pdf").unwrap().size() as u32) / 1000000f64;

    eprintln!("{mb} MB");
}
