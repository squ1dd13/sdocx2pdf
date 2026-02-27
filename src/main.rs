#![allow(unused)]
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
    byte_stream::{ByteStreamLe, ReadBitfieldError},
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
use std::{io::Write, path::PathBuf};
use thiserror::Error;

mod byte_stream;
mod end_tag;
mod media_info;
mod note_doc;
mod page;
mod page_id_info;

#[derive(Error, Debug)]
#[error("one or more bits were set but never checked: {0:#x} ({0:#b})")]
pub struct UnhandledBitsError(u32);

/// Wraps a 32-bit bitfield and tracks which bits have been queried.
///
/// If we read a bitfield from a binary file, we need to handle all of the bits that are used;
/// if a bit is set but we never check its value, it may lead to parsing errors later that are
/// hard to diagnose.
#[derive(Default, Clone, Copy, Debug)]
pub struct CheckedBitfield {
    /// The underlying bitfield.
    bits: u32,

    /// Stores which bits of `bits` have been queried.
    checked: u32,
}

impl CheckedBitfield {
    pub fn try_parse(stream: &mut impl ByteStreamLe) -> Result<CheckedBitfield, ReadBitfieldError> {
        Ok(CheckedBitfield {
            bits: stream.read_variable_length_bitfield()?,
            checked: 0,
        })
    }

    /// Returns `true` iff the `index`th bit is set, where 0 is the index of the least significant
    /// bit. Stores that this bit has been checked.
    pub const fn check_bit(&mut self, index: u8) -> bool {
        let mask = 1 << index;
        self.checked |= mask;
        self.bits & mask != 0
    }

    /// Returns `true` iff any bits are set.
    pub const fn any_set(self) -> bool {
        self.bits != 0
    }

    /// Returns an `UnhandledBitsError` containing the unhandled bits if there are any.
    pub const fn ensure_all_checked(self) -> Result<(), UnhandledBitsError> {
        // Match on the bits that are set in `bits` but not in `checked`.
        match self.bits & !self.checked {
            0 => Ok(()),
            bad => Err(UnhandledBitsError(bad)),
        }
    }
}

#[macro_export]
macro_rules! option_on_bit {
    ($bf:expr, $i:expr => $then:expr $(,)?) => {
        if $bf.check_bit($i) { Some($then) } else { None }
    };

    ($bf:expr, $i:expr => $then:expr, else $default:expr $(,)?) => {
        if $bf.check_bit($i) { $then } else { $default }
    };
}

#[macro_export]
macro_rules! unpack_field_flags {
    ($bf:expr, {$($i:literal => $name:ident: $then:expr $(, else $default:expr)?;)+}) => {
        $(
            let $name = $crate::option_on_bit!($bf, $i => $then $(, else $default)?);
        )*
    };
}

#[macro_export]
macro_rules! unpack_bool_flag {
    // Typical case: true iff the bit is set
    ($bf:expr, $i:literal => $name:ident) => {
        let $name = $bf.check_bit($i);
    };

    // Negated case: true iff the bit is not set
    ($bf:expr, $i:literal => !$name:ident) => {
        let $name = !$bf.check_bit($i);
    };
}

#[macro_export]
macro_rules! unpack_bool_flags {
    ($bf:expr, {$($i:literal => $tx:tt $($ty:ident)?;)+}) => {
        $(
            $crate::unpack_bool_flag!($bf, $i => $tx $($ty)?);
        )*
    };
}

#[macro_export]
macro_rules! impl_try_from_for_optional_from {
    ($target:ty, $prim:ty, $fromfn:ident, $v:vis $errtype:ident) => {
        #[derive(thiserror::Error, Debug)]
        #[error("invalid value {bad_value} for {}", stringify!($target))]
        $v struct $errtype {
            bad_value: $prim,
        }

        impl TryFrom<$prim> for $target {
            type Error = $errtype;

            fn try_from(v: $prim) -> Result<$target, $errtype> {
                <$target>::$fromfn(v).ok_or($errtype {
                    bad_value: v,
                })
            }
        }
    };
}

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
            bytes: stream.read_u8_buf(size)?,
        })
    }

    /// Reads `size: u32` and the `size - 4` bytes that follow, reading `size` bytes in total.
    fn try_parse_inclusive<T: ByteStreamLe>(stream: &mut T) -> Result<OpaqueBytes> {
        let size: usize = stream.read_u32_le()?.try_into()?;

        Ok(OpaqueBytes {
            bytes: stream.read_u8_buf(size.checked_sub(4).ok_or_else(|| {
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
    // println!("{}: {media_info:#?}", media_info_path.display());

    let end_tag_path: PathBuf = [dir_path, "end_tag.bin"].iter().collect();
    let end_tag =
        ModelEndTag::try_parse(&mut std::fs::File::open(&end_tag_path)?, NoteSdkType::SPen)?;
    // println!("{}: {end_tag:#?}", end_tag_path.display());

    let note_note_path: PathBuf = [dir_path, "note.note"].iter().collect();
    let note_note = NoteDoc::try_parse(&mut std::fs::File::open(&note_note_path)?)?;
    // println!("{}: {note_note:#?}", note_note_path.display());

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
