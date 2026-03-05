use std::{collections::HashMap, io::Read};

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        ByteStreamLe, ExactSizedStream, ReadBitfieldError, ReadStringError, TryParse,
        UnfinishedParsingError,
    },
    page::Rect,
    read_size_and_map,
};
use color_eyre::Result;
use thiserror::Error;

#[derive(Debug)]
#[allow(dead_code)]
pub struct PdfDataItem {
    bind_id: u32,
    page_index: u32,
    pdf_rect: Rect,
}

impl PdfDataItem {
    pub fn try_parse<T: ByteStreamLe>(stream: &mut T, format_version: u32) -> Result<PdfDataItem> {
        Ok(PdfDataItem {
            bind_id: stream.read_u32_le()?,
            page_index: stream.read_u32_le()?,
            pdf_rect: if format_version < 2034 {
                Rect::try_parse_f64(stream)?
            } else {
                Rect {
                    left: stream.read_i32_le()?.into(),
                    top: stream.read_i32_le()?.into(),
                    right: stream.read_i32_le()?.into(),
                    bottom: stream.read_i32_le()?.into(),
                }
            },
        })
    }
}

#[derive(Debug)]
#[allow(dead_code)]
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

impl CanvasCacheEntry {
    pub fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<CanvasCacheEntry> {
        Ok(CanvasCacheEntry {
            file_id: stream.read_u32_le()?,
            width: stream.read_u32_le()?,
            height: stream.read_u32_le()?,
            is_dark_mode: stream.read_u8()? == 1,
            background_colour: stream.read_u32_le()?.to_le_bytes(),
            version: [
                stream.read_u32_le()?,
                stream.read_u32_le()?,
                stream.read_u32_le()?,
            ],
            cache_version: stream.read_u32_le()?,
            property: stream.read_u32_le()?,
            locale_list_id: stream.read_u32_le()?,
            system_font_path_hash: stream.read_u32_le()?,
        })
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum CustomPageObjectParseError {
    Io(#[from] std::io::Error),
    Flags(#[from] ReadBitfieldError),
    String(#[from] ReadStringError),
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
    attached_files: HashMap<String, u32>,
    custom_data: HashMap<String, String>,
    rect: Rect,
}

impl<R: Read> TryParse<R> for CustomPageObject {
    type ParseError = CustomPageObjectParseError;

    fn try_parse(
        stream: &mut R,
    ) -> std::result::Result<CustomPageObject, CustomPageObjectParseError> {
        let object_type: CustomObjectType = stream.read_u32_le()?.into();

        let mut stream = stream.take_exclusive_length_prefixed()?;

        if let not_zero @ 1.. = stream.read_u32_le()? {
            eprintln!("Warning: Unexpected non-zero value {not_zero} at start of custom object");
            // ... whatever. Keep going.
        }

        // Property flags and field flags should be zero.
        CheckedBitfield::try_parse(&mut stream)?
            .ensure_none_set_unchecked()
            .map_err(CustomPageObjectParseError::FoundProperty)?;

        CheckedBitfield::try_parse(&mut stream)?
            .ensure_none_set_unchecked()
            .map_err(CustomPageObjectParseError::FoundField)?;

        let uuid = stream.read_short_u8_string()?;

        let attached_files = read_size_and_map!(
            stream,
            u32,
            CustomPageObjectParseError::MapTooBig,
            (stream.read_long_u8_string()?, stream.read_u32_le()?)
        );

        let custom_data = read_size_and_map!(
            stream,
            u32,
            CustomPageObjectParseError::MapTooBig,
            (stream.read_long_u8_string()?, stream.read_long_u8_string()?)
        );

        let rect = Rect::try_parse_f64(&mut stream)?;

        stream.ensure_eof()?;

        Ok(CustomPageObject {
            object_type,
            uuid,
            attached_files,
            custom_data,
            rect,
        })
    }
}
