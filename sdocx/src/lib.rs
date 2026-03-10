#![warn(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::cast_sign_loss,
    clippy::char_lit_as_u8,
    clippy::checked_conversions,
    clippy::unnecessary_cast,
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
    byte_stream::{ByteStreamLe, ReadStringError, TryParse},
    doc::Document,
};
use std::{io::Read, path::Path};
use thiserror::Error;

mod bits;
mod byte_stream;
mod context;
mod doc;
mod end_tag;
mod media_info;
mod note_doc;
mod page;
mod page_list;

#[derive(Error, Debug)]
pub enum OpaqueBytesParseError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("can't fit size {0} into `usize`")]
    TooBig(u32),

    #[error("size {0} is too small to be inclusive")]
    TooSmall(u32),
}

/// Holds a vector of bytes.
///
/// A common pattern in the binary formats is a 32-bit size `n` followed
/// by `n` bytes. This structure is intended to store the bytes that occur in these
/// patterns without having to actually parse whatever they encode.
struct OpaqueBytes(Vec<u8>);

impl OpaqueBytes {
    /// Reads `size: u32` and the `size` bytes that follow, reading `size + 4` bytes in total.
    fn try_parse_exclusive<R: Read>(stream: &mut R) -> Result<OpaqueBytes, OpaqueBytesParseError> {
        let size = stream.read_u32_le()?;

        Ok(OpaqueBytes(
            stream.read_u8s(
                size.try_into()
                    .map_err(|_| OpaqueBytesParseError::TooBig(size))?,
            )?,
        ))
    }

    /// Reads `size: u32` and the `size - 4` bytes that follow, reading `size` bytes in total.
    fn try_parse_inclusive<R: Read>(stream: &mut R) -> Result<OpaqueBytes, OpaqueBytesParseError> {
        match stream.read_u32_le()? {
            too_small @ ..4 => Err(OpaqueBytesParseError::TooSmall(too_small)),
            size => Ok(OpaqueBytes(
                stream.read_u8s(
                    size.try_into()
                        .map_err(|_| OpaqueBytesParseError::TooBig(size))?,
                )?,
            )),
        }
    }
}

impl std::fmt::Debug for OpaqueBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpaqueBytes({} bytes)", self.0.len())
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum AppVersionParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
}

#[derive(Debug)]
#[expect(dead_code)]
struct AppVersion {
    major: u32,
    minor: u32,
    patch_name: String,
}

impl<R: Read> TryParse<R> for AppVersion {
    type ParseError = AppVersionParseError;

    fn try_parse(reader: &mut R) -> std::result::Result<AppVersion, AppVersionParseError> {
        Ok(AppVersion {
            major: reader.read_u32_le()?,
            minor: reader.read_u32_le()?,
            patch_name: reader.read_short_u16_string()?,
        })
    }
}

#[test]
fn test_all() {
    let sdocx_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/empty encrypted_260219_125722.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Typed, formatted text with summary and voice memo_260220_003622.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features_260220_005438.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features plus dupes_260220_010554.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/uses handwriting recognition and pages_260220_185052.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/automatic shape recognition_260222_221513.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Shape text_260224_122639.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Maths_260227_150540.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Different pens_260228_134854.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Non Stroke objects_260228_134617.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/web_260303_103930.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/maths objects_260303_110957.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/eraser_260304_103837.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Note replay_260304_170858.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/tilt_test___Notes_260304_194325.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/CAMDOWN__up down left right pressure inc_260304_202617.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Up down left right CAMRIGHT_260304_203137.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/V small shapes_260305_233455.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Normal-sized shapes_260305_234841.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/V small shapes scaled up_260306_132640.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/Large diamond_260306_135138.sdocx",
        "/home/alex/projects/re/sdocx/sample_docs/fromwindows__V small shapes_260307_000230___meeeeeeeeeeeeee.sdocx",
    ];

    for path in sdocx_paths {
        let _zipped = Document::from_zip(path).unwrap();
        let _extracted = Document::from_dir(Path::new(path).with_extension("")).unwrap();
    }
}
