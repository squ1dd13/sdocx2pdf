use crate::{
    AppVersion, AppVersionParseError,
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BoundedStream, ByteStreamLe, ReadBitfieldError, ReadStringError, ReadTimestampError,
        TakeInclusiveLengthPrefixedError, TryParse, UnfinishedParsingError,
    },
    context::{DocumentContext, TryParseWithContext},
    end_tag::{
        BackgroundTheme, InvalidBackgroundThemeError, InvalidTextDirectionError, TextDirection,
    },
    impl_try_from_for_optional_from,
    media_info::{BoundFile, FileRegistry, NoSuchRegisteredFileError},
    page::object::text::{Text, TextParseError},
    read_size_and_map, read_size_and_vec, unpack_bool_flag, unpack_field_flags,
};
use chrono::{DateTime, Utc};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use sha2::Digest;
use std::{
    collections::HashMap,
    io::{self, Cursor, Read, Seek, SeekFrom},
    rc::Rc,
};
use thiserror::Error;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum AuthorInfoParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
}

#[derive(Debug)]
#[expect(dead_code)]
struct AuthorInfo {
    strings: [String; 3],
    image_id: u32,
}

impl<R: Read> TryParse<R> for AuthorInfo {
    type ParseError = AuthorInfoParseError;

    fn try_parse(reader: &mut R) -> Result<AuthorInfo, AuthorInfoParseError> {
        Ok(AuthorInfo {
            strings: [
                reader.read_short_u16_string()?,
                reader.read_short_u16_string()?,
                reader.read_short_u16_string()?,
            ],
            image_id: reader.read_u32_le()?,
        })
    }
}

