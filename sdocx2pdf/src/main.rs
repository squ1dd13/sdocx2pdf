use std::{
    collections::{HashMap, hash_map::Entry},
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use anyhow::Context;
use clap::{Parser, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::{Either, Itertools};
use jiff::{
    Timestamp,
    fmt::{StdIoWrite, strtime::BrokenDownTime},
};
use log::{info, warn};
use num::ToPrimitive;
use sdocx::{Document, DocumentError, MediaStorage, PageModel, page::object::InlineObject};
use thiserror::Error;

use crate::{page::PageConversionCtx, pdf::dictionary};

mod page;
mod pdf;
mod shape;
mod stroke;
mod tool;

const DETAILED_ERRORS_ARG_NAME: &str = "--detailed-errors";

#[derive(ValueEnum, Clone)]
enum BasicSplitMode {
    #[value(help = "Split the document into portrait A4 pages")]
    A4Portrait,
    #[value(help = "Split the document into landscape A4 pages")]
    A4Landscape,
}

fn parse_stroke_width_mul(s: &str) -> Result<f32, String> {
    // Extremely small width multipliers cause numerical problems in the stroke processing, and
    // extremely large values confuse PDF readers.
    // !!! - Do not change these bounds without updating the help text for the CLI options.
    let v @ 0.01..=10.0 = s.parse::<f32>().map_err(|e| e.to_string())? else {
        return Err("multiplier must be between 0.01 and 10.0 (inclusive)".to_string());
    };

    Ok(v)
}

/// A tool for converting Samsung Notes documents to vector PDFs. "Vector" means that
/// handwriting data is stored mathematically (as equations for curves) rather than as pixel data
/// (an image). This makes writing clearer and easier to read.
#[derive(Parser)]
#[command(
    version,
    about = "Converts Samsung Notes documents to vector PDFs",
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

    /// Specifies a multiplier for the widths of the fountain pen, calligraphy pen, ink pen,
    /// calligraphy brush and pencil.
    ///
    /// Minimum is 0.01 (1% of the usual width); maximum is 10 (10 times the usual width). For
    /// example, choosing a value of 2 (or 2.0) doubles the width of anything drawn with one of
    /// those tools.
    #[arg(
        long,
        default_value_t = 1.0,
        value_parser = parse_stroke_width_mul,
        help = "Scale the widths of all handwriting pens by some factor between 0.01 and 10",
        long_help
    )]
    pen_width_multiplier: f32,

    /// Specifies a multiplier for the widths of the marker pen and highlighter.
    ///
    /// Minimum is 0.01 (1% of the usual width); maximum is 10 (10 times the usual width). For
    /// example, choosing a value of 2 (or 2.0) doubles the width of anything drawn with one of
    /// those tools.
    #[arg(
        long,
        default_value_t = 1.0,
        value_parser = parse_stroke_width_mul,
        help = "Scale the widths of all highlighters and marker pens by some factor \
        between 0.01 and 10",
        long_help
    )]
    marker_width_multiplier: f32,

    // ! - Do not rename this without changing `DETAILED_ERRORS_ARG_NAME` to match.
    #[arg(
        long,
        help = "Show (very) detailed error messages if opening/parsing/converting fails"
    )]
    detailed_errors: bool,
}

