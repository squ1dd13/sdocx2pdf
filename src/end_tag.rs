use crate::{AppVersion, byte_stream::ByteStreamLe};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use std::io::Seek;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSdkType {
    /// `SAMSUNG S-Pen PAINTING SDK`; 0x1 in DLL
    SamsungSPenPainting,

    /// `S-Pen SDK`; 0x2 in DLL
    SPen,

    /// `S-Pen PAINTING SDK`; 0x3 in DLL
    SPenPainting,

    /// `SAMSUNG S-Pen SDK`; 0x4 in DLL
    SamsungSPen,
}

impl NoteSdkType {
    const fn ident(self) -> &'static str {
        match self {
            NoteSdkType::SamsungSPenPainting => "Document for SAMSUNG S-Pen PAINTING SDK",
            NoteSdkType::SPen => "Document for S-Pen SDK",
            NoteSdkType::SPenPainting => "Document for S-Pen PAINTING SDK",
            NoteSdkType::SamsungSPen => "Document for SAMSUNG S-Pen SDK",
        }
    }

    /// Checks whether the next `block_size` bytes in `stream` ends with an ident string
    /// that matches `self`.
    ///
    /// The returned `bool` is `true` if the ident is successfully read and matches `self`,
    /// and `false` if the read was successful but the ident doesn't match, or if `block_size`
    /// is smaller than the length of the expected ident. If the read is successful,
    /// the string constructed is returned as well.
    ///
    /// If `Ok` is returned, `stream` will be at the same position as it was when this method was
    /// called.
    fn verify_block_end<T: ByteStreamLe + Seek>(
        self,
        stream: &mut T,
        block_size: usize,
    ) -> Result<(bool, Option<String>)> {
        let ident = self.ident();

        let Some(ident_offset) = block_size.checked_sub(ident.len()) else {
            // The block cannot contain the ident, because it isn't big enough.
            return Ok((false, None));
        };

        // Seek to where the start of the ident should be.
        stream.seek_relative(ident_offset.try_into()?)?;

        let maybe_ident = stream.read_u8_string(ident.len())?;

        // Return to the start of the block.
        stream.seek_relative(-i64::try_from(block_size)?)?;

        Ok((maybe_ident == ident, Some(maybe_ident)))
    }
}

#[derive(Debug)]
struct EncryptionInfo {
    encryption_size: u32,
    encryption_salt: Vec<u8>,
    encryption_iv: Vec<u8>,
    encryption_key: Vec<u8>,
}

impl EncryptionInfo {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<EncryptionInfo> {
        Ok(EncryptionInfo {
            encryption_size: stream.read_u32_le()?,

            encryption_salt: {
                let salt_size: usize = stream.read_u32_le()?.try_into()?;
                stream.read_u8s(salt_size)?
            },

            encryption_iv: {
                let iv_size: usize = stream.read_u32_le()?.try_into()?;
                stream.read_u8s(iv_size)?
            },

            encryption_key: {
                let key_size: usize = stream.read_u32_le()?.try_into()?;
                stream.read_u8s(key_size)?
            },
        })
    }
}

#[derive(Debug)]
enum PagingType {
    /// Traditional pages.
    Paged,

    /// Appears pageless to the user. Implemented by putting everything on one large page.
    Pageless,

    Unknown(u16),
}

impl PagingType {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<PagingType> {
        Ok(match stream.read_u16_le()? {
            0 => PagingType::Paged,
            1 => PagingType::Pageless,
            x => {
                eprintln!("Found unknown paging type {x}. This shouldn't happen!");
                PagingType::Unknown(x)
            }
        })
    }
}

/// The structure in `end_tag.bin`.
#[derive(Debug)]
pub struct ModelEndTag {
    sdk_type: NoteSdkType,

    format_version: u32,
    note_id: String,
    last_modified_time: DateTime<Utc>,
    property_flags: u32,
    cover_image: String,
    note_width: u32,
    note_height: f32,
    app_name: String,
    app_version: AppVersion,
    min_format_version: u32,
    created_time: DateTime<Utc>,
    last_viewed_page_index: u32,
    page_model: PagingType,
    document_type: u16,
    owner_id: String,
    encryption_info: Option<EncryptionInfo>,
    display_created_time: DateTime<Utc>,
    display_modified_time: DateTime<Utc>,
    last_recognised_data_modified_time: DateTime<Utc>,
    fixed_font: String,
    fixed_text_direction: u32,
    fixed_background_theme: u32,
    server_check_point: i64,
    new_orientation: u32,
    min_unknown_version: u32,
    app_custom_data: String,
}

impl ModelEndTag {
    pub fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        sdk_type: NoteSdkType,
    ) -> Result<ModelEndTag> {
        let tag_size: usize = stream.read_u16_le()?.into();

        // Make sure the tag specifies the SDK type we are expecting.
        let (ident_matches, ident_found) = sdk_type.verify_block_end(stream, tag_size)?;

        let expected_ident = sdk_type.ident();

        if !ident_matches {
            return Err(ident_found.map_or_else(
                || eyre!("Not enough space for ident '{expected_ident}'"),
                |found| eyre!("Ident '{found}' does not match expected '{expected_ident}'"),
            ));
        }

        let format_version = stream.read_u32_le()?;

        let note_id = stream.read_short_u16_string()?;
        let last_modified_time = stream.read_timestamp()?;
        let property_flags = stream.read_u32_le()?;
        let cover_image = stream.read_short_u16_string()?;

        let note_width = stream.read_u32_le()?;
        let note_height = stream.read_f32_le()?;

        let app_name = stream.read_short_u16_string()?;
        let app_version = AppVersion::try_parse(stream)?;

        let min_format_version = stream.read_u32_le()?;

        let created_time = stream.read_timestamp()?;
        let last_viewed_page_index = stream.read_u32_le()?;

        let page_model = PagingType::try_parse(stream)?;
        let document_type = stream.read_u16_le()?;

        let owner_id = stream.read_short_u16_string()?;

        let n_to_skip: i64 = stream.read_u32_le()?.into();
        stream.seek_relative(n_to_skip)?;

        let encryption_data_size = stream.read_u32_le()?;

        let encryption_info = if encryption_data_size == 0 {
            None
        } else {
            Some(EncryptionInfo::try_parse(stream)?)
        };

        let display_created_time = stream.read_timestamp()?;
        let display_modified_time = stream.read_timestamp()?;
        let last_recognised_data_modified_time = stream.read_timestamp()?;

        let fixed_font = stream.read_short_u16_string()?;
        let fixed_text_direction = stream.read_u32_le()?;
        let fixed_background_theme = stream.read_u32_le()?;

        let server_check_point = stream.read_i64_le()?;

        let new_orientation = stream.read_u32_le()?;
        let min_unknown_version = stream.read_u32_le()?;

        let app_custom_data = stream.read_long_u16_string()?;

        // We know that the real ident and expected ident match, so to seek past the
        // real ident we can just skip the size of the expected ident.
        stream.seek_relative(expected_ident.len().try_into()?)?;

        Ok(ModelEndTag {
            sdk_type,
            format_version,
            note_id,
            last_modified_time,
            property_flags,
            cover_image,
            note_width,
            note_height,
            app_name,
            app_version,
            min_format_version,
            created_time,
            last_viewed_page_index,
            page_model,
            document_type,
            owner_id,
            encryption_info,
            display_created_time,
            display_modified_time,
            last_recognised_data_modified_time,
            fixed_font,
            fixed_text_direction,
            fixed_background_theme,
            server_check_point,
            new_orientation,
            min_unknown_version,
            app_custom_data,
        })
    }
}
