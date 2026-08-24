use std::{io::Write, path::PathBuf, process::ExitCode, time::Duration};

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
};
use log::{info, warn};
use sdocx::{Document, DocumentError, MediaStorage};

use crate::page::{EmbedMap, PageConversionCtx};

mod page;
mod pdf;
mod shape;
mod stroke;
mod tool;

const DETAILED_ERRORS_ARG_NAME: &str = "--detailed-errors";

#[derive(ValueEnum, Clone, Copy)]
enum BasicSplitMode {
    #[value(
        help = "Split the document into portrait pages with a 1:√2 aspect ratio, \
    like ISO 216 paper sizes (A1, A2, A3, A4, etc.)"
    )]
    IsoPortrait,
    #[value(
        help = "Split the document into landscape pages with a √2:1 aspect ratio, \
    like ISO 216 paper sizes (A1, A2, A3, A4, etc.)"
    )]
    IsoLandscape,
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

    /// Inserts page breaks into pageless documents between pages of any embedded PDFs. Disabled by
    /// default.
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

    // !!! - Do not rename this without changing `DETAILED_ERRORS_ARG_NAME` to match.
    #[arg(
        long,
        help = "Show (very) detailed error messages if opening/parsing/converting fails"
    )]
    detailed_errors: bool,
}

fn create_document_pdf(
    document: &Document,
    media_storage: &mut MediaStorage,
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

    // An inline object with `index_in_text == k` is represented in the raw string by an object
    // replacement character (U+FFFC) at character index `k`. Thus, if the entire raw string
    // consists of whitespace and object replacement characters, there is no meaningful text.
    if let Some(s) = document.body_text().raw_string()
        && s.chars().any(|c| !c.is_whitespace() && c != '\u{FFFC}')
    {
        warn!("Ignoring typed text in document body");
    }

    let embed_map = EmbedMap::new(document, media_storage)?;

    let pages =
        page::split_document_into_pages(document, &embed_map, args.auto_split, args.basic_split)
            // Collect the pages because the iterator borrows `embed_map`, and we need to move out
            // of it.
            .collect_vec();

    let embedded_documents = embed_map.into_documents();

    for page in pages {
        pages_bar.as_ref().inspect(|pb| pb.inc(1));

        // todo: Move page-internal conversion logic into `page`

        let output_size = page.output_size();

        let mut pdf_page = pdf.start_page_with(PageSettings::new(
            Size::from_wh(output_size.width as f32, output_size.height as f32)
                .expect("page size calculation failed"),
        ));

        let mut page_ctx = PageConversionCtx::new(&page, pdf_page.surface());

        if let Some([b, g, r, a]) = page.background_colour() {
            page_ctx.fill_background(r, g, b, a);
        }

        // Add any embedded PDF pages before drawing the page objects.
        for embed in page.embeds() {
            let src_page_index = embed.page_index() as usize;
            let dest_rect = embed.rect();

            let dest_width = dest_rect.width();
            let dest_height = dest_rect.height();

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
                dest_rect.min.x as f32,
                dest_rect.min.y as f32,
            ));

            page_ctx.surface.draw_pdf_page(
                embedded_documents
                    .get(embed.file().name())
                    .ok_or_else(|| anyhow!("Missing embedded PDF '{}'", embed.file().name()))?,
                dest_size,
                src_page_index,
            );

            page_ctx.surface.pop();
        }

        for inline_obj in page.inline_objects() {
            if let Err(err) = page_ctx.draw_single_object(
                &inline_obj.object,
                args.pen_width_multiplier,
                args.marker_width_multiplier,
            ) {
                err.log();
            }
        }

        page_ctx.draw_objects(
            page.objects(),
            args.pen_width_multiplier,
            args.marker_width_multiplier,
            multi_progress,
        );
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

    let mut pdf = create_document_pdf(&document, &mut media_storage, multi_progress, &args)?;

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
