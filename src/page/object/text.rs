use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ExactSizedStream, TryParse, UnfinishedParsingError},
    page::object::{
        HasObjectBase, ObjectBase,
        header::{ObjectHeader, ObjectHeaderError},
        shape::{InvalidBorderTypeError, Shape, ShapeParseError},
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum TextParseError {
    Io(#[from] io::Error),
    Shape(#[from] ShapeParseError),
    Header(#[from] ObjectHeaderError),
    BadBorderType(#[from] InvalidBorderTypeError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
pub struct Text {
    shape: Shape,
}

impl<R: Read + Seek> TryParse<R> for Text {
    type ParseError = TextParseError;

    fn try_parse(stream: &mut R) -> Result<Text, TextParseError> {
        let mut shape = Shape::try_parse_as_base(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 2)?;

        let field_flags = header.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            1 => border_colour: stream.read_4_bytes()?;
            2 => border_width: stream.read_f32_le()?;
            3 => border_type: stream.read_u16_le()?.try_into()?;
        });

        shape.data.border_colour = border_colour;
        shape.data.border_width = border_width;
        shape.data.border_type = border_type;

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Text { shape })
    }
}

impl HasObjectBase for Text {
    fn object_base(&self) -> &ObjectBase {
        self.shape.object_base()
    }
}
