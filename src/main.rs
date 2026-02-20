#![allow(unused)]
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use color_eyre::eyre::{OptionExt, eyre};
use std::{
    collections::HashMap,
    io::{Seek, SeekFrom},
};

fn read_u8_buf(stream: &mut impl ReadBytesExt, n: usize) -> color_eyre::Result<Vec<u8>> {
    let mut bytes = vec![0u8; n];
    stream.read_exact(&mut bytes)?;

    Ok(bytes)
}

fn read_u8_string(stream: &mut impl ReadBytesExt, n_chars: usize) -> color_eyre::Result<String> {
    String::from_utf8(read_u8_buf(stream, n_chars)?).map_err(From::from)
}

fn read_u16_string(stream: &mut impl ReadBytesExt, n_chars: usize) -> color_eyre::Result<String> {
    let bytes = read_u8_buf(stream, 2 * n_chars)?;

    char::decode_utf16((0..n_chars).map(|i| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])))
        .collect::<color_eyre::Result<String, _>>()
        .map_err(From::from)
}

fn read_short_u16_string(stream: &mut impl ReadBytesExt) -> color_eyre::Result<String> {
    let n_chars: usize = stream.read_u16::<LittleEndian>()?.into();
    read_u16_string(stream, n_chars)
}

fn read_long_u16_string(stream: &mut impl ReadBytesExt) -> color_eyre::Result<String> {
    let n_chars: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
    read_u16_string(stream, n_chars)
}

fn read_timestamp(stream: &mut impl ReadBytesExt) -> color_eyre::Result<DateTime<Utc>> {
    DateTime::from_timestamp_micros(stream.read_i64::<LittleEndian>()?)
        .ok_or_eyre("invalid timestamp")
}

struct TextObject {
    bytes: Vec<u8>,
}

impl TextObject {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<TextObject> {
        let size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;

        Ok(TextObject {
            bytes: read_u8_buf(stream, size)?,
        })
    }
}

impl std::fmt::Debug for TextObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextObject {{ ({} bytes) }}", self.bytes.len())
    }
}

#[derive(Debug)]
struct AppVersion {
    major: u32,
    minor: u32,
    patch_name: String,
}

impl AppVersion {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<AppVersion> {
        Ok(AppVersion {
            major: stream.read_u32::<LittleEndian>()?,
            minor: stream.read_u32::<LittleEndian>()?,
            patch_name: read_short_u16_string(stream)?,
        })
    }
}

#[derive(Debug)]
struct AuthorInfo {
    strings: [String; 3],
    image_id: u32,
}

