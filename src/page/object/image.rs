use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BlindWindow, ByteStreamLe, ExactSizedStream, ReadBitfieldError,
        TakeInclusiveLengthPrefixedError, UnfinishedParsingError,
    },
    page::{
        Rect,
        object::{
            ConcreteInheritsObjectBase, InheritsObjectBase,
            shape::{BorderType, InvalidBorderTypeError, ShapeObject},
        },
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
pub enum ImageObjectParseError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    BadSize(#[from] TakeInclusiveLengthPrefixedError),

    #[error("invalid data type {0} for image object (should be 3)")]
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
pub struct ImageObject {
    shape: ShapeObject,
}

impl ImageObject {
    fn try_parse(
        stream: &mut (impl ByteStreamLe + Seek),
        mut shape: ShapeObject,
    ) -> Result<ImageObject, ImageObjectParseError> {
        // See `TextObject` parsing.

        let mut stream: BlindWindow<_> = stream.take_inclusive_length_prefixed()?.into();

        match stream.read_u16_le()? {
            3 => (),
            bad => return Err(ImageObjectParseError::BadDataType(bad)),
        };

        let flex_offset: u64 = stream.read_u32_le()?.into();

        CheckedBitfield::try_parse(&mut stream)
            .map_err(ImageObjectParseError::PropertyFlags)?
            .ensure_none_set_unchecked()
            .map_err(ImageObjectParseError::UnhandledProperty)?;

        let stated_field_check_flags = CheckedBitfield::try_parse(&mut stream)
            .map_err(ImageObjectParseError::FieldCheckFlags)?;

        let mut field_check_flags = if flex_offset != 0 {
            stream.seek(io::SeekFrom::Start(flex_offset))?;
            stated_field_check_flags
        } else {
            CheckedBitfield::default()
        };

        unpack_field_flags!(field_check_flags, {
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

        field_check_flags
            .ensure_none_set_unchecked()
            .map_err(ImageObjectParseError::UnhandledField)?;

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

        stream.ensure_eof()?;

        Ok(ImageObject { shape })
    }
}

impl InheritsObjectBase for ImageObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: super::ObjectBase,
        child_count: u16,
    ) -> color_eyre::eyre::Result<ImageObject> {
        let shape = ShapeObject::try_parse_inner(stream, object_base, child_count, false)?;
        Ok(ImageObject::try_parse(stream, shape)?)
    }

    fn object_base(&self) -> &super::ObjectBase {
        self.shape.object_base()
    }
}

impl ConcreteInheritsObjectBase for ImageObject {}
