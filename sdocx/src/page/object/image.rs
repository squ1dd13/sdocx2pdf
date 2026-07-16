use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{BoundedStream, ByteStreamLe, UnfinishedParsingError},
    context::{DocumentContext, TryParseWithContext},
    page::{
        Rect,
        object::{
            base::{HasObjectBase, ObjectBase},
            header::{FlagBlockError, ObjectHeaderError, try_parse_object_header},
            shape::{InvalidBorderTypeError, Shape, ShapeParseContext, ShapeParseError},
        },
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ImageParseError {
    Io(#[from] io::Error),
    Shape(#[from] ShapeParseError),
    Header(#[from] ObjectHeaderError),
    FlagBlock(#[from] FlagBlockError),
    BadBorderType(#[from] InvalidBorderTypeError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
pub struct Image {
    shape: Shape,
}

impl<R: Read + Seek> TryParseWithContext<R, DocumentContext<'_, '_>> for Image {
    type ParseError = ImageParseError;

    fn try_parse_with_ctx(
        stream: &mut R,
        &ctx: &DocumentContext<'_, '_>,
    ) -> Result<Image, ImageParseError> {
        let mut shape = Shape::try_parse_with_ctx(
            stream,
            &ShapeParseContext {
                is_shape_only: false,
                doc_ctx: ctx,
            },
        )?;

        let (mut flag_block, mut stream) = try_parse_object_header(stream, 3)?;

        let field_flags = flag_block.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            // (missing 0)
            1 => crop_rect: Rect::try_parse_i32(&mut stream)?;
            // (missing 2)
            3 => border_colour: stream.read_4_bytes()?;
            4 => border_width: stream.read_f32_le()?;
            5 => border_type: stream.read_u16_le()?.try_into()?;
            // (missing 6, 7, 8)
            9 => border_image_bind_id: stream.read_u32_le()?;
            10 => border_image_nine_patch_rect: Rect::try_parse_i32(&mut stream)?;
            11 => border_line_width: Rect::try_parse_f32(&mut stream)?;
            12 => border_image_nine_patch_width: stream.read_u32_le()?;
            // (missing 13, 14, 15, 16)
            17 => original_rect: Rect::try_parse_f64(&mut stream)?;
            18 => original_image_bind_id: stream.read_u32_le()?;
        });

        shape.image_data.crop_rect = crop_rect;

        shape.shape_data.border_colour = border_colour;
        shape.shape_data.border_width = border_width;
        shape.shape_data.border_type = border_type;

        shape.image_data.border_image_bind_id = border_image_bind_id;
        shape.image_data.border_image_nine_patch_rect = border_image_nine_patch_rect;
        shape.image_data.border_line_width = border_line_width;
        shape.image_data.border_image_nine_patch_width = border_image_nine_patch_width;

        shape.image_data.original_rect = original_rect;
        shape.image_data.original_image_id = original_image_bind_id;

        flag_block.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Image { shape })
    }
}

impl HasObjectBase for Image {
    fn object_base(&self) -> &ObjectBase {
        self.shape.object_base()
    }
}
