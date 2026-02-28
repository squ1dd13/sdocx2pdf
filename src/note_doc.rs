use crate::{
    AppVersion, OpaqueBytes,
    byte_stream::{
        ByteStreamLe, ExactSizedStream, ReadStringError, TakeInclusiveLengthPrefixedError,
        UnfinishedParsingError,
    },
    page::object::text::TextObject,
};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use sha2::Digest;
use std::{
    collections::HashMap,
    io::{self, Read, Seek, SeekFrom, Write},
};
use thiserror::Error;

#[derive(Debug)]
struct AuthorInfo {
    strings: [String; 3],
    image_id: u32,
}

impl AuthorInfo {
    fn try_parse(mut stream: impl ByteStreamLe) -> Result<AuthorInfo> {
        Ok(AuthorInfo {
            strings: [
                stream.read_short_u16_string()?,
                stream.read_short_u16_string()?,
                stream.read_short_u16_string()?,
            ],
            image_id: stream.read_u32_le()?,
        })
    }
}

#[derive(Error, Debug)]
enum PenInfoParseError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    BadSize(#[from] TakeInclusiveLengthPrefixedError),

    #[error("failed to read name")]
    ReadName(#[source] ReadStringError),

    #[error("failed to read advanced settings")]
    ReadAdvancedSettings(#[source] ReadStringError),

    #[error(transparent)]
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
struct PenInfo {
    name: String,
    size: f32,
    colour: [u8; 4],

    size_level: u32,

    advanced_settings: String,

    is_curvable: bool,
    is_eraser_enabled: bool,
    is_fixed_width: Option<bool>,

    particle_density: u32,
    particle_size: Option<f32>,

    ui_colour_hsv: [f32; 3],
    ui_colour_info: u32,
}

impl PenInfo {
    fn try_parse_simple(mut stream: impl ByteStreamLe) -> Result<PenInfo, PenInfoParseError> {
        Ok(PenInfo {
            name: stream
                .read_short_u16_string()
                .map_err(PenInfoParseError::ReadName)?,

            size: stream.read_f32_le()?,
            colour: stream.read_4_bytes()?,

            is_curvable: stream.read_u32_le()? != 0,

            advanced_settings: stream
                .read_short_u16_string()
                .map_err(PenInfoParseError::ReadAdvancedSettings)?,

            is_eraser_enabled: stream.read_u32_le()? != 0,
            size_level: stream.read_u32_le()?,
            particle_density: stream.read_u32_le()?,

            ui_colour_hsv: [
                stream.read_f32_le()?,
                stream.read_f32_le()?,
                stream.read_f32_le()?,
            ],
            ui_colour_info: stream.read_u32_le()?,

            is_fixed_width: None,
            particle_size: None,
        })
    }

    fn try_parse_full(mut stream: impl ByteStreamLe) -> Result<PenInfo, PenInfoParseError> {
        let mut stream = stream.take_inclusive_length_prefixed()?;

        let pen_info = PenInfo {
            name: stream
                .read_short_u16_string()
                .map_err(PenInfoParseError::ReadName)?,

            size: stream.read_f32_le()?,
            colour: stream.read_4_bytes()?,

            is_curvable: stream.read_u32_le()? != 0,

            advanced_settings: stream
                .read_short_u16_string()
                .map_err(PenInfoParseError::ReadAdvancedSettings)?,

            is_eraser_enabled: stream.read_u32_le()? != 0,

            size_level: stream.read_u32_le()?,

            particle_density: stream.read_u32_le()?,
            particle_size: Some(stream.read_f32_le()?),

            is_fixed_width: Some(stream.read_u32_le()? != 0),

            ui_colour_hsv: [
                stream.read_f32_le()?,
                stream.read_f32_le()?,
                stream.read_f32_le()?,
            ],
            ui_colour_info: stream.read_u32_le()?,
        };

        stream.ensure_eof()?;

        Ok(pen_info)
    }
}

#[derive(Debug)]
struct VoiceRecordingInfo {
    id: u32,
    name: String,
    duration_str: String,
    first_big_number: DateTime<Utc>,
    somethings: Vec<(u32, DateTime<Utc>)>,
    precise_duration: Option<chrono::Duration>,
}

impl VoiceRecordingInfo {
    fn try_parse(mut stream: (impl ByteStreamLe + Seek)) -> Result<VoiceRecordingInfo> {
        let data_end_offset = {
            let data_size: u64 = stream.read_u32_le()?.into();
            let data_start_offset = stream.stream_position()?;

            data_start_offset + data_size
        };

        let id = stream.read_u32_le()?;
        let name = stream.read_short_u16_string()?;
        let duration_str = stream.read_short_u16_string()?;
        let first_big_number = stream.read_timestamp()?;

        let number_of_somethings = stream.read_u32_le()?;

        let mut somethings = Vec::with_capacity(number_of_somethings.try_into()?);

        for _ in 0..number_of_somethings {
            somethings.push((stream.read_u32_le()?, stream.read_timestamp()?));
        }

        let precise_duration = if stream.stream_position()? < data_end_offset {
            Some(chrono::Duration::milliseconds(stream.read_i64_le()?))
        } else {
            None
        };

        Ok(VoiceRecordingInfo {
            id,
            name,
            duration_str,
            first_big_number,
            somethings,
            precise_duration,
        })
    }
}

#[derive(Debug)]
struct NoteDocMetadata {
    app_name: Option<String>,
    app_version: Option<AppVersion>,
    author_info: Option<AuthorInfo>,
    latitude_longitude: Option<(f64, f64)>,
}

/// `libSpen_worddoc.dll`
#[derive(Debug)]
pub struct NoteDoc {
    property_flags: u32,
    field_check_flags: u32,
    format_version: u32,
    id: String,
    file_revision: u32,
    created_time: DateTime<Utc>,
    modified_time: DateTime<Utc>,
    width: u32,
    height: u32,
    page_horizontal_padding: u32,
    page_vertical_padding: u32,
    min_format_version: u32,
    title_text: TextObject,
    body_text: TextObject,
    metadata: NoteDocMetadata,
    template_uri: Option<String>,
    last_edited_page_index: Option<u32>,
    last_edited_page_image_id: Option<i32>,
    last_edited_page_time: Option<DateTime<Utc>>,
    managed_strings: Option<HashMap<u32, String>>,
    body_text_font_size_delta: Option<i32>,
    compatible_last_pen_info: Option<PenInfo>,
    voice_data: Option<Vec<VoiceRecordingInfo>>,
    attached_files: Option<HashMap<String, u32>>,
    last_pen_info: Option<PenInfo>,
    server_check_point: Option<i64>,
    fixed_font: Option<String>,
    fixed_text_direction: Option<u32>,
    fixed_background_theme: Option<u32>,
    text_summarisation: Option<String>,
    stroke_group_size: Option<u32>,
    app_custom_data: Option<String>,
}

impl NoteDoc {
    pub fn try_parse(mut stream: (impl ByteStreamLe + Seek)) -> Result<NoteDoc> {
        let start_offset = stream.stream_position()?;

        let flexible_data_area_offset = {
            let flexible_data_area_jump: u64 = stream.read_u32_le()?.into();
            start_offset + flexible_data_area_jump
        };

        let property_flags = stream.read_variable_length_bitfield()?;
        let field_check_flags = stream.read_variable_length_bitfield()?;

        let format_version = stream.read_u32_le()?;
        let id = stream.read_short_u16_string()?;
        let file_revision = stream.read_u32_le()?;
        let created_time = stream.read_timestamp()?;
        let modified_time = stream.read_timestamp()?;
        let width = stream.read_u32_le()?;
        let height = stream.read_u32_le()?;
        let page_horizontal_padding = stream.read_u32_le()?;
        let page_vertical_padding = stream.read_u32_le()?;
        let min_format_version = stream.read_u32_le()?;

        let title_text = {
            let mut stream = stream.take_exclusive_length_prefixed()?;

            let text = TextObject::try_parse_standalone(&mut stream)?;
            stream.ensure_eof()?;

            text
        };

        let body_text = {
            let mut stream = stream.take_exclusive_length_prefixed()?;

            let text = TextObject::try_parse_standalone(&mut stream)?;
            stream.ensure_eof()?;

            text
        };

        stream.seek(SeekFrom::Start(flexible_data_area_offset))?;

        let metadata = NoteDocMetadata {
            app_name: if field_check_flags & 1 != 0 {
                Some(stream.read_short_u16_string()?)
            } else {
                None
            },

            app_version: if field_check_flags & 2 != 0 {
                Some(AppVersion::try_parse(&mut stream)?)
            } else {
                None
            },

            author_info: if field_check_flags & 4 != 0 {
                Some(AuthorInfo::try_parse(&mut stream)?)
            } else {
                None
            },

            latitude_longitude: if field_check_flags & 8 != 0 {
                Some((stream.read_f64_le()?, stream.read_f64_le()?))
            } else {
                None
            },
        };

        let template_uri = if field_check_flags & 0x40 != 0 {
            Some(stream.read_short_u16_string()?)
        } else {
            None
        };

        let last_edited_page_index = if field_check_flags & 0x80 != 0 {
            Some(stream.read_u32_le()?)
        } else {
            None
        };

        let (last_edited_page_image_id, last_edited_page_time) = if field_check_flags & 0x200 != 0 {
            (Some(stream.read_i32_le()?), Some(stream.read_timestamp()?))
        } else {
            (None, None)
        };

        let managed_strings: Option<HashMap<u32, String>> = if field_check_flags & 0x400 != 0 {
            let string_manager_size = stream.read_u32_le()?;

            if string_manager_size != 0 {
                let string_count = stream.read_u16_le()?;

                let mut ids_and_strings = HashMap::with_capacity(string_count.into());

                for _ in 0..string_count {
                    let string_id = stream.read_u32_le()?;
                    let string = stream.read_short_u16_string()?;

                    ids_and_strings.insert(string_id, string);
                }

                Some(ids_and_strings)
            } else {
                None
            }
        } else {
            None
        };

        let body_text_font_size_delta = if field_check_flags & 0x800 != 0 {
            Some(stream.read_i32_le()?)
        } else {
            None
        };

        let compatible_last_pen_info = if field_check_flags & 0x1000 != 0 {
            Some(PenInfo::try_parse_simple(&mut stream)?)
        } else {
            None
        };

        let voice_data: Option<Vec<VoiceRecordingInfo>> = if field_check_flags & 0x2000 != 0 {
            let voice_data_count = stream.read_u32_le()?;

            Some(
                (0..voice_data_count)
                    .map(|_| VoiceRecordingInfo::try_parse(&mut stream))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            None
        };

        let attached_files: Option<HashMap<String, u32>> = if field_check_flags & 0x4000 != 0 {
            let attached_files_count = stream.read_u16_le()?;

            let mut map = HashMap::with_capacity(attached_files_count.into());

            for _ in 0..attached_files_count {
                map.insert(stream.read_short_u16_string()?, stream.read_u32_le()?);
            }

            Some(map)
        } else {
            None
        };

        let last_pen_info = if field_check_flags & 0x8000 != 0 {
            Some(PenInfo::try_parse_full(&mut stream)?)
        } else {
            None
        };

        let server_check_point = if field_check_flags & 0x10000 != 0 {
            Some(stream.read_i64_le()?)
        } else {
            None
        };

        let fixed_font = if field_check_flags & 0x20000 != 0 {
            Some(stream.read_short_u16_string()?)
        } else {
            None
        };

        let fixed_text_direction = if field_check_flags & 0x40000 != 0 {
            Some(stream.read_u32_le()?)
        } else {
            None
        };

        let fixed_background_theme = if field_check_flags & 0x80000 != 0 {
            Some(stream.read_u32_le()?)
        } else {
            None
        };

        let text_summarisation = if field_check_flags & 0x100000 != 0 {
            Some(stream.read_short_u16_string()?)
        } else {
            None
        };

        let stroke_group_size = if field_check_flags & 0x200000 != 0 {
            Some(stream.read_u32_le()?)
        } else {
            None
        };

        let app_custom_data = if field_check_flags & 0x400000 != 0 {
            Some(stream.read_long_u16_string()?)
        } else {
            None
        };

        let calculated_hash = {
            // Calculate the number of bytes we have read in total.
            let data_size = {
                let data_end_pos = stream.stream_position()?;
                data_end_pos - start_offset
            };

            stream.seek(SeekFrom::Start(start_offset))?;

            let mut hasher = sha2::Sha256::new();

            // Copy exactly `data_size` bytes from the stream into the hasher. Note that this
            // brings the position in the stream back to where it was before the `seek` above.
            io::copy(&mut (&mut stream).take(data_size), &mut hasher);

            hasher.finalize()
        };

        // Now read the corresponding hash from the stream.
        let hash_in_stream = {
            let mut v = [0_u8; 32];
            stream.read_exact(&mut v);
            v
        };

        if calculated_hash[..] != hash_in_stream {
            return Err(eyre!("hash mismatch"));
        }

        Ok(NoteDoc {
            property_flags,
            field_check_flags,
            format_version,
            id,
            file_revision,
            created_time,
            modified_time,
            width,
            height,
            page_horizontal_padding,
            page_vertical_padding,
            min_format_version,
            title_text,
            body_text,
            metadata,
            template_uri,
            last_edited_page_index,
            last_edited_page_image_id,
            last_edited_page_time,
            managed_strings,
            body_text_font_size_delta,
            compatible_last_pen_info,
            voice_data,
            attached_files,
            last_pen_info,
            server_check_point,
            fixed_font,
            fixed_text_direction,
            fixed_background_theme,
            text_summarisation,
            stroke_group_size,
            app_custom_data,
        })
    }
}
