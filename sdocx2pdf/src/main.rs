use std::{
    collections::{HashMap, hash_map::Entry},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use clap::{Parser, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use lopdf::{Dictionary as PdfDict, Document as Pdf, dictionary};
use num::ToPrimitive;
use sdocx::{Document, DocumentError, ZipError};
use thiserror::Error;

use crate::tool::Tool;

mod op_gen;
mod stroke;
mod tool;

// fixme: If splitting extends a page, the background colour will not be extended
// fixme: If splitting results in landscape pages, they won't be A4 because of how we set the width
// todo: Defer setting the background colour until the split-page rescaling step if we're splitting

#[derive(ValueEnum, Clone)]
enum BasicSplitMode {
    #[value(help = "Split the document into portrait A4 pages")]
    A4Portrait,
    #[value(help = "Split the document into landscape A4 pages")]
    A4Landscape,
}

/// A tool for converting Samsung Notes documents to vectorised PDFs. "Vectorised" means that
/// handwriting data is stored mathematically (as equations for curves) rather than as pixel data
/// (an image). This makes writing clearer and easier to read.
#[derive(Parser)]
#[command(
    version,
    about = "Converts Samsung Notes documents to vectorised PDFs",
    long_about
)]
struct Args {
    /// The path to the Samsung Notes document to be converted. This is typically an SDOCX file
    /// (.sdocx).
    ///
    /// The Windows app stores unexported documents as directories that have the same internal
    /// structure as SDOCX files. You can also pass the path to one of these directories, or to a
    /// directory containing the contents of an unzipped SDOCX file.
    #[arg(id = "IN", help = "Path to .sdocx file", long_help)]
    doc: PathBuf,

    /// The path to which the produced PDF will be written. If it already exists, the file will be
    /// overwritten.
    #[arg(help = "Path to write the PDF to", long_help)]
    out: PathBuf,

    /// Inserts page breaks into pageless documents between pages of any embedded PDFs.
    /// Disabled by default.
    ///
    /// By default, a pageless document will be converted to a PDF containing a long single page.
    /// With auto-splitting enabled, if a pageless document embeds any PDFs, page breaks are
    /// inserted to match the page breaks in the embedded PDFs. For example, if you import a
    /// five-page PDF into a blank pageless document and annotate it, auto-splitting will give you
    /// a five-page PDF rather than a single-page PDF.
    ///
    /// This option does nothing when converting a paged document. It also does nothing for
    /// pageless documents that do not embed any PDFs; see the basic splitting option.
    #[arg(
        long,
        help = "Add page breaks to pageless documents matching breaks in any embedded PDFs",
        long_help
    )]
    auto_split: bool,

    /// Specifies the page-splitting behaviour used for pageless documents when auto-splitting is
    /// not in effect, either because it is disabled or because the document being converted does
    /// not embed any PDFs.
    ///
    /// Basic splitting is disabled by default, resulting in long single-page PDFs when
    /// auto-splitting is not used. To use basic splitting only, specify a mode and do not enable
    /// auto-splitting. When basic splitting and auto-splitting are both enabled, basic splitting
    /// is used as a fallback when there are no PDFs embedded in the document. If auto-splitting is
    /// enabled but basic splitting is not, documents that embed PDFs will be auto-split, but those
    /// that don't will not be split at all.
    #[arg(
        long,
        help = "Page-splitting mode for pageless documents without embedded PDFs \
        or for when auto-splitting is disabled",
        long_help
    )]
    basic_split: Option<BasicSplitMode>,
}

