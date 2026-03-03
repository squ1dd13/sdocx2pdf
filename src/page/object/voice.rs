use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{
        ByteStreamLe, ExactSizedStream, ReadStringError, TryParse, UnfinishedParsingError,
    },
    page::object::{
        HasObjectBase, ObjectBase, ObjectBaseParseError,
        header::{ObjectHeader, ObjectHeaderError},
    },
    unpack_bool_flag, unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum VoiceObjectParseError {
    Io(#[from] io::Error),
    Base(#[from] ObjectBaseParseError),
    Header(#[from] ObjectHeaderError),

    #[error("failed to read title string")]
    Title(#[source] ReadStringError),

    #[error("failed to read play time string")]
    PlayTime(#[source] ReadStringError),

    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct VoiceObject {
    object_base: ObjectBase,

    is_recorded: bool,

    title: Option<String>,
    play_time: Option<String>,

    attached_file_id: Option<u32>,
}

impl<R: Read + Seek> TryParse<R> for VoiceObject {
    type ParseError = VoiceObjectParseError;

    fn try_parse(stream: &mut R) -> Result<VoiceObject, VoiceObjectParseError> {
        let object_base = ObjectBase::try_parse(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 10)?;

        unpack_bool_flag!(header.property_flags_mut(), 0 => is_recorded);

        let field_flags = header.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => attached_file_id: stream.read_u32_le()?;

            1 => title:
                stream.read_short_u16_string().map_err(VoiceObjectParseError::Title)?;

            2 => play_time:
                stream.read_short_u16_string().map_err(VoiceObjectParseError::PlayTime)?;
        });

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(VoiceObject {
            object_base,
            is_recorded,
            title,
            play_time,
            attached_file_id,
        })
    }
}

impl HasObjectBase for VoiceObject {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}
