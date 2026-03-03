use std::io::{Seek, SeekFrom};

use thiserror::Error;

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BlindWindow, ByteStreamLe, ExactSizedStream, ReadBitfieldError, ReadStringError,
        TakeInclusiveLengthPrefixedError, UnfinishedParsingError,
    },
    page::object::{ConcreteInheritsObjectBase, InheritsObjectBase, ObjectBase},
    unpack_field_flags,
};

#[derive(Error, Debug)]
pub enum WebObjectParseError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    BadSize(#[from] TakeInclusiveLengthPrefixedError),

    #[error("invalid data type {0} for web object (should be 13)")]
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

    #[error("failed to read body string")]
    Body(#[source] ReadStringError),

    #[error("failed to read uri string")]
    Uri(#[source] ReadStringError),

    #[error(transparent)]
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
pub struct WebObject {
    base: ObjectBase,

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

impl WebObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        base: ObjectBase,
    ) -> Result<WebObject, WebObjectParseError> {
        let mut stream: BlindWindow<_> = stream.take_inclusive_length_prefixed()?.into();

        match stream.read_u16_le()? {
            13 => (),
            bad => return Err(WebObjectParseError::BadDataType(bad)),
        }

        let flex_offset: u64 = stream.read_u32_le()?.into();

        // No property flags should be set.
        CheckedBitfield::try_parse(&mut stream)
            .map_err(WebObjectParseError::PropertyFlags)?
            .ensure_none_set_unchecked()
            .map_err(WebObjectParseError::UnhandledProperty)?;

        let stated_field_check_flags = CheckedBitfield::try_parse(&mut stream)
            .map_err(WebObjectParseError::FieldCheckFlags)?;

        let mut field_check_flags = if flex_offset != 0 {
            stream.seek(SeekFrom::Start(flex_offset))?;
            stated_field_check_flags
        } else {
            CheckedBitfield::default()
        };

        unpack_field_flags!(field_check_flags, {
            0 => attached_html_file_id: stream.read_u32_le()?;
            1 => thumbnail_file_id: stream.read_u32_le()?;
            2 => body: stream.read_short_u16_string().map_err(WebObjectParseError::Body)?;
            3 => title: stream.read_short_u16_string().map_err(WebObjectParseError::Title)?;
            4 => uri: stream.read_short_u16_string().map_err(WebObjectParseError::Uri)?;
        });

        let image_type_id = stream.read_u32_le()?;

        unpack_field_flags!(field_check_flags, {
            5 => version: stream.read_u32_le()?;
            6 => view_type: stream.read_u32_le()?;
        });

        field_check_flags
            .ensure_none_set_unchecked()
            .map_err(WebObjectParseError::UnhandledField)?;

        stream.ensure_eof()?;

        Ok(WebObject {
            base,
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

impl InheritsObjectBase for WebObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> color_eyre::eyre::Result<Self> {
        Ok(WebObject::try_parse(stream, object_base)?)
    }

    fn object_base(&self) -> &ObjectBase {
        &self.base
    }
}

impl ConcreteInheritsObjectBase for WebObject {}
