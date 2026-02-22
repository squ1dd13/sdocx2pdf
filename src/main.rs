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
    missing_copy_implementations,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::mod_module_files,
    non_ascii_idents,
    noop_method_call,
    clippy::option_if_let_else,
    clippy::semicolon_if_nothing_returned,
    clippy::unseparated_literal_suffix,
    clippy::shadow_unrelated,
    clippy::similar_names,
    clippy::suspicious_operation_groupings,
    clippy::todo,
    unused_crate_dependencies,
    unused_extern_crates,
    unused_import_braces,
    clippy::unused_self,
    clippy::used_underscore_binding,
    clippy::useless_let_if_seq,
    clippy::wildcard_dependencies,
    clippy::wildcard_imports
)]

use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use color_eyre::eyre::{Context, OptionExt, eyre};
use indexmap::IndexMap;
use sha2::Digest;
use std::{
    collections::HashMap,
    io::{Cursor, Seek, SeekFrom},
    path::PathBuf,
};

fn read_u8_buf(stream: &mut impl ReadBytesExt, n: usize) -> color_eyre::Result<Vec<u8>> {
    let mut bytes = vec![0_u8; n];
    stream.read_exact(&mut bytes)?;

    Ok(bytes)
}

fn read_u8_string(stream: &mut impl ReadBytesExt, n_chars: usize) -> color_eyre::Result<String> {
    String::from_utf8(read_u8_buf(stream, n_chars)?).map_err(From::from)
}

