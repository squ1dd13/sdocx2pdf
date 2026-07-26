use std::io::{self, Read, Seek};

use num::FromPrimitive;
use num_derive::FromPrimitive;
use thiserror::Error;

use crate::{
    byte_stream::{BoundedStream, ByteStreamLe, ReadStringError, TryParse, UnfinishedParsingError},
    impl_try_from_for_optional_from,
    page::object::{
        base::{HasObjectBase, ObjectBase, ObjectBaseParseError},
        header::{FlagBlock, FlagBlockError, ObjectHeaderError, try_parse_object_header},
    },
    read_u32_sized_vec, unpack_bool_flag, unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum BackgroundGradientError {
    Io(#[from] io::Error),
    FlagBlock(#[from] FlagBlockError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("{0} is too many gradient stops")]
    TooManyStops(u32),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct BackgroundGradient {
    stops: Vec<([u8; 4], f32)>,
}

impl<R: Read + Seek> TryParse<R> for BackgroundGradient {
    type ParseError = BackgroundGradientError;

    fn try_parse(stream: &mut R) -> Result<BackgroundGradient, BackgroundGradientError> {
        let mut stream = stream.exclusive_blind_window()?;

        FlagBlock::try_parse(&mut stream)?.ensure_flags_used()?;

        let stops = read_u32_sized_vec!(
            stream,
            BackgroundGradientError::TooManyStops,
            (stream.read_4_bytes()?, stream.read_f32_le()?),
        );

        stream.ensure_eof()?;

        Ok(BackgroundGradient { stops })
    }
}

#[derive(Debug, FromPrimitive, Default)]
enum ViewType {
    #[default]
    Small = 0,
    Medium = 1,
    Large = 2,
}

impl_try_from_for_optional_from!(ViewType, u32, from_u32, pub InvalidViewTypeError);

#[derive(Error, Debug)]
#[error(transparent)]
pub enum AudioParseError {
    Io(#[from] io::Error),
    Base(#[from] ObjectBaseParseError),
    Header(#[from] ObjectHeaderError),
    FlagBlock(#[from] FlagBlockError),
    Unfinished(#[from] UnfinishedParsingError),
    BackgroundGradient(#[from] BackgroundGradientError),
    ViewType(#[from] InvalidViewTypeError),

    #[error("failed to read title string")]
    Title(#[source] ReadStringError),

    #[error("failed to read play time string")]
    PlayTime(#[source] ReadStringError),

    #[error("failed to read body string")]
    Body(#[source] ReadStringError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Audio {
    object_base: ObjectBase,

    is_recorded: bool,

    title: Option<String>,
    play_time: Option<String>,
    attached_file_id: Option<u32>,
    background_gradient: Option<BackgroundGradient>,
    version: Option<u32>,
    thumbnail_file_id: Option<u32>,
    body: Option<String>,
    dummy_thumbnail_colours: Option<([u8; 4], [u8; 4])>,
    view_type: ViewType,
}

impl<R: Read + Seek> TryParse<R> for Audio {
    type ParseError = AudioParseError;

    fn try_parse(stream: &mut R) -> Result<Audio, AudioParseError> {
        let object_base = ObjectBase::try_parse(stream)?;

        let (mut flag_block, mut stream) = try_parse_object_header(stream, 10)?;

        unpack_bool_flag!(flag_block.property_flags_mut(), 0 => is_recorded);

        let field_flags = flag_block.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => attached_file_id: stream.read_u32_le()?;
            1 => title: stream.read_short_u16_string().map_err(AudioParseError::Title)?;
            2 => play_time: stream.read_short_u16_string().map_err(AudioParseError::PlayTime)?;
            3 => background_gradient: BackgroundGradient::try_parse(&mut stream)?;
            4 => version: stream.read_u32_le()?;
            // missing 5
            6 => thumbnail_file_id: stream.read_u32_le()?;
            7 => body: stream.read_short_u16_string().map_err(AudioParseError::Body)?;
            8 => dummy_thumbnail_colours: (stream.read_4_bytes()?, stream.read_4_bytes()?);
            9 => view_type: ViewType::try_from(stream.read_u32_le()?)?, else Default::default();
        });

        flag_block.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Audio {
            object_base,
            is_recorded,
            title,
            play_time,
            attached_file_id,
            background_gradient,
            version,
            thumbnail_file_id,
            body,
            dummy_thumbnail_colours,
            view_type,
        })
    }
}

impl HasObjectBase for Audio {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}
