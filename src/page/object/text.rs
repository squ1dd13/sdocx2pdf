use std::io::{self, Read, Seek, SeekFrom};

use num::FromPrimitive;
use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ReadBitfieldError},
    page::object::{
        ConcreteInheritsObjectBase, InheritsObjectBase, ObjectBase,
        shape::{BorderType, ShapeObject},
        shape_base::ShapeBase,
    },
};

#[derive(Error, Debug)]
pub enum TextObjectParseError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error("invalid data type {0} for text object (should be 2)")]
    BadDataType(u16),

    #[error("failed to parse property flags")]
    PropertyFlags(#[source] ReadBitfieldError),

    #[error("failed to parse field check flags")]
    FieldCheckFlags(#[source] ReadBitfieldError),

    #[error("invalid border type {0}")]
    BadBorderType(u16),

    #[error("{0} byte(s) remain after parsing")]
    BytesRemain(u64),
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
        let start_offset = stream.stream_position()?;

        let size: u64 = stream.read_u32_le()?.into();

        // Subtract 4 because we already read a u32 size.
        let mut stream = stream.take(size - 4);

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

        let _property_flags = stream
            .read_variable_length_bitfield()
            .map_err(TextObjectParseError::PropertyFlags)?;

        let field_check_flags = stream
            .read_variable_length_bitfield()
            .map_err(TextObjectParseError::FieldCheckFlags)?;

        if flex_offset != 0 {
            stream.seek(SeekFrom::Start(start_offset + flex_offset))?;

            if field_check_flags & 2 != 0 {
                shape.data.border_colour = Some(stream.read_u32_le()?.to_le_bytes());
            }

            if field_check_flags & 4 != 0 {
                shape.data.border_width = Some(stream.read_f32_le()?);
            }

            if field_check_flags & 8 != 0 {
                let val = stream.read_u16_le()?;

                shape.data.border_type = Some(
                    BorderType::from_u16(val).ok_or(TextObjectParseError::BadBorderType(val))?,
                );
            }
        }

        if let remaining @ 1.. = stream.limit() {
            return Err(TextObjectParseError::BytesRemain(remaining));
        }

        Ok(TextObject { shape })
    }
}

impl InheritsObjectBase for TextObject {
    fn try_parse<T: crate::byte_stream::ByteStreamLe + std::io::Seek>(
        stream: &mut T,
        object_base: super::ObjectBase,
        child_count: u16,
    ) -> color_eyre::eyre::Result<TextObject> {
        let shape = ShapeObject::try_parse(stream, object_base, child_count)?;
        Ok(TextObject::try_parse(stream, shape)?)
    }

    fn object_base(&self) -> &super::ObjectBase {
        self.shape.object_base()
    }
}

impl ConcreteInheritsObjectBase for TextObject {}