/// Looks for `key` in `current_dict` and its parents, climbing up the tree either until it reaches
/// the top or finds a (grand)*parent that contains the key.
fn get_inherited_attr<'dc>(
    mut current_dict: &'dc pdf::Dictionary,
    key: &[u8],
    doc: &'dc pdf::Pdf,
) -> Option<&'dc pdf::Object> {
    loop {
        if let Ok(v) = current_dict.get(key) {
            return Some(v);
        }

        match current_dict.get(b"Parent") {
            Ok(&pdf::Object::Reference(parent_id)) => {
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
    Pdf(#[from] pdf::Error),

    #[error("page has no MediaBox entry")]
    MissingMediaBox,

    #[error("page has no Resources entry")]
    MissingResources,
}

struct EmbeddedPdf {
    /// The IDs in the destination PDF of the pages copied over from the source PDF, in order.
    src_page_ids: Vec<pdf::ObjectId>,
}

impl EmbeddedPdf {
    fn embed(
        src_name: impl AsRef<Path>,
        media_storage: &mut sdocx::MediaStorage,
        dest_pdf: &mut pdf::Pdf,
    ) -> Result<EmbeddedPdf, EmbeddedPdfError> {
        // Open and parse the PDF we're embedding.
        let mut src_pdf = pdf::Pdf::load_from(media_storage.open_file(src_name)?)?;

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
        dest_pdf: &mut pdf::Pdf,
    ) -> Result<(pdf::ObjectId, f32, f32), EmbeddedPdfError> {
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

        let (src_width, src_height, src_left, src_bottom) = {
            // [left, bottom, right, top]. Can be `Integer`s or `Real`s, but `as_float` doesn't
            // care which.
            let a = media_box.as_array()?;

            (
                dest_pdf.dereference(&a[2])?.1.as_float()?
                    - dest_pdf.dereference(&a[0])?.1.as_float()?,
                dest_pdf.dereference(&a[3])?.1.as_float()?
                    - dest_pdf.dereference(&a[1])?.1.as_float()?,
                dest_pdf.dereference(&a[0])?.1.as_float()?,
                dest_pdf.dereference(&a[1])?.1.as_float()?,
            )
        };

        // Even though the source page won't show up in the destination as a normal page, the
        // object is still in there, so we can ask the destination PDF for the content.
        let content = dest_pdf.get_page_content(page_id)?;

        let xobj_dict = pdf::dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "FormType" => 1,
            "BBox" => media_box.clone(),
            // Translate the content back to the origin so it can be positioned using the graphics
            // state transformation matrix without awareness of its original position.
            "Matrix" => pdf::matrix_vec([1.0, 0.0, 0.0, 1.0, -src_left, -src_bottom]),
            "Resources" => resources.clone(),
        };

        // Add a `Stream` object containing the XObject stream.
        let xobj_id =
            dest_pdf.add_object(pdf::Object::Stream(pdf::Stream::new(xobj_dict, content)));

        Ok((xobj_id, src_width, src_height))
    }
}

fn group_inline_objects_by_page(document: &Document) -> Vec<Vec<&InlineObject>> {
    let mut by_page = vec![Vec::new(); document.pages().len()];

    let pageless = matches!(document.page_model(), PageModel::Pageless);

    for inline_obj in document.body_text().inline_objects() {
        match inline_obj.object.page_index() {
            Some(i) => by_page[i as usize].push(inline_obj),

            // If no page is specified and this is a pageless document, put the inline object on
            // the first (only) page.
            None if pageless => by_page[0].push(inline_obj),

            None => {
                warn!(
                    "Ignoring inline {} because it does not specify a page, \
                    and this is not a pageless document",
                    <&str>::from(&inline_obj.object),
                );
            }
        }
    }

    by_page
}

fn create_document_pdf(
    document: &Document,
    media_storage: &mut MediaStorage,
    document_name: &str,
    pageless: bool,
    multi_progress: &MultiProgress,
    args: &Args,
) -> Result<pdf::Pdf, anyhow::Error> {
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

    let mut pdf = pdf::Pdf::with_version("1.5");

    // (Used `printpdf::serialize::to_pdf_doc` as a reference for the basic setup)
    let pages_id = pdf.new_object_id();

    let catalog = pdf::dictionary! {
        "Type" => "Catalog",
        "PageLayout" => "OneColumn",
        "PageMode" => "UseNone",
        "Pages" => pages_id,
    };

    let mut page_id_refs = Vec::with_capacity(document.pages().len());

    const A4_PTRT_WIDTH_PT: f32 = 210.0 * 2.84526;
    const A4_PTRT_HEIGHT_PT: f32 = 297.0 * 2.84526;

    // Maps the names of PDF files to `EmbeddedPdf`s that can be used to place pages from the PDFs
    // into the output PDF.
    let mut embedded_pdfs = HashMap::new();

    let mut auto_split_points = Vec::new();

    // Since we work in pages, we need the inline objects grouped by page.
    let inline_objects_by_page = group_inline_objects_by_page(document);

    // An inline object with `index_in_text == k` is represented in the raw string by an object
    // replacement character (U+FFFC) at character index `k`. Thus, if the entire raw string
    // consists of whitespace and object replacement characters, there is no meaningful text.
    if let Some(s) = document.body_text().raw_string()
        && s.chars().any(|c| !c.is_whitespace() && c != '\u{FFFC}')
    {
        warn!("Ignoring typed text in document body");
    }

    for (pos, (page_index, page)) in document.pages().iter().enumerate().with_position() {
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
            if pageless && let Some(drawn_rect) = page.drawn_rect() {
                // The sdocx tends to report an extra "page-height" worth of empty space at the end
                // of a pageless document. When the app exports a PDF, this space is not included,
                // and we don't want to include it either, so we subtract it from the height. Just
                // to be safe, we make sure not to reduce the height below the combined height of
                // the pages we'd need to hold the drawn content or embedded PDF content if this
                // were a paged document.

                // Distance from the top of the document to the lowest point drawn to.
                let drawn_floor_pt = drawn_rect.bottom as f32 * pt_per_unit;

                // Find the rectangle of the embedded PDF page furthest down the document.
                let lowest_pdf_rect = page
                    .embedded_pdf_pages()
                    .iter()
                    .map(|epp| epp.rect())
                    .max_by(|l, r| l.bottom.total_cmp(&r.bottom));

                let (floor_pt, assumed_page_height) = if let Some(rect_unit) = lowest_pdf_rect {
                    // Distance from the top to the lowest point reached by a page of an embedded
                    // PDF.
                    let pdf_floor_pt = rect_unit.bottom as f32 * pt_per_unit;

                    (
                        pdf_floor_pt.max(drawn_floor_pt),
                        // Assume the added space will be the same size as the last PDF page.
                        (rect_unit.bottom - rect_unit.top) as f32 * pt_per_unit,
                    )
                } else {
                    // Assume the added space will be the same height as an A4 page, as we've
                    // nothing else to go on (and this is usually correct).
                    (drawn_floor_pt, A4_PTRT_HEIGHT_PT)
                };

                let used_page_count = (floor_pt / assumed_page_height).ceil();
                let reduced_page_count =
                    (page_h_internal * pt_per_unit) / assumed_page_height - 1.0;

                reduced_page_count.max(used_page_count) * assumed_page_height
            } else {
                page_h_internal * pt_per_unit
            }
        };

        let mut page_ctx = PageConversionCtx::new((page_w_internal, page_h_internal));

        if let Some([b, g, r, a]) = page.background_colour() {
            let name = page_ctx.add_graphics_dict(pdf::dictionary! {
                "Type" => "ExtGState",
                // Fill alpha
                "ca" => a as f32 / 255.0,
            });

            page_ctx.ops.extend([
                pdf::save_graphics_state(),
                pdf::load_graphics_dict(name),
                pdf::set_fill_colour(r, g, b),
                pdf::specify_rectangle([0.0, 0.0, page_w_pt, page_h_pt]),
                pdf::fill(),
                pdf::restore_graphics_state(),
            ]);
        }

        // Add any embedded PDF pages before drawing the page objects.
        for emb_page in page.embedded_pdf_pages().iter() {
            let emb_pdf_name = emb_page.file().name();

            // Get an existing `EmbeddedPdf` for the PDF in question or, if one does not exist,
            // create it by embedding the PDF into the one we're building.
            let embedded_pdf = match embedded_pdfs.entry(emb_pdf_name) {
                Entry::Occupied(occ) => occ.into_mut(),
                Entry::Vacant(vac) => vac.insert(
                    EmbeddedPdf::embed(emb_pdf_name, media_storage, &mut pdf)
                        .with_context(|| format!("Failed to embed PDF '{emb_pdf_name}'"))?,
                ),
            };

            let emb_page_index = emb_page.page_index();

            let (xobj_id, src_width_pt, src_height_pt) = embedded_pdf
                .create_page_xobject(emb_page_index, &mut pdf)
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

            let xobj_name = page_ctx.add_xobject(xobj_id);

            page_ctx.ops.extend([
                pdf::save_graphics_state(),
                pdf::set_transformation_matrix([horiz_scale, 0.0, 0.0, vert_scale, x_pt, y_pt]),
                pdf::paint_xobject(xobj_name),
                pdf::restore_graphics_state(),
            ]);

            if args.auto_split {
                // If this is the first embedded page, it needs to go on a new page.
                if auto_split_points.is_empty() {
                    // Break at the top.
                    auto_split_points.push(y_pt + vert_scale * src_height_pt);
                }

                // Break at the bottom.
                auto_split_points.push(y_pt);
            }
        }

        page_ctx.ops.push({
            // Document space has y = 0 at the top; PDF space has it at the bottom. Rather than
            // converting coordinates everywhere, we just flip everything on the horizontal axis
            // using a negative y scale followed by a translation. While doing that, we also scale
            // the document contents to fit our chosen page dimensions.
            pdf::set_transformation_matrix([pt_per_unit, 0.0, 0.0, -pt_per_unit, 0.0, page_h_pt])
        });

        for inline_obj in &inline_objects_by_page[page_index] {
            if let Err(err) = page_ctx.draw_single_object(
                &inline_obj.object,
                args.pen_width_multiplier,
                args.marker_width_multiplier,
            ) {
                err.log();
            }
        }

        for layer in page.layers() {
            page_ctx.draw_layer(
                layer,
                args.pen_width_multiplier,
                args.marker_width_multiplier,
                multi_progress,
            );
        }

        let (ops, graphics_dicts, xobject_ids) = page_ctx.into_parts();

        let content = pdf::Content { operations: ops };

        let contents_id = pdf.add_object(pdf::Stream::new(
            pdf::Dictionary::new(),
            content.encode().context("Failed to encode page content")?,
        ));

        let resources_id = pdf::dictionary! {
            "ExtGState" => pdf.add_object(graphics_dicts),
            "XObject" => xobject_ids,
        };

        let page_base = pdf::dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => resources_id,
            "Contents" => contents_id,
        };

        // Split pages if basic splitting is enabled or auto-splitting is enabled and working.
        if args.basic_split.is_some() || (args.auto_split && !auto_split_points.is_empty()) {
            let split_points = if !auto_split_points.is_empty() {
                let mut asp = &auto_split_points[..];

                // In a lot of cases, the first and last embedded PDF pages align with the
                // beginning and end of the document. We use a 5pt margin to determine whether this
                // is the case at either end. If it is, we ignore the relevant split.
                if (page_h_pt - asp[0]) < 5.0 {
                    asp = &asp[1..];
                }

                if asp.last().is_some_and(|&p| p < 5.0) {
                    asp = &asp[..asp.len() - 1];
                }

                // Add precise points at the top and bottom of the document.
                Either::Left(
                    std::iter::once(page_h_pt)
                        .chain(asp.iter().copied())
                        .chain(std::iter::once(0.0)),
                )
            } else {
                Either::Right({
                    let split_page_height = match args.basic_split {
                        Some(BasicSplitMode::A4Portrait) => A4_PTRT_HEIGHT_PT,
                        Some(BasicSplitMode::A4Landscape) => {
                            // This is a pageless document, so we've already scaled it down
                            // to have A4 width. To get A4 aspect ratio (even if we don't currently
                            // resize again to make things properly A4) we need
                            A4_PTRT_WIDTH_PT / std::f32::consts::SQRT_2
                        }
                        // Per the outer `if`
                        None => unreachable!(),
                    };

                    let split_page_count: u32 =
                        (page_h_pt / split_page_height).ceil().to_u32().unwrap();

                    // If the desired height doesn't perfectly divide the document height, we'll be
                    // adding a bit onto the last page.
                    let _last_split_page_extension =
                        split_page_height - (page_h_pt % split_page_height);

                    // fixme: That will mess up the background colour, because we only filled as
                    // much as we needed before.

                    (0..=split_page_count)
                        .map(move |i| (i as f32).mul_add(-split_page_height, page_h_pt))
                })
            };

            let page_tops_bottoms = split_points.tuple_windows::<(_, _)>();

            for (top, bottom) in page_tops_bottoms {
                // We use the same content and resources for all of the pages, but shift the media
                // box to show different parts.
                // fixme: Some PDF readers really don't like that and take ages to load the pages.
                let mut page = page_base.clone();

                page.set(
                    b"MediaBox",
                    vec![0.0.into(), bottom.into(), page_w_pt.into(), top.into()],
                );

                let page_id = pdf.new_object_id();
                pdf.set_object(page_id, page);
                page_id_refs.push(pdf::Object::Reference(page_id));
            }
        } else {
            let mut page = page_base;

            page.set(
                b"MediaBox",
                vec![0.into(), 0.into(), page_w_pt.into(), page_h_pt.into()],
            );

            let page_id = pdf.new_object_id();
            pdf.set_object(page_id, page);
            page_id_refs.push(pdf::Object::Reference(page_id));
        }
    }

    pdf.set_object(
        pages_id,
        pdf::dictionary! {
            "Type" => "Pages",
            "Count" => page_id_refs.len() as i64,
            "Kids" => page_id_refs,
        },
    );

    let catalog_ref: pdf::Object = pdf.add_object(catalog).into();

    let doc_info_ref: pdf::Object = pdf
        .add_object(pdf::dictionary! {
            "Title" => pdf::Object::string_literal(document_name),
            "Creator" => pdf::Object::string_literal("sdocx2pdf"),
        })
        .into();

    pdf.trailer.set("Root", catalog_ref);
    pdf.trailer.set("Info", doc_info_ref);

    Ok(pdf)
}

