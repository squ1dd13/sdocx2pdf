use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ExactSizedStream, TryParse, UnfinishedParsingError},
    page::{
        Rect,
        object::{
            HasObjectBase, ObjectBase,
            header::{ObjectHeader, ObjectHeaderError},
            shape::{BorderType, InvalidBorderTypeError, ShapeObject, ShapeParseError},
        },
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ImageObjectParseError {
    Io(#[from] io::Error),
    Shape(#[from] ShapeParseError),
    Header(#[from] ObjectHeaderError),
    BadBorderType(#[from] InvalidBorderTypeError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
pub struct ImageObject {
    shape: ShapeObject,
}

impl<R: Read + Seek> TryParse<R> for ImageObject {
    type ParseError = ImageObjectParseError;

    fn try_parse(stream: &mut R) -> Result<ImageObject, ImageObjectParseError> {
        let mut shape = ShapeObject::try_parse_as_base(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 3)?;

        let field_flags = header.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            // (missing 0)

            1 => crop_rect: Rect::try_parse_i32(&mut stream)?;

            // (missing 2)

            3 => border_colour: stream.read_4_bytes()?;
            4 => border_width: stream.read_f32_le()?;
            5 => border_type: BorderType::try_from(stream.read_u16_le()?)?;

            // (missing 6, 7, 8)

            9 => border_image_bind_id: stream.read_u32_le()?;
            10 => border_image_nine_patch_rect: Rect::try_parse_i32(&mut stream)?;
            11 => border_line_width: Rect::try_parse_f32(&mut stream)?;
            12 => border_image_nine_patch_width: stream.read_u32_le()?;

            // (missing 13, 14, 15, 16)

            17 => original_rect: Rect::try_parse_f64(&mut stream)?;
            18 => original_image_bind_id: stream.read_u32_le()?;
        });

        shape.image.crop_rect = crop_rect;

        shape.data.border_colour = border_colour;
        shape.data.border_width = border_width;
        shape.data.border_type = border_type;

        shape.image.border_image_bind_id = border_image_bind_id;
        shape.image.border_image_nine_patch_rect = border_image_nine_patch_rect;
        shape.image.border_line_width = border_line_width;
        shape.image.border_image_nine_patch_width = border_image_nine_patch_width;

        shape.image.original_rect = original_rect;
        shape.image.original_image_id = original_image_bind_id;

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(ImageObject { shape })
    }
}

impl HasObjectBase for ImageObject {
    fn object_base(&self) -> &ObjectBase {
        self.shape.object_base()
    }
}
