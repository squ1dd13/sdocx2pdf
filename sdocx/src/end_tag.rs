use crate::{
    AppVersion, AppVersionParseError,
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BoundedStream, ByteStreamLe, ReadStringError, ReadTimestampError, TryParse,
        UnfinishedParsingError,
    },
    context::TryParseWithContext,
    impl_try_from_for_optional_from, unpack_bool_flag,
};
use chrono::{DateTime, Utc};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{Read, Seek, SeekFrom};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSdkType {
    /// `SAMSUNG S-Pen PAINTING SDK`; 0x1 in DLL
    #[expect(dead_code)]
    SamsungSPenPainting,

    /// `S-Pen SDK`; 0x2 in DLL
    SPen,

    /// `S-Pen PAINTING SDK`; 0x3 in DLL
    #[expect(dead_code)]
    SPenPainting,

    /// `SAMSUNG S-Pen SDK`; 0x4 in DLL
    #[expect(dead_code)]
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
}

#[derive(Debug)]
#[expect(dead_code)]
struct EncryptionInfo {
    size: u32,
    salt: Vec<u8>,
    iv: Vec<u8>,
    key: Vec<u8>,
}

impl<R: Read> TryParse<R> for EncryptionInfo {
    type ParseError = std::io::Error;

    fn try_parse(reader: &mut R) -> std::io::Result<EncryptionInfo> {
        Ok(EncryptionInfo {
            size: reader.read_u32_le()?,
            salt: {
                let salt_size: usize = reader
                    .read_u32_le()?
                    .try_into()
                    .map_err(|_| std::io::ErrorKind::InvalidInput)?;

                reader.read_u8s(salt_size)?
            },
            iv: {
                let iv_size: usize = reader
                    .read_u32_le()?
                    .try_into()
                    .map_err(|_| std::io::ErrorKind::InvalidInput)?;

                reader.read_u8s(iv_size)?
            },
            key: {
                let key_size: usize = reader
                    .read_u32_le()?
                    .try_into()
                    .map_err(|_| std::io::ErrorKind::InvalidInput)?;

                reader.read_u8s(key_size)?
            },
        })
    }
}

#[derive(Debug, FromPrimitive)]
pub enum PageModel {
    /// `PageMode.LIST`
    Paged = 0,

    /// `PageMode.SINGLE`
    Pageless = 1,
}

impl_try_from_for_optional_from!(PageModel, u16, from_u16, pub InvalidPageModelError);

#[derive(Debug, FromPrimitive)]
pub enum DocumentType {
    /// `UNLOCKED_DOC`
    UnlockedDoc = 0,
    /// `LOCKED_SDOC`
    LockedSdoc = 1,
    /// `LOCKED_SPD`
    LockedSpd = 2,
    /// `LOCKED_SNB`
    LockedSnb = 3,
    /// `LOCKED_TMEMO`
    LockedTmemo = 4,
    /// `LOCKED_WDOC`
    LockedWdoc = 5,
}

impl_try_from_for_optional_from!(DocumentType, u16, from_u16, pub InvalidDocumentTypeError);

#[derive(Debug, FromPrimitive)]
pub enum TextDirection {
    /// `TEXT_DIRECTION_LTR`
    LeftToRight = 0,
    /// `TEXT_DIRECTION_RTL`
    RightToLeft = 1,
    /// `TEXT_DIRECTION_DEFAULT`
    Default = 2,
}

impl_try_from_for_optional_from!(TextDirection, u32, from_u32, pub InvalidTextDirectionError);

#[derive(Debug, FromPrimitive)]
pub enum BackgroundTheme {
    /// `THEME_LIGHT`
    Light = 0,
    /// `THEME_DARK`
    Dark = 1,
    /// `THEME_DEFAULT`
    Default = 2,
}

impl_try_from_for_optional_from!(BackgroundTheme, u32, from_u32, pub InvalidBackgroundThemeError);

#[derive(Debug, FromPrimitive)]
pub enum Orientation {
    /// `Orientation.PORTRAIT`
    Portrait = 0,
    /// `Orientation.LANDSCAPE`
    Landscape = 1,
    /// `Orientation.LANDSCAPE_NEW`
    LandscapeNew = 2,
}

impl_try_from_for_optional_from!(Orientation, u32, from_u32, pub InvalidOrientationError);

