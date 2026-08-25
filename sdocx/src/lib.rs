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

use byte_stream::{ByteStreamLe, ReadStringError, TryParse};
pub use euclid;
use std::io::Read;
use thiserror::Error;

pub use doc::{Document, DocumentError, MediaStorage};
pub use end_tag::{BackgroundTheme, DocumentType, PageModel, TextDirection};
pub use page::object::DocObject;
pub use zip::result::ZipError;

mod bits;
mod byte_stream;
mod context;
mod doc;
mod end_tag;
mod media_info;
mod note_doc;
pub mod page;
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

impl From<OpaqueBytes> for Vec<u8> {
    fn from(OpaqueBytes(v): OpaqueBytes) -> Self {
        v
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

pub enum SdocxSpace {}
pub type Point2d<T> = euclid::Point2D<T, SdocxSpace>;
pub type Vector2d<T> = euclid::Vector2D<T, SdocxSpace>;
pub type Box2d<T> = euclid::Box2D<T, SdocxSpace>;
pub type Size2d<T> = euclid::Size2D<T, SdocxSpace>;
pub type Length<T> = euclid::Length<T, SdocxSpace>;

impl<R: Read> TryParse<R> for Point2d<f32> {
    type ParseError = std::io::Error;

    fn try_parse(reader: &mut R) -> Result<Self, Self::ParseError> {
        Ok((reader.read_f32_le()?, reader.read_f32_le()?).into())
    }
}

impl<R: Read> TryParse<R> for Point2d<f64> {
    type ParseError = std::io::Error;

    fn try_parse(reader: &mut R) -> Result<Self, Self::ParseError> {
        Ok((reader.read_f64_le()?, reader.read_f64_le()?).into())
    }
}

impl<R: Read> TryParse<R> for Point2d<i32> {
    type ParseError = std::io::Error;

    fn try_parse(reader: &mut R) -> Result<Self, Self::ParseError> {
        Ok((reader.read_i32_le()?, reader.read_i32_le()?).into())
    }
}

impl<R: Read> TryParse<R> for Box2d<f32> {
    type ParseError = std::io::Error;

    fn try_parse(reader: &mut R) -> Result<Self, Self::ParseError> {
        let top_left = Point2d::try_parse(reader)?;
        let bottom_right = Point2d::try_parse(reader)?;

        Ok(Box2d::new(top_left, bottom_right))
    }
}

impl<R: Read> TryParse<R> for Box2d<f64> {
    type ParseError = std::io::Error;

    fn try_parse(reader: &mut R) -> Result<Self, Self::ParseError> {
        let top_left = Point2d::try_parse(reader)?;
        let bottom_right = Point2d::try_parse(reader)?;

        Ok(Box2d::new(top_left, bottom_right))
    }
}

pub fn try_parse_i32_box<R: Read>(reader: &mut R) -> std::io::Result<Box2d<f64>> {
    let top_left = Point2d::<i32>::try_parse(reader)?;
    let bottom_right = Point2d::<i32>::try_parse(reader)?;

    // Cast is OK here because `f64` can represent anything `i32` can.
    Ok(Box2d::new(top_left.cast(), bottom_right.cast()))
}
