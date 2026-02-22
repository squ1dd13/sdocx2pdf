use crate::{OpaqueBytes, byte_stream::ByteStreamLe, page::Rect};
use color_eyre::Result;

#[derive(Debug)]
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

#[derive(Debug)]
pub struct CustomPageObject {
    object_type: u32,
    inner: OpaqueBytes,
}

impl CustomPageObject {
    pub fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<CustomPageObject> {
        let object_type = stream.read_u32_le()?;

        Ok(CustomPageObject {
            object_type,
            inner: OpaqueBytes::try_parse_exclusive(stream)?,
        })
    }
}