fn main_convert(
    document: Document,
    mut media_storage: MediaStorage,
    multi_progress: &MultiProgress,
    mut args: Args,
) -> anyhow::Result<()> {
    let document_name = document.title_text().raw_string().unwrap_or("Missing name");

    let pageless = match document.page_model() {
        sdocx::PageModel::Paged => false,
        sdocx::PageModel::Pageless => true,
    };

    info!(
        "Successfully parsed {} document '{document_name}'",
        if pageless { "pageless" } else { "paged" },
    );

    if !pageless {
        args.auto_split = false;
        args.basic_split = None;
    }

    let mut pdf = create_document_pdf(
        &document,
        &mut media_storage,
        document_name,
        pageless,
        multi_progress,
        &args,
    )?;

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
    pdf.prune_objects();
    pdf.compress();

    pdf.save_modern(
        &mut std::fs::File::create(&args.out)
            .with_context(|| format!("Failed to create output file '{out_path_str}'"))?,
    )
    .context("Failed to save PDF to output file")?;

    let metadata_r = std::fs::metadata(&args.out);
    write_spinner.finish_and_clear();

    if let Ok(metadata) = metadata_r {
        info!(
            "Wrote {} to '{out_path_str}'.",
            indicatif::HumanBytes(metadata.len())
        );
    }

    Ok(())
}