impl AuthorInfo {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<AuthorInfo> {
        Ok(AuthorInfo {
            strings: [
                read_short_u16_string(stream)?,
                read_short_u16_string(stream)?,
                read_short_u16_string(stream)?,
            ],
            image_id: stream.read_u32::<LittleEndian>()?,
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
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<PenInfo> {
        Ok(PenInfo {
            name: read_short_u16_string(stream)?,
            size: stream.read_f32::<LittleEndian>()?,
            colour_rgba: {
                let mut rgba = [0u8; 4];
                stream.read_exact(&mut rgba);
                rgba
            },
            is_curvable: stream.read_u32::<LittleEndian>()? != 0,
            advanced_settings: read_short_u16_string(stream)?,
            is_eraser_enabled: stream.read_u32::<LittleEndian>()? != 0,
            size_level: stream.read_u32::<LittleEndian>()?,
            particle_density: stream.read_u32::<LittleEndian>()?,
            ui_colour_hsv: [
                stream.read_f32::<LittleEndian>()?,
                stream.read_f32::<LittleEndian>()?,
                stream.read_f32::<LittleEndian>()?,
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
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<VoiceRecordingInfo> {
        let data_end_offset = {
            let data_size: u64 = stream.read_u32::<LittleEndian>()?.into();
            let data_start_offset = stream.stream_position()?;

            data_start_offset + data_size
        };

        let id = stream.read_u32::<LittleEndian>()?;
        let name = read_short_u16_string(stream)?;
        let duration_str = read_short_u16_string(stream)?;
        let first_big_number = read_timestamp(stream)?;

        let number_of_somethings = stream.read_u32::<LittleEndian>()?;

        let mut somethings = Vec::with_capacity(number_of_somethings.try_into()?);

        for _ in 0..number_of_somethings {
            somethings.push((stream.read_u32::<LittleEndian>()?, read_timestamp(stream)?));
        }

        let precise_duration = if stream.stream_position()? < data_end_offset {
            Some(chrono::Duration::milliseconds(
                stream.read_i64::<LittleEndian>()?,
            ))
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
struct NoteDoc {
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
    sha256: [u8; 32],
}

impl NoteDoc {
    fn read_variable_length_bitfield<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<u32> {
        let n_bytes = stream.read_u8()?;

        Ok(match n_bytes {
            0 => 0,
            1 => stream.read_u8()?.into(),
            2 => stream.read_u16::<LittleEndian>()?.into(),
            3 => stream.read_u24::<LittleEndian>()?,
            4 => stream.read_u32::<LittleEndian>()?,
            5.. => {
                return Err(eyre!(
                    "variable length bitfield cannot be more than 4 bytes (found {n_bytes})"
                ));
            }
        })
    }

    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<NoteDoc> {
        let flexible_data_area_offset = {
            let start_offset = stream.stream_position()?;
            let flexible_data_area_jump: u64 = stream.read_u32::<LittleEndian>()?.into();

            start_offset + flexible_data_area_jump
        };

        let property_flags = Self::read_variable_length_bitfield(stream)?;
        let field_check_flags = Self::read_variable_length_bitfield(stream)?;

        let format_version = stream.read_u32::<LittleEndian>()?;
        let id = read_short_u16_string(stream)?;
        let file_revision = stream.read_u32::<LittleEndian>()?;
        let created_time = read_timestamp(stream)?;
        let modified_time = read_timestamp(stream)?;
        let width = stream.read_u32::<LittleEndian>()?;
        let height = stream.read_u32::<LittleEndian>()?;
        let page_horizontal_padding = stream.read_u32::<LittleEndian>()?;
        let page_vertical_padding = stream.read_u32::<LittleEndian>()?;
        let min_format_version = stream.read_u32::<LittleEndian>()?;

        let title_text = TextObject::try_parse(stream)?;
        let body_text = TextObject::try_parse(stream)?;

        stream.seek(SeekFrom::Start(flexible_data_area_offset))?;

        let metadata = NoteDocMetadata {
            app_name: if field_check_flags & 1 != 0 {
                Some(read_short_u16_string(stream)?)
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
                Some((
                    stream.read_f64::<LittleEndian>()?,
                    stream.read_f64::<LittleEndian>()?,
                ))
            } else {
                None
            },
        };

        let template_uri = if field_check_flags & 0x40 != 0 {
            Some(read_short_u16_string(stream)?)
        } else {
            None
        };

        let last_edited_page_index = if field_check_flags & 0x80 != 0 {
            Some(stream.read_u32::<LittleEndian>()?)
        } else {
            None
        };

        let (last_edited_page_image_id, last_edited_page_time) = if field_check_flags & 0x200 != 0 {
            (
                Some(stream.read_i32::<LittleEndian>()?),
                Some(read_timestamp(stream)?),
            )
        } else {
            (None, None)
        };

        let managed_strings: Option<HashMap<u32, String>> = if field_check_flags & 0x400 != 0 {
            let string_manager_size = stream.read_u32::<LittleEndian>()?;

            if string_manager_size != 0 {
                let string_count = stream.read_u16::<LittleEndian>()?;

                let mut ids_and_strings = HashMap::with_capacity(string_count.into());

                for _ in 0..string_count {
                    let id = stream.read_u32::<LittleEndian>()?;
                    let string = read_short_u16_string(stream)?;

                    ids_and_strings.insert(id, string);
                }

                Some(ids_and_strings)
            } else {
                None
            }
        } else {
            None
        };

        let body_text_font_size_delta = if field_check_flags & 0x800 != 0 {
            Some(stream.read_i32::<LittleEndian>()?)
        } else {
            None
        };

        let compatible_last_pen_info = if field_check_flags & 0x1000 != 0 {
            Some(PenInfo::try_parse(stream)?)
        } else {
            None
        };

        let voice_data: Option<Vec<VoiceRecordingInfo>> = if field_check_flags & 0x2000 != 0 {
            let voice_data_count = stream.read_u32::<LittleEndian>()?;

            Some(
                (0..voice_data_count)
                    .map(|_| VoiceRecordingInfo::try_parse(stream))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            None
        };

        let attached_files: Option<HashMap<String, u32>> = if field_check_flags & 0x4000 != 0 {
            let attached_files_count = stream.read_u16::<LittleEndian>()?;

            let mut map = HashMap::with_capacity(attached_files_count.into());

            for _ in 0..attached_files_count {
                map.insert(
                    read_short_u16_string(stream)?,
                    stream.read_u32::<LittleEndian>()?,
                );
            }

            Some(map)
        } else {
            None
        };

        let last_pen_info = if field_check_flags & 0x8000 != 0 {
            let pen_info_end_offset = {
                let pen_info_start_offset = stream.stream_position()?;
                let pen_info_data_size: u64 = stream.read_u32::<LittleEndian>()?.into();

                pen_info_start_offset + pen_info_data_size
            };

            let pen_info = PenInfo::try_parse(stream)?;

            stream.seek(SeekFrom::Start(pen_info_end_offset))?;

            Some(pen_info)
        } else {
            None
        };

        let server_check_point = if field_check_flags & 0x10000 != 0 {
            Some(stream.read_i64::<LittleEndian>()?)
        } else {
            None
        };

        let fixed_font = if field_check_flags & 0x20000 != 0 {
            Some(read_short_u16_string(stream)?)
        } else {
            None
        };

        let fixed_text_direction = if field_check_flags & 0x40000 != 0 {
            Some(stream.read_u32::<LittleEndian>()?)
        } else {
            None
        };

        let fixed_background_theme = if field_check_flags & 0x80000 != 0 {
            Some(stream.read_u32::<LittleEndian>()?)
        } else {
            None
        };

        let text_summarisation = if field_check_flags & 0x100000 != 0 {
            Some(read_short_u16_string(stream)?)
        } else {
            None
        };

        let stroke_group_size = if field_check_flags & 0x200000 != 0 {
            Some(stream.read_u32::<LittleEndian>()?)
        } else {
            None
        };

        let app_custom_data = if field_check_flags & 0x400000 != 0 {
            Some(read_long_u16_string(stream)?)
        } else {
            None
        };

        let mut sha256 = [0u8; 32];
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteSdkType {
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
    fn ident(self) -> &'static str {
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
    fn verify_block_end<T: ReadBytesExt + Seek>(
        self,
        stream: &mut T,
        block_size: usize,
    ) -> color_eyre::Result<(bool, Option<String>)> {
        let ident = self.ident();

        let Some(ident_offset) = block_size.checked_sub(ident.len()) else {
            // The block cannot contain the ident, because it isn't big enough.
            return Ok((false, None));
        };

        // Seek to where the start of the ident should be.
        stream.seek_relative(ident_offset.try_into()?)?;

        let maybe_ident = read_u8_string(stream, ident.len())?;

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
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<EncryptionInfo> {
        Ok(EncryptionInfo {
            encryption_size: stream.read_u32::<LittleEndian>()?,

            encryption_salt: {
                let salt_size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
                read_u8_buf(stream, salt_size)?
            },

            encryption_iv: {
                let iv_size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
                read_u8_buf(stream, iv_size)?
            },

            encryption_key: {
                let key_size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
                read_u8_buf(stream, key_size)?
            },
        })
    }
}

/// The structure in `end_tag.bin`.
#[derive(Debug)]
struct ModelEndTag {
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
    page_model: u16,
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
    fn try_parse<T: ReadBytesExt + Seek>(
        stream: &mut T,
        sdk_type: NoteSdkType,
    ) -> color_eyre::Result<ModelEndTag> {
        let tag_size: usize = stream.read_u16::<LittleEndian>()?.into();

        // Make sure the tag specifies the SDK type we are expecting.
        let (ident_matches, ident_found) = sdk_type.verify_block_end(stream, tag_size)?;

        let expected_ident = sdk_type.ident();

        if !ident_matches {
            return Err(match ident_found {
                Some(found) => eyre!("ident '{found}' does not match expected '{expected_ident}'"),
                None => eyre!("not enough space for ident '{expected_ident}'"),
            });
        }

        let format_version = stream.read_u32::<LittleEndian>()?;

        let note_id = read_short_u16_string(stream)?;
        let last_modified_time = read_timestamp(stream)?;
        let property_flags = stream.read_u32::<LittleEndian>()?;
        let cover_image = read_short_u16_string(stream)?;

        let note_width = stream.read_u32::<LittleEndian>()?;
        let note_height = stream.read_f32::<LittleEndian>()?;

        let app_name = read_short_u16_string(stream)?;
        let app_version = AppVersion::try_parse(stream)?;

        let min_format_version = stream.read_u32::<LittleEndian>()?;

        let created_time = read_timestamp(stream)?;
        let last_viewed_page_index = stream.read_u32::<LittleEndian>()?;

        let page_model = stream.read_u16::<LittleEndian>()?;
        let document_type = stream.read_u16::<LittleEndian>()?;

        let owner_id = read_short_u16_string(stream)?;

        let n_to_skip: i64 = stream.read_u32::<LittleEndian>()?.into();
        stream.seek_relative(n_to_skip)?;

        let encryption_data_size = stream.read_u32::<LittleEndian>()?;

        let encryption_info = if encryption_data_size == 0 {
            None
        } else {
            Some(EncryptionInfo::try_parse(stream)?)
        };

        let display_created_time = read_timestamp(stream)?;
        let display_modified_time = read_timestamp(stream)?;
        let last_recognised_data_modified_time = read_timestamp(stream)?;

        let fixed_font = read_short_u16_string(stream)?;
        let fixed_text_direction = stream.read_u32::<LittleEndian>()?;
        let fixed_background_theme = stream.read_u32::<LittleEndian>()?;

        let server_check_point = stream.read_i64::<LittleEndian>()?;

        let new_orientation = stream.read_u32::<LittleEndian>()?;
        let min_unknown_version = stream.read_u32::<LittleEndian>()?;

        let app_custom_data = read_long_u16_string(stream)?;

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

#[derive(Debug)]
struct BoundFile {
    bind_id: u32,
    name: String,
    hash: String,
    ref_count: u16,
    ref_count_modified_time: DateTime<Utc>,
    is_attached: bool,
}

#[derive(Debug)]
struct MediaInfo {
    format_version: u32,
    bound_files: Vec<BoundFile>,
}

impl MediaInfo {
    /// Based on `SPen::MediaFileManagerNew::Load` in `libSpen_document.dll`.
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<MediaInfo> {
        Ok(MediaInfo {
            format_version: stream.read_u32::<LittleEndian>()?,
            bound_files: {
                let bound_file_count = stream.read_u16::<LittleEndian>()?;
                let mut bound_files = Vec::with_capacity(bound_file_count.into());

                for _ in 0..bound_file_count {
                    let data_size = stream.read_u32::<LittleEndian>()?;
                    let stream_pos_pre = stream.stream_position()?;
                    let expected_data_end = stream_pos_pre + data_size as u64;

                    let id = stream.read_u32::<LittleEndian>()?;
                    let filename = read_short_u16_string(stream)?;

                    let file_hash = read_u8_string(stream, 64)?;

                    let ref_count = stream.read_u16::<LittleEndian>()?;
                    let ref_count_modified_time = read_timestamp(stream)?;

                    let is_file_attached = stream.read_u8()? != 0;

                    let stream_pos_post = stream.stream_position()?;

                    if stream_pos_post != expected_data_end {
                        let actual_size = stream_pos_post - stream_pos_pre;

                        eprintln!(
                            "mismatch: declared size is {data_size}, but actual size is {actual_size}"
                        );

                        stream.seek(SeekFrom::Start(expected_data_end))?;
                    }

                    bound_files.push(BoundFile {
                        bind_id: id,
                        name: filename,
                        hash: file_hash,
                        ref_count,
                        ref_count_modified_time,
                        is_attached: is_file_attached,
                    })
                }

                bound_files
            },
        })
    }
}

fn demo_media_info() -> color_eyre::Result<()> {
    let media_info_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/empty encrypted_260219_125722/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Typed, formatted text with summary and voice memo_260220_003622/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features_260220_005438/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features plus dupes_260220_010554/media/mediaInfo.dat",
    ];

    for path in media_info_paths {
        let mut media_info = std::fs::File::open(path)?;

        let info = MediaInfo::try_parse(&mut media_info)?;

        println!("{path}: {info:#?}");
    }

    Ok(())
}

fn demo_end_tag() -> color_eyre::Result<()> {
    let end_tag_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/empty encrypted_260219_125722/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/Typed, formatted text with summary and voice memo_260220_003622/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features_260220_005438/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features plus dupes_260220_010554/end_tag.bin",
    ];

    for path in end_tag_paths {
        let mut end_tag_file = std::fs::File::open(path)?;

        let end_tag = ModelEndTag::try_parse(&mut end_tag_file, NoteSdkType::SPen)?;

        println!("{path}: {end_tag:#?}");
    }

    Ok(())
}

fn demo_note_doc() -> color_eyre::Result<()> {
    let note_doc_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010/note.note",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754/note.note",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735/note.note",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632/note.note",
        "/home/alex/projects/re/sdocx/sample_docs/empty encrypted_260219_125722/note.note",
        "/home/alex/projects/re/sdocx/sample_docs/Typed, formatted text with summary and voice memo_260220_003622/note.note",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features_260220_005438/note.note",
        "/home/alex/projects/re/sdocx/sample_docs/uses LOADS of features plus dupes_260220_010554/note.note",
    ];

    for path in note_doc_paths {
        let mut note_doc_file = std::fs::File::open(path)?;

        let note_doc = NoteDoc::try_parse(&mut note_doc_file)?;

        println!("{path}: {note_doc:#?}");
    }

    Ok(())
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    demo_media_info()?;
    demo_end_tag()?;
    demo_note_doc()?;

    Ok(())
}