#[derive(Error, Debug)]
#[error(transparent)]
pub enum EndTagParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
    Timestamp(#[from] ReadTimestampError),
    AppVersion(#[from] AppVersionParseError),
    PageModel(#[from] InvalidPageModelError),
    DocumentType(#[from] InvalidDocumentTypeError),
    TextDirection(#[from] InvalidTextDirectionError),
    BackgroundTheme(#[from] InvalidBackgroundThemeError),
    Orientation(#[from] InvalidOrientationError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("one or more property bits were unhandled")]
    UnhandledProperty(#[from] UnhandledBitsError),

    #[error("expected ident '{1}', but found '{0}' instead")]
    WrongIdent(String, &'static str),
}

/// The structure in `end_tag.bin`.
#[derive(Debug)]
#[expect(dead_code)]
pub struct EndTag {
    sdk_type: NoteSdkType,

    format_version: u32,
    note_uuid: String,
    last_modified_time: DateTime<Utc>,
    is_landscape: bool,
    cover_image: String,
    note_width: u32,
    note_height: f32,
    app_name: String,
    app_version: AppVersion,
    min_format_version: u32,
    created_time: DateTime<Utc>,
    last_viewed_page_index: u32,
    page_model: PageModel,
    document_type: DocumentType,
    owner_id: String,
    encryption_info: Option<EncryptionInfo>,
    display_created_time: DateTime<Utc>,
    display_modified_time: DateTime<Utc>,
    last_recognised_data_modified_time: DateTime<Utc>,
    fixed_font: String,
    fixed_text_direction: TextDirection,
    fixed_background_theme: BackgroundTheme,
    server_check_point: i64,
    new_orientation: Orientation,
    min_unknown_version: u32,
    app_custom_data: String,
}

impl<R: Read + Seek> TryParseWithContext<R, NoteSdkType> for EndTag {
    type ParseError = EndTagParseError;

    fn try_parse_with_ctx(
        reader: &mut R,
        sdk_type: &NoteSdkType,
    ) -> Result<EndTag, EndTagParseError> {
        // First two bytes give the size of the rest of the data.
        let mut reader = reader.read_u16_le().map(|n| reader.take(n.into()))?;

        let ident = sdk_type.ident();

        // `unwrap` is fine because none of the idents come anywhere near even the `i8` limit.
        let ident_len_i = i64::try_from(ident.len()).unwrap();

        // The file's SDK type is indicated by a string at the end. Before parsing anything
        // significant, we have to go and find that string to make sure it matches the SDK type we
        // are trying to parse for.
        {
            reader.seek(SeekFrom::End(-ident_len_i))?;

            let mut last_bytes = Vec::with_capacity(ident.len());
            reader.read_to_end(&mut last_bytes)?;

            if last_bytes != ident.as_bytes() {
                return Err(EndTagParseError::WrongIdent(
                    // Waiting for `string_from_utf8_lossy_owned` to be made stable...
                    String::from_utf8_lossy(&last_bytes).into(),
                    ident,
                ));
            }

            // Go back to the beginning so we can actually parse the tag.
            reader.seek(SeekFrom::Start(0))?;
        }

        let format_version = reader.read_u32_le()?;
        let note_uuid = reader.read_short_u16_string()?;
        let last_modified_time = reader.read_timestamp()?;

        let is_landscape = {
            let mut property_flags: CheckedBitfield = reader.read_u32_le()?.into();
            unpack_bool_flag!(property_flags, 1 => v);

            property_flags.ensure_none_set_unchecked()?;

            v
        };

        let cover_image = reader.read_short_u16_string()?;

        // Notice the two different types here:
        let note_width = reader.read_u32_le()?;
        let note_height = reader.read_f32_le()?;

        let app_name = reader.read_short_u16_string()?;
        let app_version = AppVersion::try_parse(&mut reader)?;

        let min_format_version = reader.read_u32_le()?;

        let created_time = reader.read_timestamp()?;
        let last_viewed_page_index = reader.read_u32_le()?;

        let page_model: PageModel = reader.read_u16_le()?.try_into()?;
        let document_type: DocumentType = reader.read_u16_le()?.try_into()?;

        let owner_id = reader.read_short_u16_string()?;

        if let n_to_skip @ 1.. = reader.read_u32_le()? {
            eprintln!(
                "Warning: Skipping {n_to_skip} bytes in end tag. There must be something there!"
            );

            reader.seek_relative(n_to_skip.into())?;
        }

        if let _encryption_data_size @ 1.. = reader.read_u32_le()? {
            // Notes are exported unencrypted (usually?). If the encryption data is present, it
            // could mean that the note is actually encrypted, in which case we won't be able to do
            // much with it. You could probably get this to happen if you tried to read an end tag
            // from the app's storage (i.e., not exported).
            eprintln!(
                "Warning: Encryption data present, but it shouldn't be if this is an exported file"
            );

            Some(EncryptionInfo::try_parse(&mut reader)?)
        } else {
            None
        };

        let display_created_time = reader.read_timestamp()?;
        let display_modified_time = reader.read_timestamp()?;
        let last_recognised_data_modified_time = reader.read_timestamp()?;

        let fixed_font = reader.read_short_u16_string()?;
        let fixed_text_direction: TextDirection = reader.read_u32_le()?.try_into()?;
        let fixed_background_theme: BackgroundTheme = reader.read_u32_le()?.try_into()?;

        let server_check_point = reader.read_i64_le()?;

        let new_orientation: Orientation = reader.read_u32_le()?.try_into()?;
        let min_unknown_version = reader.read_u32_le()?;

        let app_custom_data = reader.read_long_u16_string()?;

        // Only thing left in our `Take` should be the ident.
        reader.seek_relative(ident_len_i)?;
        reader.ensure_eof()?;

        Ok(EndTag {
            sdk_type: *sdk_type,
            format_version,
            note_uuid,
            last_modified_time,
            is_landscape,
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
            encryption_info: None,
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
