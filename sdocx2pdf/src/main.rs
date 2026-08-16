use std::{
    collections::{HashMap, hash_map::Entry},
    io::{Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, anyhow};
use clap::{Parser, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use jiff::{
    Timestamp,
    fmt::{StdIoWrite, strtime::BrokenDownTime},
};
use krilla::{
    geom::{Size, Transform},
    metadata::Metadata,
    page::PageSettings,
    pdf::{Pdf, PdfDocument},
};
use log::{info, warn};
use num::ToPrimitive;
use sdocx::{Document, DocumentError, MediaStorage, PageModel, page::object::InlineObject};

use crate::page::PageConversionCtx;

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
    pageless: bool,
    multi_progress: &MultiProgress,
    args: &Args,
) -> Result<krilla::Document, anyhow::Error> {
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

    let mut pdf = krilla::Document::new();

    const A4_PTRT_WIDTH_PT: f32 = 210.0 * 2.84526;
    const A4_PTRT_HEIGHT_PT: f32 = 297.0 * 2.84526;

    // Maps the names of PDF files to `PdfDocument`s that can be used to place pages from the PDFs
    // into the output PDF.
    let mut embedded_pdfs = HashMap::new();

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
        // the app, this results in A4-sized pages for both portrait and landscape. For pageless
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

        let mut pdf_page = pdf.start_page_with(PageSettings::new(
            Size::from_wh(page_w_pt, page_h_pt).expect("page size calculation failed"),
        ));

        let mut page_ctx = PageConversionCtx::new(
            (page_w_internal, page_h_internal),
            pt_per_unit,
            pdf_page.surface(),
        );

        if let Some([b, g, r, a]) = page.background_colour() {
            page_ctx.fill_background(r, g, b, a);
        }

        // Add any embedded PDF pages before drawing the page objects.
        for emb_page in page.embedded_pdf_pages().iter() {
            let emb_pdf_name = emb_page.file().name();

            // Usually, there are many pages embedded that come from the same PDF, in which
            // case it is likely that this is not the first page we're embedding from the PDF.
            // Find the already-loaded PDF object, or load it from the media storage if it is
            // actually the first time we're embedding something from this PDF.
            let embedded_pdf = match embedded_pdfs.entry(emb_pdf_name) {
                Entry::Occupied(occ) => occ.into_mut(),
                Entry::Vacant(vac) => {
                    let mut pdf_bytes = Vec::new();

                    media_storage
                        .open_file(emb_pdf_name)
                        .with_context(|| format!("Failed to open embedded PDF '{emb_pdf_name}'"))?
                        .read_to_end(&mut pdf_bytes)
                        .with_context(|| format!("Failed to read embedded PDF '{emb_pdf_name}'"))?;

                    let pdf = Pdf::new(pdf_bytes).map_err(|e| {
                        anyhow::anyhow!("Failed to parse embedded PDF '{emb_pdf_name}': {e:?}")
                    })?;

                    vac.insert(PdfDocument::new(Arc::new(pdf)))
                }
            };

            let src_page_index = emb_page.page_index() as usize;
            let dest_rect = emb_page.rect();

            let dest_width = dest_rect.right - dest_rect.left;
            let dest_height = dest_rect.bottom - dest_rect.top;

            let dest_size =
                Size::from_wh(dest_width as f32, dest_height as f32).ok_or_else(|| {
                    anyhow!(
                        "Invalid destination size ({}, {}) for embedded PDF page",
                        dest_width,
                        dest_height,
                    )
                })?;

            // Transform to position the embedded page. The scaling is handled by `draw_pdf_page`
            // (which apparently doesn't want to bother with positioning...?)
            page_ctx.surface.push_transform(&Transform::from_row(
                1.0,
                0.0,
                0.0,
                1.0,
                dest_rect.left as f32,
                dest_rect.top as f32,
            ));

            page_ctx
                .surface
                .draw_pdf_page(embedded_pdf, dest_size, src_page_index);

            page_ctx.surface.pop();
        }

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
    }

    Ok(pdf)
}

fn main_convert(
    document: Document,
    mut media_storage: MediaStorage,
    multi_progress: &MultiProgress,
    args: Args,
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

    let mut pdf = create_document_pdf(
        &document,
        &mut media_storage,
        pageless,
        multi_progress,
        &args,
    )?;

    pdf.set_metadata(
        Metadata::new()
            .title(document_name.to_owned())
            .producer("sdocx2pdf".to_owned())
            .creator("Samsung Notes".to_owned()),
        // todo: Include date created/modified (included in the end tag)
        // Not bothering now because krilla uses its own structure for timestamps and we'd have to
        // build it from year, month, day, hour, etc. - no fun
    );

    let out_path_str = args.out.to_string_lossy();

    let write_spinner = ProgressBar::no_length()
        .with_style(
            ProgressStyle::with_template("{spinner} {wide_msg}")
                .unwrap()
                .tick_chars("-\\|/ "),
        )
        .with_message(format!("Saving to '{}'...", out_path_str));

    write_spinner.enable_steady_tick(Duration::from_millis(130));

    let pdf_bytes = pdf.finish().context("Failed to finalise PDF document")?;

    std::fs::write(&args.out, &pdf_bytes)
        .with_context(|| format!("Failed to write output file '{out_path_str}'"))?;

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
