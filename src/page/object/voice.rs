use std::io::{self, Seek};

use thiserror::Error;

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BlindWindow, ByteStreamLe, ExactSizedStream, ReadBitfieldError, ReadStringError,
        TakeInclusiveLengthPrefixedError, UnfinishedParsingError,
    },
    page::object::{ConcreteInheritsObjectBase, InheritsObjectBase, ObjectBase},
    read_flags, unpack_bool_flag, unpack_field_flags,
};

#[derive(Error, Debug)]
pub enum VoiceObjectParseError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    BadSize(#[from] TakeInclusiveLengthPrefixedError),

    #[error("invalid data type {0} for voice object (should be 10)")]
    BadDataType(u16),

    #[error("failed to parse property flags")]
    PropertyFlags(#[source] ReadBitfieldError),

    #[error("one or more property bits were not handled")]
    UnhandledProperty(#[source] UnhandledBitsError),

    #[error("failed to parse field check flags")]
    FieldCheckFlags(#[source] ReadBitfieldError),

    #[error("one or more field check flags were not handled")]
    UnhandledField(#[source] UnhandledBitsError),

    #[error("failed to read title string")]
    Title(#[source] ReadStringError),

    #[error("failed to read play time string")]
    PlayTime(#[source] ReadStringError),

    #[error(transparent)]
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
pub struct VoiceObject {
    object_base: ObjectBase,

    is_recorded: bool,

    title: Option<String>,
    play_time: Option<String>,

    attached_file_id: Option<u32>,
}

impl VoiceObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
    ) -> Result<VoiceObject, VoiceObjectParseError> {
        let mut stream: BlindWindow<_> = stream.take_inclusive_length_prefixed()?.into();

        match stream.read_u16_le()? {
            10 => (),
            bad => return Err(VoiceObjectParseError::BadDataType(bad)),
        };

        let flex_offset: u64 = stream.read_u32_le()?.into();

        read_flags!(
            &mut stream,
            property_flags,
            VoiceObjectParseError::PropertyFlags,
            VoiceObjectParseError::UnhandledProperty,
            {
                unpack_bool_flag!(property_flags, 0 => is_recorded);
            }
        );

        read_flags!(
            &mut stream,
            field_check_flags,
            VoiceObjectParseError::FieldCheckFlags,
            VoiceObjectParseError::UnhandledField,
            {
                if flex_offset != 0 {
                    stream.seek(io::SeekFrom::Start(flex_offset))?;
                } else {
                    // Nullify the field check flags if there's no flex data.
                    field_check_flags.clear();
                }

                unpack_field_flags!(field_check_flags, {
                    0 => attached_file_id: stream.read_u32_le()?;

                    1 => title:
                        stream.read_short_u16_string().map_err(VoiceObjectParseError::Title)?;

                    2 => play_time:
                        stream.read_short_u16_string().map_err(VoiceObjectParseError::PlayTime)?;
                });
            }
        );

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

impl InheritsObjectBase for VoiceObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> color_eyre::eyre::Result<Self> {
        Ok(VoiceObject::try_parse(stream, object_base)?)
    }

    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}

impl ConcreteInheritsObjectBase for VoiceObject {}
