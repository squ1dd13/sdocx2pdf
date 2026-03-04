use std::io::{Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{
        ByteStreamLe, ExactSizedStream, ReadStringError, TryParse, UnfinishedParsingError,
    },
    page::object::{
        base::{HasObjectBase, ObjectBase, ObjectBaseParseError},
        header::{ObjectHeader, ObjectHeaderError},
    },
    unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum WebParseError {
    Io(#[from] std::io::Error),
    Base(#[from] ObjectBaseParseError),
    Header(#[from] ObjectHeaderError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("failed to read title string")]
    Title(#[source] ReadStringError),

    #[error("failed to read body string")]
    Body(#[source] ReadStringError),

    #[error("failed to read uri string")]
    Uri(#[source] ReadStringError),
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Web {
    object_base: ObjectBase,

    attached_html_file_id: Option<u32>,
    thumbnail_file_id: Option<u32>,
    body: Option<String>,
    title: Option<String>,
    uri: Option<String>,

    image_type_id: u32,

    version: Option<u32>,

    // todo: Find an enum for this.
    view_type: Option<u32>,
}

impl<R: Read + Seek> TryParse<R> for Web {
    type ParseError = WebParseError;

    fn try_parse(stream: &mut R) -> Result<Web, WebParseError> {
        let object_base = ObjectBase::try_parse(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 13)?;

        let field_flags = header.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => attached_html_file_id: stream.read_u32_le()?;
            1 => thumbnail_file_id: stream.read_u32_le()?;
            2 => body: stream.read_short_u16_string().map_err(WebParseError::Body)?;
            3 => title: stream.read_short_u16_string().map_err(WebParseError::Title)?;
            4 => uri: stream.read_short_u16_string().map_err(WebParseError::Uri)?;
        });

        let image_type_id = stream.read_u32_le()?;

        unpack_field_flags!(field_flags, {
            5 => version: stream.read_u32_le()?;
            6 => view_type: stream.read_u32_le()?;
        });

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Web {
            object_base,
            attached_html_file_id,
            thumbnail_file_id,
            body,
            title,
            uri,
            image_type_id,
            version,
            view_type,
        })
    }
}

impl HasObjectBase for Web {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}
