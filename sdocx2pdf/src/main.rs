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
        // "/home/alex/projects/re/sdocx/sample_docs/Different pen types_260623_171841.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Landscape_260624_145202.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Landscape_260624_145950_has_empty_page.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Landscape_260624_150246_with_explicit_blank_page_last.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Paged with handwriting_260624_142215.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/long page with rotated squiggle_260624_155155.sdocx",
    )
    .unwrap();

    let name = document
        .title_text()
        .raw_string()
        .unwrap_or("Unnamed document");

    eprintln!("Name is '{name}'");

    let mut pdf = PdfDocument::new(name);

    let pageless = match document.page_model() {
        sdocx::PageModel::Paged => {
            eprintln!("This is a paged document");
            false
        }
        sdocx::PageModel::Pageless => {
            eprintln!("This is a pageless document");
            true
        }
    };

    for (pos, page) in document.pages().iter().with_position() {
        // For paged documents, there is a ghost page in the sdocx that is not represented in the
        // raster PDF. We ignore it too.
        if !pageless && matches!(pos, itertools::Position::Last) && page.is_empty() {
            continue;
        }

        let (page_w_internal, page_h_internal) = page.width_height();
        let page_w_internal = page_w_internal.to_f32().unwrap();
        let page_h_internal = page_h_internal.to_f32().unwrap();

        // Use A4 width for the smaller dimension of the page. When the paged A4 mode is used in
        // the app, this results in A4-sized pages for both portrait and lanscape. For pageless
        // documents and for the app's "long portrait" option, the width is that of A4, with the
        // height scaled accordingly.
        let mm_per_unit = 210.0 / page_w_internal.min(page_h_internal);

        let page_w_mm = page_w_internal * mm_per_unit;
        let page_h_mm = page_h_internal * mm_per_unit;

        let mut page_contents = {
            let tx = printpdf::Op::SetTransformationMatrix {
                matrix: printpdf::CurTransMat::Raw(
                    // Document space has y = 0 at the top; PDF space has it at the bottom. Rather
                    // than converting coordinates everywhere, we just flip everything on the
                    // horizontal axis using a negative y scale followed by a translation. While
                    // doing that, we also scale the document contents to fit our chosen page
                    // dimensions.
                    printpdf::CurTransMat::combine_matrix(
                        printpdf::CurTransMat::Scale(mm_per_unit, -mm_per_unit).as_array(),
                        printpdf::CurTransMat::Translate(
                            printpdf::Pt(0.0),
                            Mm(page_h_mm).into_pt(),
                        )
                        .as_array(),
                    ),
                ),
            };

            vec![tx]
        };

        // Map for keeping track of the graphics states used by different tools. This lets us reuse
        // the graphics state created previously by an identical tool rather than adding a new
        // graphics state to the document every time the tool changes.
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

            for (tool, strokes) in &strokes_by_tool {
                // Get the extended graphics state required by this tool, creating it if it does
                // not yet exist.
                let egs_id = tool_graphics_state_ids
                    .entry(tool.clone())
                    .or_insert_with(|| pdf.add_graphics_state(tool.create_egs()));

                tool.draw_events(
                    egs_id,
                    strokes
                        .inspect(|_| strokes_handled += 1)
                        .map(|s| s.events()),
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

        pdf.pages
            .push(PdfPage::new(Mm(page_w_mm), Mm(page_h_mm), page_contents));
    }

    let mut warnings = vec![];
    let bytes = pdf.save(&PdfSaveOptions::default(), &mut warnings);

    eprintln!("Warnings: {warnings:#?}");

    std::fs::write("/tmp/doc.pdf", &bytes).unwrap();

    let mb = f64::from(std::fs::metadata("/tmp/doc.pdf").unwrap().size() as u32) / 1000000f64;

    eprintln!("{mb} MB");
}
