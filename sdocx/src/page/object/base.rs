use std::{
    collections::HashMap,
    io::{Read, Seek},
};

use chrono::{DateTime, Utc};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    OpaqueBytes, OpaqueBytesParseError,
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BoundedStream, ByteStreamLe, ReadStringError, ReadTimestampError, TryParse,
        UnfinishedParsingError,
    },
    impl_try_from_for_optional_from,
    page::{
        Point, Rect,
        object::header::{ObjectHeader, ObjectHeaderError},
    },
    read_size_and_map, read_size_and_vec, unpack_bool_flags, unpack_field_flags,
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum BundleParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
    OpaqueBytes(#[from] OpaqueBytesParseError),
    UnhandledFlags(#[from] UnhandledBitsError),
}

#[derive(Debug, Default)]
#[expect(dead_code)]
struct Bundle {
    strings: HashMap<String, String>,
    integers: HashMap<String, u32>,
    string_vecs: HashMap<String, Vec<String>>,
    byte_vecs: HashMap<String, OpaqueBytes>,
}

impl Bundle {
    /// Like `read_short_u8_string`, but removes a single trailing `\0` from the string if
    /// it finds one.
    ///
    /// This is useful because the keys for these maps tend to end with a nul for some reason.
    /// (This seems intentional (?): in the JVM code, we see things
    /// like `"SPEN_SDK_KEY_SYSTEM_RESERVED_EXTRA_DATA\u0000"`.)
    fn read_short_u8_string_without_nul(stream: &mut impl Read) -> Result<String, ReadStringError> {
        let mut s = stream.read_short_u8_string()?;

        if !s.is_empty() && s.ends_with('\0') {
            s.pop();
        }

        Ok(s)
    }
}

impl<R: Read> TryParse<R> for Bundle {
    type ParseError = BundleParseError;

    fn try_parse(stream: &mut R) -> Result<Bundle, BundleParseError> {
        let mut map_presence_flags: CheckedBitfield = stream.read_u8()?.into();

        unpack_field_flags!(map_presence_flags, {
            0 => strings: read_size_and_map!(stream, u16, (
                Bundle::read_short_u8_string_without_nul(stream)?,
                stream.read_short_u16_string()?,
            ));

            1 => integers: read_size_and_map!(stream, u16, (
                Bundle::read_short_u8_string_without_nul(stream)?,
                stream.read_u32_le()?,
            ));

            2 => string_vecs: read_size_and_map!(stream, u16, (
                Bundle::read_short_u8_string_without_nul(stream)?,
                read_size_and_vec!(stream, u16, stream.read_short_u16_string()?),
            ));

            3 => byte_vecs: read_size_and_map!(stream, u16, (
                Bundle::read_short_u8_string_without_nul(stream)?,
                OpaqueBytes::try_parse_exclusive(stream)?,
            ));
        });

        map_presence_flags.ensure_none_set_unchecked()?;

        Ok(Bundle {
            strings: strings.unwrap_or_default(),
            integers: integers.unwrap_or_default(),
            string_vecs: string_vecs.unwrap_or_default(),
            byte_vecs: byte_vecs.unwrap_or_default(),
        })
    }
}

#[derive(Debug, FromPrimitive)]
pub enum LayoutType {
    /// `LAYOUT_NORMAL`
    Normal = 0,
    /// `LAYOUT_FLOW`
    Flow = 1,
    /// `LAYOUT_BLOCK`
    Block = 2,
    /// `LAYOUT_UNDEFINED`
    Undefined = 3,
}

impl_try_from_for_optional_from!(LayoutType, u8, from_u8, pub InvalidLayoutTypeError);

#[derive(Debug, FromPrimitive)]
pub enum ResizeMode {
    /// `RESIZE_OPTION_FREE`
    Free = 0,
    /// `RESIZE_OPTION_KEEP_RATIO`
    KeepRatio = 1,
    /// `RESIZE_OPTION_DISABLE`
    Disabled = 2,
}

impl_try_from_for_optional_from!(ResizeMode, u8, from_u8, pub InvalidResizeModeError);

#[derive(Debug)]
#[expect(dead_code)]
pub struct ObjectBase {
    is_rotatable: bool,
    is_selectable: bool,
    is_movable: bool,
    is_visible: bool,
    is_replayable: bool,
    is_clippable: bool,
    is_template_object: bool,
    is_flippable: bool,
    has_been_to_att: bool,
    is_locked: bool,
    is_removable: bool,

    pub format_version: u32,
    uuid: String,
    modified_time: DateTime<Utc>,
    rect: Rect,
    timestamp_int: u32,
    resize_mode: ResizeMode,

    angle: Option<f32>,
    unknown_somethings: Option<Vec<[u8; 16]>>,
    ao_info: Option<String>,
    sor_bundle: Option<Bundle>,
    plugin_link: Option<String>,
    extra_bundle: Option<Bundle>,
    attached_file_id: Option<u32>,
    min_width_height: Option<(f32, f32)>,
    max_width_height: Option<(f32, f32)>,
    append_time: Option<DateTime<Utc>>,
    owner_page_width_height: Option<(u32, u32)>,
    layout_type: Option<LayoutType>,
    unknown_20: Option<[u8; 20]>,
    thumbnail_bind_id: Option<u32>,
    pivot: Option<Point>,
    group_id: Option<String>,
}

pub trait HasObjectBase {
    fn object_base(&self) -> &ObjectBase;
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ObjectBaseParseError {
    Io(#[from] std::io::Error),
    Header(#[from] ObjectHeaderError),
    Bundle(#[from] BundleParseError),
    Timestamp(#[from] ReadTimestampError),
    String(#[from] ReadStringError),
    LayoutType(#[from] InvalidLayoutTypeError),
    ResizeMode(#[from] InvalidResizeModeError),
    Unfinished(#[from] UnfinishedParsingError),
}

impl<R: Read + Seek> TryParse<R> for ObjectBase {
    type ParseError = ObjectBaseParseError;

    fn try_parse(stream: &mut R) -> Result<ObjectBase, ObjectBaseParseError> {
        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 0)?;

        let property_flags = header.property_flags_mut();

        unpack_bool_flags!(property_flags, {
            0 => is_rotatable;
            1 => is_selectable;
            2 => is_movable;
            3 => is_visible;
            4 => is_replayable;
            5 => is_clippable;
            6 => is_template_object;
            7 => is_flippable;
            8 => has_been_to_att;
            9 => is_locked;
            // missing 10, 11
            12 => !is_removable;
        });

        let format_version = stream.read_u32_le()?;
        let uuid = stream.read_short_u8_string()?;
        let modified_time = stream.read_timestamp()?;
        let rect = Rect::try_parse_f64(&mut stream)?;
        let timestamp_int = stream.read_u32_le()?;
        let resize_mode: ResizeMode = stream.read_u8()?.try_into()?;

        let field_flags = header.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => angle: stream.read_f32_le()?;

            1 => unknown_somethings: read_size_and_vec!(stream, u16, {
                let mut bytes = [0_u8; 16];
                stream.read_exact(&mut bytes)?;
                bytes
            });

            2 => ao_info: stream.read_short_u16_string()?;
            3 => sor_bundle: Bundle::try_parse(&mut stream)?;
            4 => plugin_link: stream.read_short_u16_string()?;
            5 => extra_bundle: Bundle::try_parse(&mut stream)?;
            6 => attached_file_id: stream.read_u32_le()?;
            7 => min_width_height: (stream.read_f32_le()?, stream.read_f32_le()?);
            8 => max_width_height: (stream.read_f32_le()?, stream.read_f32_le()?);

            // missing 9, 10, 11, 12

            13 => append_time: stream.read_timestamp()?;
            14 => owner_page_width_height: (stream.read_u32_le()?, stream.read_u32_le()?);
            15 => layout_type: stream.read_u8()?.try_into()?;

            16 => unknown_20: {
                let mut bytes = [0_u8; 20];
                stream.read_exact(&mut bytes)?;
                bytes
            };

            17 => thumbnail_bind_id: stream.read_u32_le()?;
            18 => pivot: Point::try_parse_f64(&mut stream)?;
            19 => group_id: stream.read_short_u16_string()?;
        });

        if unknown_somethings.is_some() || unknown_20.is_some() {
            eprintln!(
                "Warning: Read one or more unknown fields: {:?}/{:?}",
                unknown_somethings, unknown_20
            );
        }

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(ObjectBase {
            is_rotatable,
            is_selectable,
            is_movable,
            is_visible,
            is_replayable,
            is_clippable,
            is_template_object,
            is_flippable,
            has_been_to_att,
            is_locked,
            is_removable,
            format_version,
            uuid,
            modified_time,
            rect,
            timestamp_int,
            resize_mode,
            angle,
            unknown_somethings,
            ao_info,
            sor_bundle,
            plugin_link,
            extra_bundle,
            attached_file_id,
            min_width_height,
            max_width_height,
            append_time,
            owner_page_width_height,
            layout_type,
            unknown_20,
            thumbnail_bind_id,
            pivot,
            group_id,
        })
    }
}

impl ObjectBase {
    pub fn compute_hash(&self) -> [u8; 32] {
        let s = format!("{}{}", self.uuid, self.modified_time.timestamp_micros());
        Sha256::digest(s.as_bytes()).into()
    }

    pub fn uuid(&self) -> &str {
        &self.uuid
    }
}
