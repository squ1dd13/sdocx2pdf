use std::{collections::HashMap, io::Read, rc::Rc};

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BoundedStream, ByteStreamLe, ReadBitfieldError, ReadStringError, TryParse,
        UnfinishedParsingError,
    },
    context::TryParseWithContext,
    media_info::{BoundFile, FileRegistry, NoSuchRegisteredFileError},
    page::Rect,
    read_size_and_map,
};
use thiserror::Error;

pub struct PdfDataItemParseCtx<'fr> {
    pub file_registry: &'fr FileRegistry,
    pub format_version: u32,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum PdfDataItemParseError {
    Io(#[from] std::io::Error),
    NoSuchFile(#[from] NoSuchRegisteredFileError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct PdfPage {
    pdf: Rc<BoundFile>,
    page_index: u32,
    rect: Rect,
}

impl<R: Read> TryParseWithContext<R, PdfDataItemParseCtx<'_>> for PdfPage {
    type ParseError = PdfDataItemParseError;

    fn try_parse_with_ctx(
        reader: &mut R,
        ctx: &PdfDataItemParseCtx<'_>,
    ) -> Result<PdfPage, PdfDataItemParseError> {
        Ok(PdfPage {
            pdf: ctx.file_registry.try_get(reader.read_u32_le()?)?,
            page_index: reader.read_u32_le()?,
            rect: if ctx.format_version < 2034 {
                Rect::try_parse_f64(reader)?
            } else {
                Rect::try_parse_i32(reader)?
            },
        })
    }
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct CanvasCacheEntry {
    file_id: u32,
    width: u32,
    height: u32,
    is_dark_mode: bool,
    background_colour: [u8; 4],
    version: [u32; 3],
    cache_version: u32,
    property: u32,
    locale_list_id: u32,
    system_font_path_hash: u32,
}

impl<R: Read> TryParse<R> for CanvasCacheEntry {
    type ParseError = std::io::Error;

    fn try_parse(reader: &mut R) -> std::io::Result<CanvasCacheEntry> {
        Ok(CanvasCacheEntry {
            file_id: reader.read_u32_le()?,
            width: reader.read_u32_le()?,
            height: reader.read_u32_le()?,
            is_dark_mode: reader.read_u8()? == 1,
            background_colour: reader.read_u32_le()?.to_le_bytes(),
            version: [
                reader.read_u32_le()?,
                reader.read_u32_le()?,
                reader.read_u32_le()?,
            ],
            cache_version: reader.read_u32_le()?,
            property: reader.read_u32_le()?,
            locale_list_id: reader.read_u32_le()?,
            system_font_path_hash: reader.read_u32_le()?,
        })
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum CustomPageObjectParseError {
    Io(#[from] std::io::Error),
    Flags(#[from] ReadBitfieldError),
    String(#[from] ReadStringError),
    BadAttachedFile(#[from] NoSuchRegisteredFileError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("one or more property bits were set, but none expected")]
    FoundProperty(#[source] UnhandledBitsError),

    #[error("one or more field bits were set, but none expected")]
    FoundField(#[source] UnhandledBitsError),

    #[error("map entry count {0} is too big for `usize`")]
    MapTooBig(u32),
}

#[derive(Debug)]
pub enum CustomObjectType {
    /// `TYPE_STICKY_NOTE = 1`
    ///
    /// These have the sticky note's sdocx file's ID in `attached_files["co_attach_file"]`, the
    /// background colour (as a string representation of a signed integer) in
    /// `custom_data["skn_bg_color"]`, and a rectangle in `custom_data["skn_collapse_rect"]`
    /// written as `"left,top,right,bottom"`, where each component is a decimal value.
    StickyNote,

    /// Anything else.
    ///
    /// Future custom objects should, in theory, use the format we already parse here. In that
    /// case, we can parse them without knowing what the type ID represents.
    Unknown(#[expect(dead_code)] u32),
}

impl From<u32> for CustomObjectType {
    fn from(value: u32) -> Self {
        match value {
            1 => CustomObjectType::StickyNote,
            u => CustomObjectType::Unknown(u),
        }
    }
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct CustomPageObject {
    object_type: CustomObjectType,

    uuid: String,
    attached_files: HashMap<String, Rc<BoundFile>>,
    custom_data: HashMap<String, String>,
    rect: Rect,
}

impl<R: Read> TryParseWithContext<R, FileRegistry> for CustomPageObject {
    type ParseError = CustomPageObjectParseError;

    fn try_parse_with_ctx(
        reader: &mut R,
        file_registry: &FileRegistry,
    ) -> Result<CustomPageObject, CustomPageObjectParseError> {
        let object_type: CustomObjectType = reader.read_u32_le()?.into();

        let mut reader = reader.take_exclusive_length_prefixed()?;

        if let not_zero @ 1.. = reader.read_u32_le()? {
            eprintln!("Warning: Unexpected non-zero value {not_zero} at start of custom object");
            // ... whatever. Keep going.
        }

        // Property flags and field flags should be zero.
        CheckedBitfield::try_parse(&mut reader)?
            .ensure_none_set_unchecked()
            .map_err(CustomPageObjectParseError::FoundProperty)?;

        CheckedBitfield::try_parse(&mut reader)?
            .ensure_none_set_unchecked()
            .map_err(CustomPageObjectParseError::FoundField)?;

        let uuid = reader.read_short_u8_string()?;

        let attached_files = read_size_and_map!(
            reader,
            u32,
            CustomPageObjectParseError::MapTooBig,
            (
                reader.read_long_u8_string()?,
                file_registry.try_get(reader.read_u32_le()?)?
            )
        );

        let custom_data = read_size_and_map!(
            reader,
            u32,
            CustomPageObjectParseError::MapTooBig,
            (reader.read_long_u8_string()?, reader.read_long_u8_string()?)
        );

        let rect = Rect::try_parse_f64(&mut reader)?;

        reader.ensure_eof()?;

        Ok(CustomPageObject {
            object_type,
            uuid,
            attached_files,
            custom_data,
            rect,
        })
    }
}
