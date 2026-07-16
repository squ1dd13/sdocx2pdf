use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{BoundedStream, ByteStreamLe, ReadStringError, TryParse, UnfinishedParsingError},
    page::object::{
        base::{HasObjectBase, ObjectBase, ObjectBaseParseError},
        header::{FlagBlockError, ObjectHeaderError, try_parse_object_header},
    },
    unpack_bool_flag, unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum AudioParseError {
    Io(#[from] io::Error),
    Base(#[from] ObjectBaseParseError),
    Header(#[from] ObjectHeaderError),
    FlagBlock(#[from] FlagBlockError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("failed to read title string")]
    Title(#[source] ReadStringError),

    #[error("failed to read play time string")]
    PlayTime(#[source] ReadStringError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Audio {
    object_base: ObjectBase,

    is_recorded: bool,

    title: Option<String>,
    play_time: Option<String>,

    attached_file_id: Option<u32>,
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

            1 => title:
                stream.read_short_u16_string().map_err(AudioParseError::Title)?;

            2 => play_time:
                stream.read_short_u16_string().map_err(AudioParseError::PlayTime)?;
        });

        flag_block.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Audio {
            object_base,
            is_recorded,
            title,
            play_time,
            attached_file_id,
        })
    }
}

impl HasObjectBase for Audio {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}
