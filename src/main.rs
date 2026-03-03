#![warn(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::cast_sign_loss,
    clippy::char_lit_as_u8,
    clippy::checked_conversions,
    clippy::unnecessary_cast,
    clippy::cognitive_complexity,
    clippy::dbg_macro,
    clippy::debug_assert_with_mut_call,
    clippy::doc_link_with_quotes,
    clippy::doc_markdown,
    clippy::empty_line_after_outer_attr,
    clippy::float_cmp,
    clippy::float_cmp_const,
    clippy::float_equality_without_abs,
    keyword_idents,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::mod_module_files,
    non_ascii_idents,
    noop_method_call,
    clippy::option_if_let_else,
    clippy::redundant_pub_crate,
    clippy::semicolon_if_nothing_returned,
    clippy::shadow_unrelated,
    clippy::similar_names,
    clippy::suspicious_operation_groupings,
    clippy::todo,
    clippy::unseparated_literal_suffix,
    unused_crate_dependencies,
    unused_extern_crates,
    unused_import_braces,
    clippy::unused_self,
    clippy::used_underscore_binding,
    clippy::useless_let_if_seq,
    clippy::wildcard_dependencies,
    clippy::wildcard_imports,
    clippy::unnested_or_patterns,
    clippy::unneeded_field_pattern
)]

use crate::{
    byte_stream::ByteStreamLe,
    end_tag::{ModelEndTag, NoteSdkType},
    media_info::MediaInfo,
    note_doc::NoteDoc,
    page::Page,
    page_id_info::PageIdInfo,
};
use color_eyre::{
    Result,
    eyre::{Context, eyre},
};
use std::path::PathBuf;

mod bits;
mod byte_stream;
mod end_tag;
mod media_info;
mod note_doc;
mod page;
mod page_id_info;

/// Holds a generic vector of bytes.
///
/// A common pattern in the binary formats is a 32-bit size `n` followed
/// by `n` bytes. This structure is intended to store the bytes that occur in these
/// patterns without having to actually parse whatever they encode.
struct OpaqueBytes {
    bytes: Vec<u8>,
}

impl OpaqueBytes {
    /// Reads `size: u32` and the `size` bytes that follow, reading `size + 4` bytes in total.
    fn try_parse_exclusive<T: ByteStreamLe>(stream: &mut T) -> Result<OpaqueBytes> {
        let size: usize = stream.read_u32_le()?.try_into()?;

        Ok(OpaqueBytes {
            bytes: stream.read_u8s(size)?,
        })
    }

    /// Reads `size: u32` and the `size - 4` bytes that follow, reading `size` bytes in total.
    fn try_parse_inclusive<T: ByteStreamLe>(stream: &mut T) -> Result<OpaqueBytes> {
        let size: usize = stream.read_u32_le()?.try_into()?;

        Ok(OpaqueBytes {
            bytes: stream.read_u8s(size.checked_sub(4).ok_or_else(|| {
                eyre!("Size ({size}) cannot be inclusive as it is less than 4")
            })?)?,
        })
    }
}

impl std::fmt::Debug for OpaqueBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpaqueBytes {{ ({} bytes) }}", self.bytes.len())
    }
}

#[derive(Debug)]
#[expect(dead_code)]
struct AppVersion {
    major: u32,
    minor: u32,
    patch_name: String,
}

impl AppVersion {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<AppVersion> {
        Ok(AppVersion {
            major: stream.read_u32_le()?,
            minor: stream.read_u32_le()?,
            patch_name: stream.read_short_u16_string()?,
        })
    }
}

fn demo_for_extracted_dir(dir_path: impl AsRef<str>) -> Result<()> {
    let dir_path = dir_path.as_ref();

    let media_info_path: PathBuf = [dir_path, "media/mediaInfo.dat"].iter().collect();
    let media_info = MediaInfo::try_parse(&mut std::fs::File::open(&media_info_path)?)?;
    println!("{}: {media_info:#?}", media_info_path.display());

    let end_tag_path: PathBuf = [dir_path, "end_tag.bin"].iter().collect();
    let end_tag =
        ModelEndTag::try_parse(&mut std::fs::File::open(&end_tag_path)?, NoteSdkType::SPen)?;
    // println!("{}: {end_tag:#?}", end_tag_path.display());

    let note_note_path: PathBuf = [dir_path, "note.note"].iter().collect();
    let note_note = NoteDoc::try_parse(&mut std::fs::File::open(&note_note_path)?)?;
    println!("{}: {note_note:#?}", note_note_path.display());

    let page_id_info_path: PathBuf = [dir_path, "pageIdInfo.dat"].iter().collect();
    let page_id_info = PageIdInfo::try_parse(&mut std::fs::File::open(&page_id_info_path)?)?;
    // println!("{}: {page_id_info:?}", page_id_info_path.display());

    for page_info in &page_id_info.pages {
        let mut page_path: PathBuf = [dir_path, &page_info.page_id].iter().collect();
        page_path.set_extension("page");

        let page = Page::try_parse_full(
            &mut std::fs::File::open(&page_path)
                .wrap_err_with(|| eyre!("Failed to open {}", page_path.display()))?,
        )?;

        // println!("{}: {page:#?}", page_path.display());
    }

    Ok(())
}

fn demo_all() -> Result<()> {
    let extracted_sdocx_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632",
        "/home/alex/projects/re/sdocx/sample_docs/empty encrypted_260219_125722",
        "/home/alex/projects/re/sdocx/sample_docs/Typed, formatted text with summary and voice memo_260220_003622",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features_260220_005438",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features plus dupes_260220_010554",
        "/home/alex/projects/re/sdocx/sample_docs/uses handwriting recognition and pages_260220_185052",
        "/home/alex/projects/re/sdocx/sample_docs/automatic shape recognition_260222_221513",
        "/home/alex/projects/re/sdocx/sample_docs/Shape text_260224_122639",
        "/home/alex/projects/re/sdocx/sample_docs/Maths_260227_150540",
        "/home/alex/projects/re/sdocx/sample_docs/Different pens_260228_134854",
        "/home/alex/projects/re/sdocx/sample_docs/Non Stroke objects_260228_134617",
        "/home/alex/projects/re/sdocx/sample_docs/web_260303_103930",
        "/home/alex/projects/re/sdocx/sample_docs/maths objects_260303_110957",
    ];

    for path in extracted_sdocx_paths {
        demo_for_extracted_dir(path)?;
    }

    Ok(())
}

// .ssf is "snap saved file"
// https://github.com/fschutt/printpdf

fn main() -> Result<()> {
    color_eyre::install()?;

    demo_all().inspect_err(|err| println!("source error: {:?}", err.source()))
}