fn print_report_request(detailed: bool) {
    if !detailed {
        eprintln!("For more detailed error messages, rerun with '{DETAILED_ERRORS_ARG_NAME}'.");
        eprintln!();
    }

    eprintln!("If you believe this should have worked, please do the following:");
    eprint!("  1. Ensure you are using the latest version of sdocx2pdf. ");

    if let Some(ver) = CARGO_PKG_VERSION {
        eprint!("You are currently using version {ver}. ");
    }

    eprintln!(
        "You can find the latest version at https://github.com/squ1dd13/sdocx2pdf/releases/latest. \
    If you are not using it, update and try again."
    );

    eprint!(
        "  2. If you are on the latest version or if the problem persists after updating, \
    report an issue on GitHub at https://github.com/squ1dd13/sdocx2pdf/issues/new. "
    );

    if detailed {
        eprintln!("Include the whole of this error message in your report.");
    } else {
        eprintln!(
            "Include the whole of the error message you get \
            with '{DETAILED_ERRORS_ARG_NAME}' in your report."
        );
    }
}

fn print_indented_string(s: String) {
    for line in s.lines() {
        eprintln!("    {line}");
    }
}

fn print_double_open_error(err_for_zip: DocumentError, err_for_dir: DocumentError, detailed: bool) {
    eprintln!();
    eprintln!("Tried opening as both a zip file and a directory, but got an error each time.");
    eprintln!();
    eprintln!("Got this error when trying to open as a zip:");

    print_indented_string(if detailed {
        format!("{err_for_zip:#?}")
    } else {
        format!("{err_for_zip}")
    });

    eprintln!();
    eprintln!("Got this error when trying to open as a directory:");

    print_indented_string(if detailed {
        format!("{err_for_dir:#?}")
    } else {
        format!("{err_for_dir}")
    });

    eprintln!();
    print_report_request(detailed);
}

