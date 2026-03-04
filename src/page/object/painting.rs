use std::io::{Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ExactSizedStream, TryParse, UnfinishedParsingError},
    page::{
        Rect,
        object::{
            base::{HasObjectBase, ObjectBase, ObjectBaseParseError},
            header::{ObjectHeader, ObjectHeaderError},
        },
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum PaintingParseError {
    Io(#[from] std::io::Error),
    Base(#[from] ObjectBaseParseError),
    Header(#[from] ObjectHeaderError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Painting {
    object_base: ObjectBase,

    attached_file_id: Option<u32>,
    attached_thumbnail_id: Option<u32>,

    crop_rect: Option<Rect>,
    original_rect: Option<Rect>,

    ratio: f32,
}

impl<R: Read + Seek> TryParse<R> for Painting {
    type ParseError = PaintingParseError;

    fn try_parse(stream: &mut R) -> Result<Painting, PaintingParseError> {
        let object_base = ObjectBase::try_parse(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 14)?;

        let field_flags = header.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => attached_file_id: stream.read_u32_le()?;
            1 => attached_thumbnail_id: stream.read_u32_le()?;
            2 => ratio: stream.read_f32_le()?, else 1.0;
            3 => crop_rect: Rect::try_parse_i32(&mut stream)?;
            4 => original_rect: Rect::try_parse_f64(&mut stream)?;
        });

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Painting {
            object_base,
            attached_file_id,
            attached_thumbnail_id,
            crop_rect,
            original_rect,
            ratio,
        })
    }
}

impl HasObjectBase for Painting {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}
