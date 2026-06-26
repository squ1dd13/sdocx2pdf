use std::{collections::HashMap, os::unix::fs::MetadataExt, time::Duration};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use lopdf::dictionary;
use num::ToPrimitive;

use crate::tool::Tool;

mod op_gen;
mod stroke;
mod tool;

fn main() {
    let (document, mut _media_storage) = sdocx::Document::from_zip(
        // "/home/alex/projects/re/sdocx/sample_docs/TSI exam_260507_125853.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/FM C1_260525_134723.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Different pen types_260623_171841.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Nearly empty but long inf scroll_260624_170445.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Landscape_260624_145202.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Landscape_260624_145950_has_empty_page.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Landscape_260624_150246_with_explicit_blank_page_last.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Much handwriting on pages_260625_184756.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/Paged with handwriting_260624_142215.sdocx",
        // "/home/alex/projects/re/sdocx/sample_docs/long page with rotated squiggle_260624_155155.sdocx",
    )
    .unwrap();

    let document_name = document.title_text().raw_string().unwrap_or("Invalid name");

    eprintln!("Name is '{document_name}'");

    let mut lpdf = lopdf::Document::with_version("1.5");

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

    let multi_progress = MultiProgress::new();

    // Only show a progress bar for the pages if there is more than one.
    let pages_bar = if let page_count @ 2.. = document.pages().len() as u64 {
        Some(
            multi_progress.add(ProgressBar::new(page_count)).with_style(
                ProgressStyle::with_template("Processing pages   [{bar:40}] [{pos}/{len}]")
                    .unwrap()
                    .progress_chars("# "),
            ),
        )
    } else {
        None
    };

    // (Used `printpdf::serialize::to_lopdf_doc` as a reference)
    let pages_id = lpdf.new_object_id();

    let catalog = lopdf::dictionary! {
        "Type" => "Catalog",
        "PageLayout" => "OneColumn",
        "PageMode" => "UseNone",
        "Pages" => pages_id,
    };

    let mut page_ids = Vec::with_capacity(document.pages().len());

    const A4_PORTRAIT_WIDTH_PT: f32 = 210.0 * 2.84526;
    const A4_PTRT_HEIGHT_PT: f32 = 297.0 * 2.84526;

    for (pos, page) in document.pages().iter().with_position() {
        pages_bar.as_ref().inspect(|pb| pb.inc(1));

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
        let pt_per_unit = A4_PORTRAIT_WIDTH_PT / page_w_internal.min(page_h_internal);

        let page_w_pt = page_w_internal * pt_per_unit;

        let page_h_pt = {
            if pageless && let Some(drawn_rect) = page.drawn_rect() {
                // Since pageless documents are A4-width, the height of a page in an equivalent
                // paged document is A4 height, 297 mm. The sdocx tends to report an extra
                // page-height worth of empty space at the end of a pageless document. When the app
                // exports a PDF, this space is not included, and we don't want to include it
                // either, so we subtract it from the height. Just to be safe, we make sure not to
                // reduce the height below the combined height of the pages we'd need to hold the
                // drawn content if this were a paged document.

                let drawn_height_pt = (drawn_rect.bottom - drawn_rect.top) as f32 * pt_per_unit;

                let drawn_page_count = (drawn_height_pt / A4_PTRT_HEIGHT_PT).ceil();
                let reduced_page_count = (page_h_internal * pt_per_unit) / A4_PTRT_HEIGHT_PT - 1.0;

                reduced_page_count.max(drawn_page_count) * A4_PTRT_HEIGHT_PT
            } else {
                page_h_internal * pt_per_unit
            }
        };

        // todo: Construct the matrix manually and drop the `printpdf` dependency.
        let mut operations = {
            // Document space has y = 0 at the top; PDF space has it at the bottom. Rather than
            // converting coordinates everywhere, we just flip everything on the horizontal axis
            // using a negative y scale followed by a translation. While doing that, we also scale
            // the document contents to fit our chosen page dimensions.
            let matrix = printpdf::CurTransMat::combine_matrix(
                printpdf::CurTransMat::Scale(pt_per_unit, -pt_per_unit).as_array(),
                printpdf::CurTransMat::Translate(printpdf::Pt(0.0), printpdf::Pt(page_h_pt))
                    .as_array(),
            );

            vec![op_gen::set_transformation_matrix(matrix)]
        };

        // Maps names to graphics states. This will go directly into the PDF.
        let mut graphics_states = lopdf::dictionary! {};

        // Maps tools to graphics state names. We use this to build the other map while avoiding
        // duplicates. We could go without this second map and derive unique graphics state names
        // from the tools in the other map, but then we'd have to construct a new string every time
        // we wanted to check if there is already a graphics state for a tool.
        let mut tool_graphics_state_names = HashMap::new();

        for layer in page.layers() {
            let objects = layer.objects();
            let mut strokes_handled = 0;

            let objects_bar = multi_progress
                .add(ProgressBar::new(objects.len() as _))
                .with_style(
                    ProgressStyle::with_template(
                        "Processing objects [{bar:40}] {percent}% [{pos}/{len}]",
                    )
                    .unwrap()
                    .progress_chars("# "),
                );

            let strokes =
                objects
                    .iter()
                    .inspect(|_| objects_bar.inc(1))
                    .filter_map(|obj| match obj {
                        sdocx::DocObject::Stroke(stroke) => Some(stroke),
                        _ => None,
                    });

            let strokes_by_tool = strokes.chunk_by(|stroke| Tool::for_stroke(stroke));

            for (tool, strokes) in &strokes_by_tool {
                // Get the extended graphics state required by this tool, creating it if it does
                // not yet exist.
                let gs_name = tool_graphics_state_names
                    .entry(tool.clone())
                    .or_insert_with(|| {
                        let name = format!("egs{}", graphics_states.len());
                        graphics_states.set(name.clone(), tool.create_egs());
                        name
                    });

                tool.draw_events(
                    gs_name,
                    (page_w_internal, page_h_internal),
                    strokes
                        .inspect(|_| strokes_handled += 1)
                        .map(|s| s.events()),
                    &mut operations,
                )
                .unwrap();
            }
        }

        // Media/trim/crop box for the page.
        let mtc_box: Vec<lopdf::Object> = vec![
            0.into(),
            0.into(),
            (page_w_pt.round() as i64).into(),
            (page_h_pt.round() as i64).into(),
        ];

        let content = lopdf::content::Content { operations };

        let contents_id = lpdf.add_object(lopdf::Stream::new(
            lopdf::dictionary! {},
            content.encode().unwrap(),
        ));

        let resources_id = lopdf::dictionary! {
            "ExtGState" => lpdf.add_object(graphics_states),
        };

        let page = lopdf::dictionary! {
            "Type" => "Page",
            "MediaBox" => mtc_box.clone(),
            "TrimBox" => mtc_box.clone(),
            "CropBox" => mtc_box,
            "Parent" => pages_id,
            "Resources" => resources_id,
            "Contents" => contents_id,
        };

        let page_id = lpdf.new_object_id();
        lpdf.set_object(page_id, page);
        page_ids.push(lopdf::Object::Reference(page_id));
    }

    lpdf.set_object(
        pages_id,
        lopdf::dictionary! {
            "Type" => "Pages",
            "Count" => page_ids.len() as i64,
            "Kids" => page_ids,
        },
    );

    let catalog_ref: lopdf::Object = lpdf.add_object(catalog).into();

    let doc_info_ref: lopdf::Object = lpdf
        .add_object(lopdf::dictionary! {
            "Title" => lopdf::Object::string_literal(document_name),
        })
        .into();

    lpdf.trailer.set("Root", catalog_ref);
    lpdf.trailer.set("Info", doc_info_ref);

    let _ = multi_progress.clear();

    let out_path = "/tmp/doc.pdf";

    let write_spinner = ProgressBar::no_length()
        .with_style(
            ProgressStyle::with_template("{spinner} {wide_msg}")
                .unwrap()
                .tick_chars("-\\|/ "),
        )
        .with_message(format!("Saving to '{out_path}'"));

    write_spinner.enable_steady_tick(Duration::from_millis(130));

    lpdf.compress();
    lpdf.save_modern(&mut std::fs::File::create(out_path).expect("failed to open output file"))
        .expect("failed to write PDF");

    let mb = f64::from(std::fs::metadata("/tmp/doc.pdf").unwrap().size() as u32) / 1000000f64;
    write_spinner.finish_and_clear();

    eprintln!("{mb} MB");
}
