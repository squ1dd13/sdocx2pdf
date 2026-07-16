use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{BoundedStream, ByteStreamLe, UnfinishedParsingError},
    context::{DocumentContext, TryParseWithContext},
    page::object::{
        base::{HasObjectBase, ObjectBase},
        header::{FlagBlockError, ObjectHeaderError, try_parse_object_header},
        shape::{InvalidBorderTypeError, Shape, ShapeParseContext, ShapeParseError},
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum TextParseError {
    Io(#[from] io::Error),
    Shape(#[from] ShapeParseError),
    Header(#[from] ObjectHeaderError),
    FlagBlock(#[from] FlagBlockError),
    BadBorderType(#[from] InvalidBorderTypeError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
pub struct Text {
    shape: Shape,
}

impl Text {
    pub fn raw_string(&self) -> Option<&str> {
        self.shape.raw_text_string()
    }
}

impl<R: Read + Seek> TryParseWithContext<R, DocumentContext<'_, '_>> for Text {
    type ParseError = TextParseError;

    fn try_parse_with_ctx(
        stream: &mut R,
        &doc_ctx: &DocumentContext<'_, '_>,
    ) -> Result<Text, TextParseError> {
        let mut shape = Shape::try_parse_with_ctx(
            stream,
            &ShapeParseContext {
                is_shape_only: false,
                doc_ctx,
            },
        )?;

        let (mut flag_block, mut stream) = try_parse_object_header(stream, 2)?;

        let field_flags = flag_block.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            1 => border_colour: stream.read_4_bytes()?;
            2 => border_width: stream.read_f32_le()?;
            3 => border_type: stream.read_u16_le()?.try_into()?;
        });

        shape.shape_data.border_colour = border_colour;
        shape.shape_data.border_width = border_width;
        shape.shape_data.border_type = border_type;

        flag_block.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Text { shape })
    }
}

impl HasObjectBase for Text {
    fn object_base(&self) -> &ObjectBase {
        self.shape.object_base()
    }
}