/// Looks for `key` in `current_dict` and its parents, climbing up the tree either until it reaches
/// the top or finds a (grand)*parent that contains the key.
fn get_inherited_attr<'dc>(
    mut current_dict: &'dc PdfDict,
    key: &[u8],
    doc: &'dc Pdf,
) -> Option<&'dc lopdf::Object> {
    loop {
        if let Ok(v) = current_dict.get(key) {
            return Some(v);
        }

        match current_dict.get(b"Parent") {
            Ok(&lopdf::Object::Reference(parent_id)) => {
                current_dict = doc.get_dictionary(parent_id).ok()?;
            }

            _ => return None,
        };
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
enum EmbeddedPdfError {
    Io(#[from] std::io::Error),
    Pdf(#[from] lopdf::Error),

    #[error("page has no MediaBox entry")]
    MissingMediaBox,

    #[error("page has no Resources entry")]
    MissingResources,
}

struct EmbeddedPdf {
    /// The IDs in the destination PDF of the pages copied over from the source PDF, in order.
    src_page_ids: Vec<lopdf::ObjectId>,
}

impl EmbeddedPdf {
    fn embed(
        src_name: impl AsRef<Path>,
        media_storage: &mut sdocx::MediaStorage,
        dest_pdf: &mut Pdf,
    ) -> Result<EmbeddedPdf, EmbeddedPdfError> {
        // Open and parse the PDF we're embedding.
        let mut src_pdf = Pdf::load_from(media_storage.open_file(src_name)?)?;

        // Renumber the objects in the source so their IDs don't collide with those in the
        // destination. This lets us move objects from the source to the destination directly,
        // including images, fonts, etc.
        src_pdf.renumber_objects_with(dest_pdf.max_id + 1);

        // `page_iter` is in order, so the nth element of this vector is the ID of the nth source
        // page. This is useful because `sdocx` files refer to pages by indices.
        let src_page_ids: Vec<_> = src_pdf.page_iter().collect();

        // Move all the objects from the source over to the destination.
        dest_pdf.objects.extend(src_pdf.objects);

        // Having manually inserted objects, we must manually update the max ID.
        dest_pdf.max_id = src_pdf.max_id;

        Ok(EmbeddedPdf { src_page_ids })
    }

    /// Adds to `dest_pdf` an XObject containing the contents of the page at `index` in the source
    /// PDF. The ID of the XObject is returned along with the width and height of the source page.
    fn create_page_xobject(
        &self,
        index: u32,
        dest_pdf: &mut Pdf,
    ) -> Result<(lopdf::ObjectId, f32, f32), EmbeddedPdfError> {
        let page_id = self.src_page_ids[index as usize];

        let (media_box, resources) = {
            let dict = dest_pdf.get_object(page_id)?.as_dict()?;

            (
                get_inherited_attr(dict, b"MediaBox", dest_pdf)
                    .ok_or(EmbeddedPdfError::MissingMediaBox)?,
                get_inherited_attr(dict, b"Resources", dest_pdf)
                    .ok_or(EmbeddedPdfError::MissingResources)?,
            )
        };

        let (src_width, src_height) = {
            // [x, y, width, height]. Can be `Integer`s or `Real`s, but `as_float` doesn't care
            // which.
            let a = media_box.as_array()?;

            (
                dest_pdf.dereference(&a[2])?.1.as_float()?,
                dest_pdf.dereference(&a[3])?.1.as_float()?,
            )
        };

        // Even though the source page won't show up in the destination as a normal page, the
        // object is still in there, so we can ask the destination PDF for the content.
        let content = dest_pdf.get_page_content(page_id)?;

        let xobj_dict = lopdf::dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "FormType" => 1,
            "BBox" => media_box.clone(),
            "Resources" => resources.clone(),
        };

        // Add a `Stream` object containing the XObject stream.
        let xobj_id = dest_pdf.add_object(lopdf::Object::Stream(lopdf::Stream::new(
            xobj_dict, content,
        )));

        Ok((xobj_id, src_width, src_height))
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Try to read the document as though it's a zip file.
    let (document, mut media_storage) = match Document::from_zip(&args.doc) {
        Ok(v) => v,
        // If that fails because it's a directory and not a zip, read the directory instead.
        Err(DocumentError::Io(e) | DocumentError::Zip(ZipError::Io(e)))
            if e.kind() == std::io::ErrorKind::IsADirectory =>
        {
            Document::from_dir(args.doc).context("Failed to read document as directory")?
        }
        Err(e) => return Err(e).context("Failed to read document as zip file"),
    };

    let mut lpdf = Pdf::with_version("1.5");

    let document_name = document.title_text().raw_string().unwrap_or("Invalid name");

    let pageless = match document.page_model() {
        sdocx::PageModel::Paged => false,
        sdocx::PageModel::Pageless => true,
    };

    eprintln!(
        "Opened {} document '{document_name}'.",
        if pageless { "pageless" } else { "paged" }
    );

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

    // (Used `printpdf::serialize::to_lopdf_doc` as a reference for the basic setup)
    let pages_id = lpdf.new_object_id();

    let catalog = lopdf::dictionary! {
        "Type" => "Catalog",
        "PageLayout" => "OneColumn",
        "PageMode" => "UseNone",
        "Pages" => pages_id,
    };

    let mut page_ids = Vec::with_capacity(document.pages().len());

    const A4_PTRT_WIDTH_PT: f32 = 210.0 * 2.84526;
    const A4_PTRT_HEIGHT_PT: f32 = 297.0 * 2.84526;

    // Maps the names of PDF files to `EmbeddedPdf`s that can be used to place pages from the PDFs
    // into the output PDF.
    let mut embedded_pdfs = HashMap::new();

    let mut combined_page_height = 0.0;
    let mut combined_page_width = 0.0;

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
        let pt_per_unit = A4_PTRT_WIDTH_PT / page_w_internal.min(page_h_internal);

        let page_w_pt = page_w_internal * pt_per_unit;

        let page_h_pt = {
            if pageless
                && page.embedded_pdf_pages().is_empty()
                && let Some(drawn_rect) = page.drawn_rect()
            {
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

        let mut operations = Vec::new();

        // Maps names to graphics states. This will go directly into the PDF.
        let mut graphics_states = lopdf::dictionary! {};

        if let Some([b, g, r, a]) = page.background_colour() {
            graphics_states.set(
                "fillbg",
                lopdf::dictionary! {
                    "Type" => "ExtGState",
                    // Fill alpha
                    "ca" => a as f32 / 255.0,
                },
            );

            operations.extend([
                op_gen::save_graphics_state(),
                op_gen::load_graphics_state("fillbg"),
                op_gen::set_fill_colour(r, g, b),
                op_gen::specify_rectangle([0.0, 0.0, page_w_pt, page_h_pt]),
                op_gen::fill(),
                op_gen::restore_graphics_state(),
            ]);
        }

        let mut xobjects = PdfDict::new();

        // Add any embedded PDF pages before drawing the page objects.
        for (emb_i, emb_page) in page.embedded_pdf_pages().iter().enumerate() {
            let emb_pdf_name = emb_page.file().name();

            // Get an existing `EmbeddedPdf` for the PDF in question or, if one does not exist,
            // create it by embedding the PDF into the one we're building.
            let embedded_pdf = &*match embedded_pdfs.entry(emb_pdf_name) {
                Entry::Occupied(occ) => occ.into_mut(),
                Entry::Vacant(vac) => vac.insert(
                    EmbeddedPdf::embed(emb_pdf_name, &mut media_storage, &mut lpdf)
                        .with_context(|| format!("Failed to embed PDF '{emb_pdf_name}'"))?,
                ),
            };

            let emb_page_index = emb_page.page_index();

            let (xobj_id, src_width_pt, src_height_pt) = embedded_pdf
                .create_page_xobject(emb_page_index, &mut lpdf)
                .with_context(|| {
                    format!(
                        "Failed to embed page {} of PDF '{emb_pdf_name}'",
                        emb_page_index + 1
                    )
                })?;

            // We have to scale and translate the embedded page to fit inside the prescribed
            // rectangle.
            let (x_pt, y_pt, horiz_scale, vert_scale) = {
                let sdocx::page::Rect {
                    left,
                    top,
                    right,
                    bottom,
                } = emb_page.rect();

                // y = 0 at the top in document space, so `bottom > top`.
                let dest_width_units = (right - left) as f32;
                let dest_height_units = (bottom - top) as f32;

                let x_pt = left as f32 * pt_per_unit;
                let horiz_scale = (dest_width_units * pt_per_unit) / src_width_pt;

                // The document gives us the vertical position of the lower-left corner in document
                // space, so we have to flip it. We don't use a negative vertical scale because the
                // content of the page being embedded lives in PDF space, so is already the correct
                // way up.
                let y_pt = page_h_pt - bottom as f32 * pt_per_unit;
                let vert_scale = (dest_height_units * pt_per_unit) / src_height_pt;

                (x_pt, y_pt, horiz_scale, vert_scale)
            };

            // Name the XObject and add it to the XObject dictionary. The name doesn't matter, as
            // long as it's unique.
            let xobj_name = format!("embpage{emb_i}");
            xobjects.set(xobj_name.clone(), xobj_id);

            operations.extend([
                op_gen::save_graphics_state(),
                op_gen::set_transformation_matrix([horiz_scale, 0.0, 0.0, vert_scale, x_pt, y_pt]),
                lopdf::content::Operation::new("Do", vec![xobj_name.into()]),
                op_gen::restore_graphics_state(),
            ]);
        }

        operations.push({
            // Document space has y = 0 at the top; PDF space has it at the bottom. Rather than
            // converting coordinates everywhere, we just flip everything on the horizontal axis
            // using a negative y scale followed by a translation. While doing that, we also scale
            // the document contents to fit our chosen page dimensions.
            op_gen::set_transformation_matrix([pt_per_unit, 0.0, 0.0, -pt_per_unit, 0.0, page_h_pt])
        });

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

                if let Err(()) = tool.draw_events(
                    gs_name,
                    (page_w_internal, page_h_internal),
                    strokes
                        .inspect(|_| strokes_handled += 1)
                        .map(|s| s.events()),
                    &mut operations,
                ) {
                    eprintln!("Failed to draw stroke");
                }
            }
        }

        combined_page_width += page_w_pt.round();
        combined_page_height += page_h_pt.round();

        // Media/trim/crop box for the page. (Trim box defaults to crop box, which defaults to
        // media box.)
        let media_box: Vec<lopdf::Object> = vec![
            0.into(),
            0.into(),
            (page_w_pt.round() as i64).into(),
            (page_h_pt.round() as i64).into(),
        ];

        let content = lopdf::content::Content { operations };

        let contents_id = lpdf.add_object(lopdf::Stream::new(
            lopdf::dictionary! {},
            content.encode().context("Failed to encode page content")?,
        ));

        let resources_id = lopdf::dictionary! {
            "ExtGState" => lpdf.add_object(graphics_states),
            "XObject" => xobjects,
        };

        let page = lopdf::dictionary! {
            "Type" => "Page",
            "MediaBox" => media_box,
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
            "Creator" => lopdf::Object::string_literal("sdocx2pdf"),
        })
        .into();

    lpdf.trailer.set("Root", catalog_ref);
    lpdf.trailer.set("Info", doc_info_ref);

    let _ = multi_progress.clear();

    let out_path_str = args.out.to_string_lossy();

    let write_spinner = ProgressBar::no_length()
        .with_style(
            ProgressStyle::with_template("{spinner} {wide_msg}")
                .unwrap()
                .tick_chars("-\\|/ "),
        )
        .with_message(format!("Saving to '{}'...", out_path_str));

    write_spinner.enable_steady_tick(Duration::from_millis(130));

    // Pruning unused objects is most important when embedding PDFs because there may be some large
    // unused objects if only some of the PDF is embedded (or if the PDF being embedded is poorly
    // optimised).
    lpdf.prune_objects();
    lpdf.compress();

    lpdf.save_modern(
        &mut std::fs::File::create(&args.out)
            .with_context(|| format!("Failed to create output file '{out_path_str}'"))?,
    )
    .context("Failed to save PDF to output file")?;

    let metadata_r = std::fs::metadata(&args.out);
    write_spinner.finish_and_clear();

    if let Ok(metadata) = metadata_r {
        eprintln!(
            "Wrote {} to '{out_path_str}'.",
            indicatif::HumanBytes(metadata.size())
        );
    }

    Ok(())
}