#[derive(Error, Debug)]
pub enum PenInfoParseError {
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
#[expect(dead_code)]
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
    fn try_parse_simple(stream: &mut impl Read) -> Result<PenInfo, PenInfoParseError> {
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

    fn try_parse_full(stream: &mut impl Read) -> Result<PenInfo, PenInfoParseError> {
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

#[derive(Debug, FromPrimitive)]
enum VoiceAction {
    /// `VOICE_ACTION_NONE`
    None = 0,
    /// `VOICE_ACTION_START`
    Start = 1,
    /// `VOICE_ACTION_PAUSE`
    Pause = 2,
    /// `VOICE_ACTION_RESUME`
    Resume = 3,
    /// `VOICE_ACTION_STOP`
    Stop = 4,
}

impl_try_from_for_optional_from!(VoiceAction, u32, from_u32, pub InvalidVoiceActionError);

#[derive(Debug)]
#[expect(dead_code)]
struct VoiceEvent {
    time: DateTime<Utc>,
    action: VoiceAction,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum VoiceRecordingParseError {
    Io(#[from] std::io::Error),
    NoSuchFile(#[from] NoSuchRegisteredFileError),
    String(#[from] ReadStringError),
    Timestamp(#[from] ReadTimestampError),
    Action(#[from] InvalidVoiceActionError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("event count {0} is too large")]
    TooManyEvents(u32),
}

#[derive(Debug)]
#[expect(dead_code)]
struct VoiceRecording {
    file: Rc<BoundFile>,
    name: String,
    duration_str: String,
    date_created: DateTime<Utc>,
    events: Vec<VoiceEvent>,
    precise_duration: chrono::Duration,
}

impl<R: Read> TryParseWithContext<R, FileRegistry> for VoiceRecording {
    type ParseError = VoiceRecordingParseError;

    fn try_parse_with_ctx(
        reader: &mut R,
        file_registry: &FileRegistry,
    ) -> Result<VoiceRecording, VoiceRecordingParseError> {
        let mut reader = reader.read_u32_le().map(|n| reader.take(n.into()))?;

        let vri = VoiceRecording {
            file: file_registry.try_get(reader.read_u32_le()?)?,
            name: reader.read_short_u16_string()?,
            duration_str: reader.read_short_u16_string()?,
            date_created: reader.read_timestamp()?,
            events: read_size_and_vec!(reader, u32, VoiceRecordingParseError::TooManyEvents, {
                VoiceEvent {
                    action: reader.read_u32_le()?.try_into()?,
                    time: reader.read_timestamp()?,
                }
            }),
            precise_duration: chrono::Duration::milliseconds(reader.read_i64_le()?),
        };

        reader.ensure_eof()?;

        Ok(vri)
    }
}

#[derive(Debug)]
#[expect(dead_code)]
struct NoteDocMetadata {
    app_name: Option<String>,
    app_version: Option<AppVersion>,
    author_info: Option<AuthorInfo>,
    latitude_longitude: Option<(f64, f64)>,
}

#[derive(Error, Debug)]
#[error("there is no string registered with id {0}")]
pub struct NoSuchRegisteredStringError(u32);

#[derive(Default, Debug)]
pub struct StringRegistry {
    /// Keys are string IDs.
    strings: HashMap<u32, Rc<str>>,
}

impl StringRegistry {
    pub fn get(&self, key: u32) -> Option<Rc<str>> {
        self.strings.get(&key).map(Rc::clone)
    }

    pub fn try_get(&self, key: u32) -> Result<Rc<str>, NoSuchRegisteredStringError> {
        self.get(key).ok_or(NoSuchRegisteredStringError(key))
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum StringRegistryParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
    Unfinished(#[from] UnfinishedParsingError),
}

impl<R: Read> TryParse<R> for StringRegistry {
    type ParseError = StringRegistryParseError;

    fn try_parse(reader: &mut R) -> std::result::Result<StringRegistry, StringRegistryParseError> {
        let mut reader = reader.read_u32_le().map(|v| reader.take(v.into()))?;

        // If the size of the string manager is zero, there's nothing to read.
        if reader.limit() == 0 {
            return Ok(Default::default());
        }

        let registry = StringRegistry {
            strings: read_size_and_map!(
                reader,
                u16,
                (
                    reader.read_u32_le()?,
                    reader.read_short_u16_string()?.into(),
                )
            ),
        };

        reader.ensure_eof()?;

        Ok(registry)
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum NoteDocParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
    Timestamp(#[from] ReadTimestampError),
    Text(#[from] TextParseError),
    AppVersion(#[from] AppVersionParseError),
    AuthorInfo(#[from] AuthorInfoParseError),
    StringRegistry(#[from] StringRegistryParseError),
    PenInfo(#[from] PenInfoParseError),
    VoiceRecording(#[from] VoiceRecordingParseError),
    NoSuchFile(#[from] NoSuchRegisteredFileError),
    TextDirection(#[from] InvalidTextDirectionError),
    BackgroundTheme(#[from] InvalidBackgroundThemeError),

    #[error("failed to read property flags")]
    PropertyFlags(#[source] ReadBitfieldError),

    #[error("one or more properties were unhandled")]
    UnhandledProperty(#[source] UnhandledBitsError),

    #[error("failed to read field flags")]
    FieldFlags(#[source] ReadBitfieldError),

    #[error("one or more field flags were unhandled")]
    UnhandledField(#[source] UnhandledBitsError),

    #[error("bytes left over after parsing text object")]
    TextObject(#[source] UnfinishedParsingError),

    #[error("{0} too large for `usize`")]
    UsizeTooSmall(u32),

    #[error("computed hash does not match hash in stream")]
    HashMismatch,
}

/// `libSpen_worddoc.dll`
#[derive(Debug)]
#[expect(dead_code)]
pub struct NoteDoc {
    is_background_colour_inverted: bool,
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
    pub title_text: Text,
    pub body_text: Text,
    metadata: NoteDocMetadata,
    template_uri: Option<String>,
    last_edited_page_index: Option<u32>,
    last_edited_page_image_id: Option<i32>,
    last_edited_page_time: Option<DateTime<Utc>>,
    body_text_font_size_delta: Option<i32>,
    compatible_last_pen_info: Option<PenInfo>,
    voice_data: Option<Vec<VoiceRecording>>,
    attached_files: Option<HashMap<String, Rc<BoundFile>>>,
    last_pen_info: Option<PenInfo>,
    server_check_point: Option<i64>,
    fixed_font: Option<String>,
    fixed_text_direction: Option<TextDirection>,
    fixed_background_theme: Option<BackgroundTheme>,
    text_summarisation: Option<String>,
    stroke_group_size: Option<u32>,
    app_custom_data: Option<String>,

    string_registry: StringRegistry,

    hash: [u8; 32],
}

impl NoteDoc {
    pub const fn string_registry(&self) -> &StringRegistry {
        &self.string_registry
    }

    pub const fn hash(&self) -> &[u8; 32] {
        &self.hash
    }
}

impl<R: Read + Seek> TryParseWithContext<R, FileRegistry> for NoteDoc {
    type ParseError = NoteDocParseError;

    fn try_parse_with_ctx(
        reader: &mut R,
        file_registry: &FileRegistry,
    ) -> Result<NoteDoc, NoteDocParseError> {
        let start = reader.stream_position()?;
        let flex_offset = start + u64::from(reader.read_u32_le()?);

        let is_background_colour_inverted = {
            let mut property_flags =
                CheckedBitfield::try_parse(reader).map_err(NoteDocParseError::PropertyFlags)?;

            unpack_bool_flag!(property_flags, 3 => v);

            property_flags
                .ensure_none_set_unchecked()
                .map_err(NoteDocParseError::UnhandledProperty)?;

            v
        };

        let mut field_flags =
            CheckedBitfield::try_parse(reader).map_err(NoteDocParseError::FieldFlags)?;

        let format_version = reader.read_u32_le()?;
        let id = reader.read_short_u16_string()?;
        let file_revision = reader.read_u32_le()?;
        let created_time = reader.read_timestamp()?;
        let modified_time = reader.read_timestamp()?;
        let width = reader.read_u32_le()?;
        let height = reader.read_u32_le()?;
        let page_horizontal_padding = reader.read_u32_le()?;
        let page_vertical_padding = reader.read_u32_le()?;
        let min_format_version = reader.read_u32_le()?;

        // The text objects need file and string registries for parsing. We have the file registry,
        // but we haven't yet parsed the string registry, so we read the bytes only and defer the
        // parsing until after we have the string registry.
        let title_text_bytes = {
            let n = reader.read_u32_le()?;
            reader.read_u8s(
                n.try_into()
                    .map_err(|_| NoteDocParseError::UsizeTooSmall(n))?,
            )?
        };

        let body_text_bytes = {
            let n = reader.read_u32_le()?;
            reader.read_u8s(
                n.try_into()
                    .map_err(|_| NoteDocParseError::UsizeTooSmall(n))?,
            )?
        };

        // fixme: Sometimes there's an eight-byte underread here.
        // Parsing the gap as (u32, u32) yields something that looks suspiciously like a
        // size (e.g. (1600, 2262)). Sometimes it's not there, though.

        {
            let here = reader.stream_position()?;

            if here != flex_offset {
                eprintln!(
                    "Warning: Did not reach note flex offset naturally. \
                Will seek from {here} to {flex_offset} (delta {}).",
                    flex_offset.wrapping_sub(here).cast_signed()
                );

                reader.seek(SeekFrom::Start(flex_offset))?;
            }
        }

        unpack_field_flags!(field_flags, {
            0 => app_name: reader.read_short_u16_string()?;
            1 => app_version: AppVersion::try_parse(reader)?;
            2 => author_info: AuthorInfo::try_parse(reader)?;
            3 => latitude_longitude: (reader.read_f64_le()?, reader.read_f64_le()?);
            // missing 4, 5
            6 => template_uri: reader.read_short_u16_string()?;
            7 => last_edited_page_index: reader.read_u32_le()?;
            // missing 8

            // These two are on the same bit:
            9 => last_edited_page_image_id: reader.read_i32_le()?;
            9 => last_edited_page_time: reader.read_timestamp()?;

            10 => string_registry: StringRegistry::try_parse(reader)?, else Default::default();
            11 => body_text_font_size_delta: reader.read_i32_le()?;
            12 => compatible_last_pen_info: PenInfo::try_parse_simple(reader)?;
            13 => voice_data: read_size_and_vec!(
                reader,
                u32,
                NoteDocParseError::UsizeTooSmall,
                VoiceRecording::try_parse_with_ctx(reader, file_registry)?,
            );

            14 => attached_files: read_size_and_map!(
                reader,
                u16,
                (
                    reader.read_short_u16_string()?,
                    file_registry.try_get(reader.read_u32_le()?)?,
                )
            );

            15 => last_pen_info: PenInfo::try_parse_full(reader)?;
            16 => server_check_point: reader.read_i64_le()?;
            17 => fixed_font: reader.read_short_u16_string()?;
            18 => fixed_text_direction: reader.read_u32_le()?.try_into()?;
            19 => fixed_background_theme: reader.read_u32_le()?.try_into()?;
            20 => text_summarisation: reader.read_short_u16_string()?;
            21 => stroke_group_size: reader.read_u32_le()?;
            22 => app_custom_data: reader.read_long_u16_string()?;
        });

        // Now that we have the string registry, we can parse the text objects.
        let doc_ctx = DocumentContext {
            file_registry,
            string_registry: &string_registry,
        };

        let title_text = Text::try_parse_with_ctx(&mut Cursor::new(title_text_bytes), &doc_ctx)?;
        let body_text = Text::try_parse_with_ctx(&mut Cursor::new(body_text_bytes), &doc_ctx)?;

        field_flags
            .ensure_none_set_unchecked()
            .map_err(NoteDocParseError::UnhandledField)?;

        let calculated_hash = {
            // Calculate the number of bytes we have read in total.
            let data_size = reader.stream_position()? - start;

            reader.seek(SeekFrom::Start(start))?;

            let mut hasher = sha2::Sha256::new();

            // Copy exactly `data_size` bytes from the stream into the hasher. Note that this
            // brings the position in the stream back to where it was before the `seek` above.
            io::copy(&mut reader.take(data_size), &mut hasher)?;

            hasher.finalize()
        };

        let hash_in_stream = {
            let mut b = [0_u8; 32];
            reader.read_exact(&mut b)?;
            b
        };

        if calculated_hash[..] != hash_in_stream {
            return Err(NoteDocParseError::HashMismatch);
        }

        Ok(NoteDoc {
            is_background_colour_inverted,
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
            metadata: NoteDocMetadata {
                app_name,
                app_version,
                author_info,
                latitude_longitude,
            },
            template_uri,
            last_edited_page_index,
            last_edited_page_image_id,
            last_edited_page_time,
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
            string_registry,
            hash: hash_in_stream,
        })
    }
}
