use crate::{AppVersion, OpaqueBytes, byte_stream::ByteStreamLe};
use chrono::{DateTime, Utc};
use color_eyre::Result;
use std::{
    collections::HashMap,
    io::{Seek, SeekFrom},
};

#[derive(Debug)]
struct AuthorInfo {
    strings: [String; 3],
    image_id: u32,
}

impl AuthorInfo {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<AuthorInfo> {
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

#[derive(Debug)]
struct PenInfo {
    name: String,
    size: f32,
    colour_rgba: [u8; 4],
    is_curvable: bool,
    advanced_settings: String,
    is_eraser_enabled: bool,
    size_level: u32,
    particle_density: u32,
    ui_colour_hsv: [f32; 3],
}

impl PenInfo {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<PenInfo> {
        Ok(PenInfo {
            name: stream.read_short_u16_string()?,
            size: stream.read_f32_le()?,
            colour_rgba: stream.read_u32_le()?.to_le_bytes(),
            is_curvable: stream.read_u32_le()? != 0,
            advanced_settings: stream.read_short_u16_string()?,
            is_eraser_enabled: stream.read_u32_le()? != 0,
            size_level: stream.read_u32_le()?,
            particle_density: stream.read_u32_le()?,
            ui_colour_hsv: [
                stream.read_f32_le()?,
                stream.read_f32_le()?,
                stream.read_f32_le()?,
            ],
        })
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
    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<VoiceRecordingInfo> {
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
    title_text: OpaqueBytes,
    body_text: OpaqueBytes,
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
    sha256: [u8; 32],
}

impl NoteDoc {
    pub fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<NoteDoc> {
        let flexible_data_area_offset = {
            let start_offset = stream.stream_position()?;
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

        let title_text = OpaqueBytes::try_parse_exclusive(stream)?;
        let body_text = OpaqueBytes::try_parse_exclusive(stream)?;

        stream.seek(SeekFrom::Start(flexible_data_area_offset))?;

        let metadata = NoteDocMetadata {
            app_name: if field_check_flags & 1 != 0 {
                Some(stream.read_short_u16_string()?)
            } else {
                None
            },

            app_version: if field_check_flags & 2 != 0 {
                Some(AppVersion::try_parse(stream)?)
            } else {
                None
            },

            author_info: if field_check_flags & 4 != 0 {
                Some(AuthorInfo::try_parse(stream)?)
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
            Some(PenInfo::try_parse(stream)?)
        } else {
            None
        };

        let voice_data: Option<Vec<VoiceRecordingInfo>> = if field_check_flags & 0x2000 != 0 {
            let voice_data_count = stream.read_u32_le()?;

            Some(
                (0..voice_data_count)
                    .map(|_| VoiceRecordingInfo::try_parse(stream))
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
            let pen_info_end_offset = {
                let pen_info_start_offset = stream.stream_position()?;
                let pen_info_data_size: u64 = stream.read_u32_le()?.into();

                pen_info_start_offset + pen_info_data_size
            };

            let pen_info = PenInfo::try_parse(stream)?;

            stream.seek(SeekFrom::Start(pen_info_end_offset))?;

            Some(pen_info)
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

        let mut sha256 = [0_u8; 32];
        stream.read_exact(&mut sha256)?;

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
            sha256,
        })
    }
}
