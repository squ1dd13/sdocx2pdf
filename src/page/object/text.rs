use std::io::{self, Read, Seek, SeekFrom};

use num::FromPrimitive;
use thiserror::Error;

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BlindWindow, ByteStreamLe, ExactSizedStream, ReadBitfieldError,
        TakeInclusiveLengthPrefixedError, UnfinishedParsingError,
    },
    page::object::{
        ConcreteInheritsObjectBase, InheritsObjectBase, ObjectBase,
        shape::{BorderType, InvalidBorderTypeError, ShapeObject},
        shape_base::ShapeBase,
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
pub enum TextObjectParseError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    BadSize(#[from] TakeInclusiveLengthPrefixedError),

    #[error("invalid data type {0} for text object (should be 2)")]
    BadDataType(u16),

    #[error("failed to parse property flags")]
    PropertyFlags(#[source] ReadBitfieldError),

    #[error("one or more property bits were not handled")]
    UnhandledProperty(#[source] UnhandledBitsError),

    #[error("failed to parse field check flags")]
    FieldCheckFlags(#[source] ReadBitfieldError),

    #[error("one or more field check flags were not handled")]
    UnhandledField(#[source] UnhandledBitsError),

    #[error(transparent)]
    BadBorderType(#[from] InvalidBorderTypeError),

    #[error(transparent)]
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
pub struct TextObject {
    shape: ShapeObject,
}

impl TextObject {
    fn try_parse(
        stream: &mut (impl ByteStreamLe + Seek),
        mut shape: ShapeObject,
    ) -> Result<TextObject, TextObjectParseError> {
        let mut stream: BlindWindow<_> = stream.take_inclusive_length_prefixed()?.into();

        #[allow(
            irrefutable_let_patterns,
            reason = "https://github.com/rust-lang/rust/pull/146832"
        )]
        if let data_type = stream.read_u16_le()?
            && data_type != 2
        {
            return Err(TextObjectParseError::BadDataType(data_type));
        }

        let flex_offset: u64 = stream.read_u32_le()?.into();

        // There is a property flags field, but it should just be zero.
        CheckedBitfield::try_parse(&mut stream)
            .map_err(TextObjectParseError::PropertyFlags)?
            .ensure_none_set_unchecked()
            .map_err(TextObjectParseError::UnhandledProperty)?;

        let stated_field_check_flags = CheckedBitfield::try_parse(&mut stream)
            .map_err(TextObjectParseError::FieldCheckFlags)?;

        let mut field_check_flags = if flex_offset != 0 {
            stream.seek(SeekFrom::Start(flex_offset))?;
            stated_field_check_flags
        } else {
            CheckedBitfield::default()
        };

        unpack_field_flags!(field_check_flags, {
            1 => border_colour: stream.read_4_bytes()?;
            2 => border_width: stream.read_f32_le()?;
            3 => border_type: BorderType::try_from(stream.read_u16_le()?)?;
        });

        shape.data.border_colour = border_colour;
        shape.data.border_width = border_width;
        shape.data.border_type = border_type;

        field_check_flags
            .ensure_none_set_unchecked()
            .map_err(TextObjectParseError::UnhandledField)?;

        stream.ensure_eof()?;

        Ok(TextObject { shape })
    }

    pub fn try_parse_standalone(
        stream: &mut (impl ByteStreamLe + Seek),
    ) -> color_eyre::Result<TextObject> {
        ObjectBase::try_parse_inheritor(stream, 0)
    }
}

impl InheritsObjectBase for TextObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> color_eyre::eyre::Result<TextObject> {
        let shape = ShapeObject::try_parse_inner(stream, object_base, child_count, false)?;
        Ok(TextObject::try_parse(stream, shape)?)
    }

    fn object_base(&self) -> &ObjectBase {
        self.shape.object_base()
    }
}

impl ConcreteInheritsObjectBase for TextObject {}