const CARGO_PKG_VERSION: Option<&'static str> = option_env!("CARGO_PKG_VERSION");

fn print_intro() {
    if let Some(ver) = CARGO_PKG_VERSION {
        eprint!("This is sdocx2pdf {ver}. ");
    } else {
        eprint!("Unable to determine sdocx2pdf version. ");
    }

    eprintln!(
        "You can check for updates at https://github.com/squ1dd13/sdocx2pdf/releases/latest."
    );
}

fn main() -> ExitCode {
    let logger = env_logger::builder()
        .format(|mut buf, record| {
            BrokenDownTime::from(Timestamp::now())
                .format("%H:%M:%S%.3f", StdIoWrite(&mut buf))
                .unwrap();

            let style = buf.default_level_style(record.level());

            writeln!(
                buf,
                " {style}{}{style:#} [{}] {}",
                record.level(),
                record.target(),
                record.args()
            )
        })
        .filter_level(log::LevelFilter::Trace)
        .build();

    let multi = MultiProgress::new();

    indicatif_log_bridge::LogWrapper::new(multi.clone(), logger)
        .try_init()
        .unwrap();

    let args = Args::parse();
    let detailed_errors = args.detailed_errors;

    print_intro();

    let (document, media_storage) = match Document::from_zip(&args.doc) {
        Ok(v) => v,
        Err(err_for_zip) => match Document::from_dir(&args.doc) {
            Ok(v) => v,
            Err(err_for_dir) => {
                print_double_open_error(err_for_zip, err_for_dir, detailed_errors);
                return ExitCode::FAILURE;
            }
        },
    };

    if let Err(err) = main_convert(document, media_storage, &multi, args) {
        eprintln!("Encountered an error when converting the document:");

        print_indented_string(if detailed_errors {
            format!("{err:#?}")
        } else {
            format!("{err}")
        });

        eprintln!();
        print_report_request(detailed_errors);

        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