fn read_u16_string(stream: &mut impl ReadBytesExt, n_chars: usize) -> color_eyre::Result<String> {
    let mut buf = vec![0_u16; n_chars];
    stream.read_u16_into::<LittleEndian>(&mut buf)?;

    char::decode_utf16(buf)
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

fn read_short_u8_string(stream: &mut impl ReadBytesExt) -> color_eyre::Result<String> {
    let n_chars: usize = stream.read_u16::<LittleEndian>()?.into();
    read_u8_string(stream, n_chars)
}

fn read_timestamp(stream: &mut impl ReadBytesExt) -> color_eyre::Result<DateTime<Utc>> {
    let value = stream.read_i64::<LittleEndian>()?;

    DateTime::from_timestamp_micros(value).ok_or_else(|| eyre!("Invalid timestamp value {value}"))
}

fn read_variable_length_bitfield(stream: &mut impl ReadBytesExt) -> color_eyre::Result<u32> {
    let n_bytes = stream.read_u8()?;

    Ok(match n_bytes {
        0 => 0,
        1 => stream.read_u8()?.into(),
        2 => stream.read_u16::<LittleEndian>()?.into(),
        3 => stream.read_u24::<LittleEndian>()?,
        4 => stream.read_u32::<LittleEndian>()?,
        5.. => {
            return Err(eyre!(
                "Variable length bitfield cannot be more than 4 bytes (found {n_bytes})"
            ));
        }
    })
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
    fn try_parse_exclusive<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<OpaqueBytes> {
        let size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;

        Ok(OpaqueBytes {
            bytes: read_u8_buf(stream, size)?,
        })
    }

    /// Reads `size: u32` and the `size - 4` bytes that follow, reading `size` bytes in total.
    fn try_parse_inclusive<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<OpaqueBytes> {
        let size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;

        Ok(OpaqueBytes {
            bytes: read_u8_buf(
                stream,
                size.checked_sub(4).ok_or_else(|| {
                    eyre!("Size ({size}) cannot be inclusive as it is less than 4")
                })?,
            )?,
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
                let mut rgba = [0_u8; 4];
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
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<NoteDoc> {
        let flexible_data_area_offset = {
            let start_offset = stream.stream_position()?;
            let flexible_data_area_jump: u64 = stream.read_u32::<LittleEndian>()?.into();

            start_offset + flexible_data_area_jump
        };

        let property_flags = read_variable_length_bitfield(stream)?;
        let field_check_flags = read_variable_length_bitfield(stream)?;

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

        let title_text = OpaqueBytes::try_parse_exclusive(stream)?;
        let body_text = OpaqueBytes::try_parse_exclusive(stream)?;

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
                    let string_id = stream.read_u32::<LittleEndian>()?;
                    let string = read_short_u16_string(stream)?;

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
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<EncryptionInfo> {
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

#[derive(Debug)]
enum PagingType {
    /// Traditional pages.
    Paged,

    /// Appears pageless to the user. Implemented by putting everything on one large page.
    Pageless,

    Unknown(u16),
}

impl PagingType {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<PagingType> {
        Ok(match stream.read_u16::<LittleEndian>()? {
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
    fn try_parse<T: ReadBytesExt + Seek>(
        stream: &mut T,
        sdk_type: NoteSdkType,
    ) -> color_eyre::Result<ModelEndTag> {
        let tag_size: usize = stream.read_u16::<LittleEndian>()?.into();

        // Make sure the tag specifies the SDK type we are expecting.
        let (ident_matches, ident_found) = sdk_type.verify_block_end(stream, tag_size)?;

        let expected_ident = sdk_type.ident();

        if !ident_matches {
            return Err(ident_found.map_or_else(
                || eyre!("Not enough space for ident '{expected_ident}'"),
                |found| eyre!("Ident '{found}' does not match expected '{expected_ident}'"),
            ));
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

        let page_model = PagingType::try_parse(stream)?;
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
                    let data_size: u64 = stream.read_u32::<LittleEndian>()?.into();
                    let stream_pos_pre = stream.stream_position()?;
                    let expected_data_end = stream_pos_pre + data_size;

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
                    });
                }

                bound_files
            },
        })
    }
}

#[derive(Debug)]
struct PageIdInfoPage {
    page_id: String,
    hash: [u8; 32],
}

#[derive(Debug)]
struct PageIdInfo {
    /// The SHA256 digest from the associated `note.note` file.
    note_doc_sha256: [u8; 32],
    pages: Vec<PageIdInfoPage>,
}

impl PageIdInfo {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<PageIdInfo> {
        let mut note_doc_sha256 = [0_u8; 32];
        stream.read_exact(&mut note_doc_sha256)?;

        let page_count = stream.read_u16::<LittleEndian>()?;

        let mut pages = Vec::with_capacity(page_count.into());

        for _ in 0..page_count {
            pages.push(PageIdInfoPage {
                page_id: read_short_u16_string(stream)?,
                hash: {
                    let mut buf = [0_u8; 32];
                    stream.read_exact(&mut buf)?;
                    buf
                },
            });
        }

        Ok(PageIdInfo {
            note_doc_sha256,
            pages,
        })
    }
}

#[derive(Debug)]
struct PointF64 {
    x: f64,
    y: f64,
}

impl PointF64 {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<PointF64> {
        Ok(PointF64 {
            x: stream.read_f64::<LittleEndian>()?,
            y: stream.read_f64::<LittleEndian>()?,
        })
    }
}

#[derive(Debug)]
struct RectF64 {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl RectF64 {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<RectF64> {
        Ok(RectF64 {
            left: stream.read_f64::<LittleEndian>()?,
            top: stream.read_f64::<LittleEndian>()?,
            right: stream.read_f64::<LittleEndian>()?,
            bottom: stream.read_f64::<LittleEndian>()?,
        })
    }
}

#[derive(Debug)]
struct PdfDataItem {
    bind_id: u32,
    page_index: u32,
    pdf_rect: RectF64,
}

impl PdfDataItem {
    fn try_parse<T: ReadBytesExt>(
        stream: &mut T,
        format_version: u32,
    ) -> color_eyre::Result<PdfDataItem> {
        Ok(PdfDataItem {
            bind_id: stream.read_u32::<LittleEndian>()?,
            page_index: stream.read_u32::<LittleEndian>()?,
            pdf_rect: if format_version < 2034 {
                RectF64::try_parse(stream)?
            } else {
                RectF64 {
                    left: stream.read_i32::<LittleEndian>()?.into(),
                    top: stream.read_i32::<LittleEndian>()?.into(),
                    right: stream.read_i32::<LittleEndian>()?.into(),
                    bottom: stream.read_i32::<LittleEndian>()?.into(),
                }
            },
        })
    }
}

#[derive(Debug)]
struct CanvasCacheEntry {
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
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<CanvasCacheEntry> {
        Ok(CanvasCacheEntry {
            file_id: stream.read_u32::<LittleEndian>()?,
            width: stream.read_u32::<LittleEndian>()?,
            height: stream.read_u32::<LittleEndian>()?,
            is_dark_mode: stream.read_u8()? == 1,
            background_colour: stream.read_u32::<LittleEndian>()?.to_le_bytes(),
            version: [
                stream.read_u32::<LittleEndian>()?,
                stream.read_u32::<LittleEndian>()?,
                stream.read_u32::<LittleEndian>()?,
            ],
            cache_version: stream.read_u32::<LittleEndian>()?,
            property: stream.read_u32::<LittleEndian>()?,
            locale_list_id: stream.read_u32::<LittleEndian>()?,
            system_font_path_hash: stream.read_u32::<LittleEndian>()?,
        })
    }
}

#[derive(Debug)]
struct CustomPageObject {
    object_type: u32,
    inner: OpaqueBytes,
}

impl CustomPageObject {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<CustomPageObject> {
        let object_type = stream.read_u32::<LittleEndian>()?;

        Ok(CustomPageObject {
            object_type,
            inner: OpaqueBytes::try_parse_exclusive(stream)?,
        })
    }
}

#[derive(Debug, Default)]
struct DocBundle {
    strings: IndexMap<String, String>,
    integers: IndexMap<String, u32>,
    string_vecs: IndexMap<String, Vec<String>>,
    byte_vecs: IndexMap<String, OpaqueBytes>,
}

impl DocBundle {
    fn try_parse<T: ReadBytesExt>(stream: &mut T) -> color_eyre::Result<DocBundle> {
        let map_presence_flags = stream.read_u8()?;

        let mut bundle = DocBundle::default();

        if map_presence_flags & 1 != 0 {
            let entry_count: usize = stream.read_u16::<LittleEndian>()?.into();
            bundle.strings.reserve(entry_count);

            for _ in 0..entry_count {
                bundle.strings.insert(
                    read_short_u8_string(stream)?,
                    read_short_u16_string(stream)?,
                );
            }
        }

        if map_presence_flags & 2 != 0 {
            let entry_count: usize = stream.read_u16::<LittleEndian>()?.into();
            bundle.integers.reserve(entry_count);

            for _ in 0..entry_count {
                bundle.integers.insert(
                    read_short_u8_string(stream)?,
                    stream.read_u32::<LittleEndian>()?,
                );
            }
        }

        if map_presence_flags & 4 != 0 {
            let entry_count: usize = stream.read_u16::<LittleEndian>()?.into();
            bundle.string_vecs.reserve(entry_count);

            for _ in 0..entry_count {
                let key = read_short_u8_string(stream)?;

                let string_count: usize = stream.read_u16::<LittleEndian>()?.into();
                let mut strings = Vec::with_capacity(string_count);

                for _ in 0..string_count {
                    strings.push(read_short_u16_string(stream)?);
                }

                bundle.string_vecs.insert(key, strings);
            }
        }

        if map_presence_flags & 8 != 0 {
            let entry_count: usize = stream.read_u16::<LittleEndian>()?.into();
            bundle.byte_vecs.reserve(entry_count);

            for _ in 0..entry_count {
                bundle.byte_vecs.insert(
                    read_short_u8_string(stream)?,
                    OpaqueBytes::try_parse_exclusive(stream)?,
                );
            }
        }

        Ok(bundle)
    }
}

#[derive(Debug)]
struct ObjectBase {
    rotatable: bool,
    selectable: bool,
    movable: bool,
    visible: bool,
    replayable: bool,
    clippable: bool,
    is_template_object: bool,
    flippable: bool,
    has_been_to_att: bool,
    lock_state: bool,
    removable: bool,

    format_version: u32,
    uuid: String,
    modified_time: DateTime<Utc>,
    rect: RectF64,
    timestamp_int: u32,
    resizable_b: u8,

    rotation_degree: f32,
    unknown_somethings: Option<Vec<[u8; 16]>>,
    ao_info: Option<String>,
    sor_bundle: Option<DocBundle>,
    plugin_link: Option<String>,
    extra_bundle: Option<DocBundle>,
    attached_file_id: Option<u32>,
    min_width_height: Option<(f32, f32)>,
    max_width_height: Option<(f32, f32)>,
    append_time: Option<DateTime<Utc>>,
    owner_page_width_height: Option<(u32, u32)>,
    layout_type: u32,
    unknown_20: Option<[u8; 20]>,
    thumbnail_bind_id: Option<u32>,
    pivot: Option<PointF64>,
    group_id: Option<String>,
}

impl ObjectBase {
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<ObjectBase> {
        let expected_end = {
            let start = stream.stream_position()?;
            let base_size: u64 = stream.read_u32::<LittleEndian>()?.into();

            start + base_size
        };

        let data_type = stream.read_u16::<LittleEndian>()?;

        if data_type != 0 {
            return Err(eyre!("Data type should be 0, not {data_type}"));
        }

        let flex_offset = stream.read_u32::<LittleEndian>()?;
        let has_flex_data = flex_offset != 0;

        let property_flags = read_variable_length_bitfield(stream)?;

        let rotatable = property_flags & 1 != 0;
        let selectable = property_flags & 2 != 0;
        let movable = property_flags & 4 != 0;
        let visible = property_flags & 8 != 0;
        let replayable = property_flags & 16 != 0;
        let clippable = property_flags & 32 != 0;
        let is_template_object = property_flags & 64 != 0;
        let flippable = property_flags & 128 != 0;
        let has_been_to_att = property_flags & 256 != 0;
        let lock_state = property_flags & 512 != 0;

        // Note: This is encoded as its inverse.
        let removable = property_flags & 4096 == 0;

        // These are the "stated" field check flags because if the flex offset is zero, we treat
        // them all as unset.
        let stated_field_check_flags = read_variable_length_bitfield(stream)?;
        let format_version = stream.read_u32::<LittleEndian>()?;

        if format_version > 5034 {
            eprintln!("Warning: Version {format_version} newer than expected (5034)");
        }

        let uuid = read_short_u8_string(stream)?;
        let modified_time = read_timestamp(stream)?;
        let rect = RectF64::try_parse(stream)?;
        let timestamp_int = stream.read_u32::<LittleEndian>()?;
        let resizable_b = stream.read_u8()?;

        let field_check_flags = if has_flex_data {
            stated_field_check_flags
        } else {
            0
        };

        let rotation_degree = (field_check_flags & 1 != 0)
            .then(|| stream.read_f32::<LittleEndian>())
            .transpose()?
            .unwrap_or(0.);

        let unknown_somethings = if field_check_flags & 2 != 0 {
            let count = stream.read_u16::<LittleEndian>()?;

            // JVM code skips over these once it knows how many there are.
            let mut somethings = Vec::with_capacity(count.into());

            for _ in 0..count {
                let mut bytes = [0_u8; 16];
                stream.read_exact(&mut bytes)?;
                somethings.push(bytes);
            }

            Some(somethings)
        } else {
            None
        };

        let ao_info = (field_check_flags & 4 != 0)
            .then(|| read_short_u16_string(stream))
            .transpose()?;

        let sor_bundle = (field_check_flags & 8 != 0)
            .then(|| DocBundle::try_parse(stream))
            .transpose()?;

        let plugin_link = (field_check_flags & 16 != 0)
            .then(|| read_short_u16_string(stream))
            .transpose()?;

        let extra_bundle = (field_check_flags & 32 != 0)
            .then(|| DocBundle::try_parse(stream))
            .transpose()?;

        let attached_file_id = (field_check_flags & 64 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?;

        let min_width_height = if field_check_flags & 128 != 0 {
            Some((
                stream.read_f32::<LittleEndian>()?,
                stream.read_f32::<LittleEndian>()?,
            ))
        } else {
            None
        };

        let max_width_height = if field_check_flags & 256 != 0 {
            Some((
                stream.read_f32::<LittleEndian>()?,
                stream.read_f32::<LittleEndian>()?,
            ))
        } else {
            None
        };

        let append_time = (field_check_flags & 8192 != 0)
            .then(|| read_timestamp(stream))
            .transpose()?;

        let owner_page_width_height = if field_check_flags & 16384 != 0 {
            Some((
                stream.read_u32::<LittleEndian>()?,
                stream.read_u32::<LittleEndian>()?,
            ))
        } else {
            None
        };

        let layout_type = (field_check_flags & 32768 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?
            .unwrap_or(0);

        let unknown_20 = if field_check_flags & 65536 != 0 {
            let mut bytes = [0_u8; 20];
            stream.read_exact(&mut bytes)?;
            Some(bytes)
        } else {
            None
        };

        let thumbnail_bind_id = (field_check_flags & 131072 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?;

        let pivot = (field_check_flags & 262144 != 0)
            .then(|| PointF64::try_parse(stream))
            .transpose()?;

        let group_id = (field_check_flags & 524288 != 0)
            .then(|| read_short_u16_string(stream))
            .transpose()?;

        let position_now = stream.stream_position()?;

        // Higher-level objects have data after the `ObjectBase`. For them to read this correctly,
        // the stream needs to be in exactly the right position.
        if position_now != expected_end {
            eprintln!(
                "Warning: Position after parsing ObjectBase is {position_now}, \
                 but {expected_end} expected. Will seek to correct this."
            );

            stream.seek(SeekFrom::Start(expected_end))?;
        }

        Ok(ObjectBase {
            rotatable,
            selectable,
            movable,
            visible,
            replayable,
            clippable,
            is_template_object,
            flippable,
            has_been_to_att,
            lock_state,
            removable,
            format_version,
            uuid,
            modified_time,
            rect,
            timestamp_int,
            resizable_b,
            rotation_degree,
            unknown_somethings,
            ao_info,
            sor_bundle,
            plugin_link,
            extra_bundle,
            attached_file_id,
            min_width_height,
            max_width_height,
            append_time,
            owner_page_width_height,
            layout_type,
            unknown_20,
            thumbnail_bind_id,
            pivot,
            group_id,
        })
    }
}

#[derive(Debug)]
struct OpaqueObjectInner {
    child_count: u16,
    inner: OpaqueBytes,
}

impl DocObjectInner for OpaqueObjectInner {
    fn try_parse<T: ReadBytesExt>(
        stream: &mut T,
        child_count: u16,
    ) -> color_eyre::Result<OpaqueObjectInner> {
        Ok(OpaqueObjectInner {
            child_count,
            inner: OpaqueBytes::try_parse_inclusive(stream)?,
        })
    }
}

trait DocObjectInner: Sized + std::fmt::Debug {
    fn try_parse<T: ReadBytesExt + Seek>(
        stream: &mut T,
        child_count: u16,
    ) -> color_eyre::Result<Self>;
}

#[derive(Debug)]
struct ObjectBaseWrapper<I: DocObjectInner> {
    base: ObjectBase,
    inner: I,
}

impl<I: DocObjectInner> ObjectBaseWrapper<I> {
    fn try_parse<T: ReadBytesExt + Seek>(
        stream: &mut T,
    ) -> color_eyre::Result<ObjectBaseWrapper<I>> {
        let child_count = stream.read_u16::<LittleEndian>()?;

        // This is the size of the `ObjectBase`, the inner object, and the hash.
        let total_size: u64 = stream.read_u32::<LittleEndian>()?.into();
        let expected_end = stream.stream_position()? + total_size;

        let base = ObjectBase::try_parse(stream)?;
        let inner = I::try_parse(stream, child_count)?;

        let mut hash_read = [0_u8; 32];
        stream.read_exact(&mut hash_read)?;

        let hash_calculated = sha2::Sha256::digest(
            format!("{}{}", base.uuid, base.modified_time.timestamp_micros()).as_bytes(),
        );

        if hash_calculated[..] != hash_read {
            eprintln!("Warning: Hash mismatch");
        } else {
            eprintln!("Hashes match!");
        }

        let here = stream.stream_position()?;

        if here != expected_end {
            eprintln!(
                "Warning: Object hash ended at {here}, not {expected_end}. Will `seek` to fix."
            );

            stream.seek(SeekFrom::Start(expected_end))?;
        } else {
            eprintln!("Object ended as expected");
        }

        Ok(ObjectBaseWrapper { base, inner })
    }
}

// fixme: `ObjectBaseWrapper<OpaqueObjectInner>` is wrong for anything that is not a direct
// subclass of `WCon_ObjectBase`.

#[derive(Debug)]
enum DocObject {
    /// `WCon_ObjectStroke`; extends `WCon_ObjectBase`
    Stroke {
        is_old_type: bool,
        object: ObjectBaseWrapper<OpaqueObjectInner>,
    },

    /// `WCon_ObjectTextBoxOrImage` (variant 1) extends `WCon_ObjectShape` (`Shape`)
    Text(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectTextBoxOrImage` (variant 0) extends `WCon_ObjectShape` (`Shape`)
    Image(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectContainer`; extends `WCon_ObjectBase`
    Container(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectShape`; extends `WCon_ObjectShapeBase`, which extends `WCon_ObjectBase`
    Shape(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectLine`; extends `WCon_ObjectShapeBase` (see `Shape`)
    Line(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectVoice`; extends `WCon_ObjectBase`
    Voice(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectFormula`; extends `WCon_ObjectBase`
    Formula(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectTable`; extends `WCon_ObjectBase`
    Table(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectWeb`; extends `WCon_ObjectBase`
    Web(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectPainting`; extends `WCon_ObjectBase`
    Painting(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectLink`; extends `WCon_ObjectBase`
    Link(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectMath`; extends `WCon_ObjectBase`
    Maths(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectPlot`; extends `WCon_ObjectBase`
    Plot(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectUnknown`; extends `WCon_ObjectBase`
    Generic(ObjectBaseWrapper<OpaqueObjectInner>),
}

impl DocObject {
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<DocObject> {
        let object_type = stream.read_u8()?;

        eprintln!("Object type {object_type}");

        let object = ObjectBaseWrapper::try_parse(stream)?;

        Ok(match object_type {
            1 | 15 => DocObject::Stroke {
                is_old_type: object_type == 15,
                object,
            },

            2 => DocObject::Text(object),
            3 => DocObject::Image(object),
            4 => DocObject::Container(object),
            7 => DocObject::Shape(object),
            8 => DocObject::Line(object),
            10 => DocObject::Voice(object),
            11 => DocObject::Formula(object),
            13 => DocObject::Web(object),
            14 => DocObject::Painting(object),
            17 => DocObject::Link(object),
            19 => DocObject::Generic(object),
            20 => DocObject::Plot(object),
            21 => DocObject::Maths(object),
            22 => DocObject::Table(object),

            unknown => return Err(eyre!("Unrecognised object type {unknown}")),
            // There is also an object type 100, for `WDocObjectStrokeGroup`.
            // As far as I can tell, this is not supposed to be written to disk, so we should
            // never read it.
        })
    }
}

#[derive(Debug)]
struct Layer {
    visible: bool,
    lock_state: bool,
    event_forwardable: bool,

    layer_id: u32,

    alpha: u8,
    background_colour: [u8; 4],
    name: Option<String>,
    uuid: Option<String>,
    modified_time: Option<DateTime<Utc>>,
    thumbnail_media_id: Option<u32>,
    shadow_effect: Option<OpaqueBytes>,

    objects: Vec<DocObject>,

    hash: [u8; 32],
}

impl Layer {
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<Layer> {
        let data_size = stream.read_u32::<LittleEndian>()?;
        let flex_offset: u64 = stream.read_u32::<LittleEndian>()?.into();

        let property_flags = read_variable_length_bitfield(stream)?;
        let field_check_flags = read_variable_length_bitfield(stream)?;

        // The first property flag is for invisibility, so visibility is its inverse.
        let visible = property_flags & 1 == 0;
        let lock_state = property_flags & 4 != 0;
        let event_forwardable = property_flags & 2 != 0;

        let layer_id = stream.read_u32::<LittleEndian>()?;

        stream.seek(SeekFrom::Start(flex_offset))?;

        let alpha = (field_check_flags & 1 != 0)
            .then(|| stream.read_u8())
            .transpose()?
            .unwrap_or(255);

        let background_colour = (field_check_flags & 2 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?
            .map_or([0xff, 0xff, 0xff, 0xff], u32::to_le_bytes);

        let name = (field_check_flags & 4 != 0)
            .then(|| read_short_u16_string(stream))
            .transpose()?;

        let uuid = (field_check_flags & 8 != 0)
            .then(|| read_short_u16_string(stream))
            .transpose()?;

        let modified_time = (field_check_flags & 16 != 0)
            .then(|| read_timestamp(stream))
            .transpose()?;

        let thumbnail_media_id = (field_check_flags & 32 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?;

        let shadow_effect = (field_check_flags & 64 != 0)
            .then(|| OpaqueBytes::try_parse_exclusive(stream))
            .transpose()?;

        let objects = {
            let object_count: usize = stream.read_u32::<LittleEndian>()?.try_into()?;

            let mut objects = Vec::with_capacity(object_count);

            for _ in 0..object_count {
                objects.push(DocObject::try_parse(stream)?);
            }

            objects
        };

        let mut hash = [0_u8; 32];
        stream.read_exact(&mut hash)?;

        Ok(Layer {
            visible,
            lock_state,
            event_forwardable,
            layer_id,
            alpha,
            background_colour,
            name,
            uuid,
            modified_time,
            thumbnail_media_id,
            shadow_effect,
            objects,
            hash,
        })
    }
}

#[derive(Debug)]
struct Page {
    is_text_only: bool,

    orientation: u32,
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    page_id: String,
    modified_time: DateTime<Utc>,
    format_version: u32,
    min_format_version: u32,

    drawn_rect: Option<RectF64>,
    tag_list: Option<Vec<String>>,
    template_uri: Option<String>,
    background_image_id: Option<i32>,
    background_image_mode: u32,
    background_colour: [u8; 4],
    background_width: u32,
    background_rotation: u32,
    pdf_data_items: Option<Vec<PdfDataItem>>,
    template_type: Option<u32>,
    canvas_cache_map: Vec<(u32, CanvasCacheEntry)>,
    imported_data_height: Option<u32>,
    theme: Option<u32>,
    recognised_data_modified_time: Option<DateTime<Utc>>,
    stroke_recognition_data: Option<Vec<OpaqueBytes>>,
    custom_objects: Vec<CustomPageObject>,

    hash: [u8; 32],

    layers: Vec<Layer>,
}

impl Page {
    const CLOSING_STRING: &str = "Page for SAMSUNG S-Pen SDK";

    /// Parses a single page using all of `stream`.
    ///
    /// `stream` must not have anything after the end of the page data, because this method
    /// seeks to the end of `stream` and expects it to match the correct format for a page.
    fn try_parse_full<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<Page> {
        let data_start_pos = stream.stream_position()?;
        let closing_string_size: i64 = Self::CLOSING_STRING.len().try_into()?;

        // Seek to where the closing string should begin.
        stream.seek(SeekFrom::End(-closing_string_size))?;

        let closing_string = read_u8_string(stream, Self::CLOSING_STRING.len())?;

        if closing_string != Self::CLOSING_STRING {
            return Err(eyre!(
                "Closing string '{closing_string}' does not match expected '{}'",
                Self::CLOSING_STRING
            ));
        }

        // Return to the beginning.
        stream.seek(SeekFrom::Start(data_start_pos))?;

        let page_size = stream.read_u32::<LittleEndian>()?;
        let flex_data_offset: u64 = stream.read_u32::<LittleEndian>()?.into();

        let property_flags = read_variable_length_bitfield(stream)?;
        let is_text_only = property_flags & 0x1 != 0;

        let field_check_flags = read_variable_length_bitfield(stream)?;

        // == "Fixed area" ==
        let orientation = stream.read_u32::<LittleEndian>()?;
        let width = stream.read_u32::<LittleEndian>()?;
        let height = stream.read_u32::<LittleEndian>()?;
        let offset_x = stream.read_u32::<LittleEndian>()?;
        let offset_y = stream.read_u32::<LittleEndian>()?;
        let page_id = read_short_u16_string(stream)?;
        let modified_time = read_timestamp(stream)?;
        let format_version = stream.read_u32::<LittleEndian>()?;
        let min_format_version = stream.read_u32::<LittleEndian>()?;
        // == End ==

        stream.seek(SeekFrom::Start(flex_data_offset))?;

        // == "Flexible area" ==
        let drawn_rect = (field_check_flags & 1 != 0)
            .then(|| RectF64::try_parse(stream))
            .transpose()?;

        let tag_list: Option<Vec<String>> = if field_check_flags & 2 != 0 {
            let tag_count = stream.read_u16::<LittleEndian>()?;

            Some(
                (0..tag_count)
                    .map(|_| read_short_u16_string(stream))
                    .collect::<color_eyre::Result<_>>()?,
            )
        } else {
            None
        };

        let template_uri = (field_check_flags & 4 != 0)
            .then(|| read_short_u16_string(stream))
            .transpose()?;

        let background_image_id = (field_check_flags & 8 != 0)
            .then(|| stream.read_i32::<LittleEndian>())
            .transpose()?;

        let background_image_mode = (field_check_flags & 16 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?
            .unwrap_or(0);

        let background_colour = (field_check_flags & 32 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?
            .map_or([0xff, 0xff, 0xff, 0xff], u32::to_le_bytes);

        let background_width = (field_check_flags & 64 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?
            .unwrap_or(0);

        let background_rotation = (field_check_flags & 128 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?
            .unwrap_or(0);

        let pdf_data_items: Option<Vec<PdfDataItem>> = if field_check_flags & 256 != 0 {
            let item_count = stream.read_u16::<LittleEndian>()?;

            let mut items = Vec::with_capacity(item_count.into());

            for _ in 0..item_count {
                items.push(PdfDataItem::try_parse(stream, format_version)?);
            }

            Some(items)
        } else {
            None
        };

        let template_type = (field_check_flags & 512 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?;

        // The app uses a `LinkedHashMap` here, so entry order must be important.
        // Since we are unlikely to use this for much, a `Vec` is fine in place of a real map.
        let mut canvas_cache_map: Vec<(u32, CanvasCacheEntry)> = vec![];

        if field_check_flags & 1024 != 0 {
            let entry_count: i64 = stream.read_u32::<LittleEndian>()?.into();
            let entry_size: i64 = stream.read_u16::<LittleEndian>()?.into();

            if entry_size == 49 {
                canvas_cache_map.reserve(entry_count.try_into()?);

                for _ in 0..entry_count {
                    let key = stream.read_u32::<LittleEndian>()?;
                    let entry = CanvasCacheEntry::try_parse(stream)?;

                    canvas_cache_map.push((key, entry));
                }
            } else {
                eprintln!("Skipping canvas cache map: entry size is {entry_size}, not 49.");
                stream.seek_relative(entry_count * entry_size)?;
            }
        }

        let imported_data_height = (field_check_flags & 2048 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?;

        let theme = (field_check_flags & 4096 != 0)
            .then(|| stream.read_u32::<LittleEndian>())
            .transpose()?;

        let recognised_data_modified_time = (field_check_flags & 32768 != 0)
            .then(|| read_timestamp(stream))
            .transpose()?;

        let stroke_recognition_data: Option<Vec<OpaqueBytes>> = if field_check_flags & 65536 != 0 {
            let entry_count = stream.read_u32::<LittleEndian>()?;

            let mut entries = Vec::with_capacity(entry_count as usize);

            for _ in 0..entry_count {
                entries.push(OpaqueBytes::try_parse_exclusive(stream)?);
            }

            Some(entries)
        } else {
            None
        };

        let mut custom_objects: Vec<CustomPageObject> = vec![];

        if field_check_flags & 262144 != 0 {
            let custom_object_count: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
            custom_objects.reserve(custom_object_count);

            for _ in 0..custom_object_count {
                custom_objects.push(CustomPageObject::try_parse(stream)?);
            }
        }

        // == End flexible ==

        // fixme: The hash could be read at basically any point, since we seek back after.
        // Not sure why it's done here.
        let hash = {
            let pos = stream.stream_position()?;

            // The hash comes before the closing string, so seek before both.
            stream.seek(SeekFrom::End(-closing_string_size - 32))?;

            let mut hash = [0_u8; 32];
            stream.read_exact(&mut hash)?;

            // Return to where we were before.
            stream.seek(SeekFrom::Start(pos))?;

            hash
        };

        let layer_count: usize = stream.read_u16::<LittleEndian>()?.into();
        let current_layer_index = stream.read_u16::<LittleEndian>()?;

        let mut layers = Vec::with_capacity(layer_count);

        for _ in 0..layer_count {
            layers.push(Layer::try_parse(stream)?);
        }

        let mut remaining_bytes = vec![];
        stream.read_to_end(&mut remaining_bytes)?;

        let expected_remaining_count: usize = (32 + closing_string_size).try_into()?;

        if remaining_bytes.len() != expected_remaining_count {
            return Err(eyre!(
                "Wrong number of bytes remaining: found {}, not {}",
                remaining_bytes.len(),
                expected_remaining_count
            ));
        }

        Ok(Page {
            is_text_only,
            orientation,
            width,
            height,
            offset_x,
            offset_y,
            page_id,
            modified_time,
            format_version,
            min_format_version,
            drawn_rect,
            tag_list,
            template_uri,
            background_image_id,
            background_image_mode,
            background_colour,
            background_width,
            background_rotation,
            pdf_data_items,
            template_type,
            canvas_cache_map,
            imported_data_height,
            theme,
            recognised_data_modified_time,
            stroke_recognition_data,
            custom_objects,
            hash,
            layers,
        })
    }
}

fn demo_for_extracted_dir(dir_path: impl AsRef<str>) -> color_eyre::Result<()> {
    let dir_path = dir_path.as_ref();

    let media_info_path: PathBuf = [dir_path, "media/mediaInfo.dat"].iter().collect();
    let media_info = MediaInfo::try_parse(&mut std::fs::File::open(&media_info_path)?)?;
    println!("{}: {media_info:#?}", media_info_path.display());

    let end_tag_path: PathBuf = [dir_path, "end_tag.bin"].iter().collect();
    let end_tag =
        ModelEndTag::try_parse(&mut std::fs::File::open(&end_tag_path)?, NoteSdkType::SPen)?;
    println!("{}: {end_tag:#?}", end_tag_path.display());

    let note_note_path: PathBuf = [dir_path, "note.note"].iter().collect();
    let note_note = NoteDoc::try_parse(&mut std::fs::File::open(&note_note_path)?)?;
    println!("{}: {note_note:#?}", note_note_path.display());

    let page_id_info_path: PathBuf = [dir_path, "pageIdInfo.dat"].iter().collect();
    let page_id_info = PageIdInfo::try_parse(&mut std::fs::File::open(&page_id_info_path)?)?;
    println!("{}: {page_id_info:?}", page_id_info_path.display());

    for page_info in &page_id_info.pages {
        let mut page_path: PathBuf = [dir_path, &page_info.page_id].iter().collect();
        page_path.set_extension("page");

        let page = Page::try_parse_full(
            &mut std::fs::File::open(&page_path)
                .wrap_err_with(|| eyre!("Failed to open {}", page_path.display()))?,
        )?;

        println!("{}: {page:#?}", page_path.display());
    }

    Ok(())
}

fn demo_all() -> color_eyre::Result<()> {
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
    ];

    for path in extracted_sdocx_paths {
        demo_for_extracted_dir(path)?;
    }

    Ok(())
}

// .ssf is "snap saved file"
// https://github.com/fschutt/printpdf

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    demo_all()?;

    Ok(())
}
