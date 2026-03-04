use crate::{
    OpaqueBytes,
    byte_stream::{ByteStreamLe, SeekableByteStreamLe, TryParse},
    page::{
        Point, Rect,
        object::{
            audio::{Audio, AudioParseError},
            image::{Image, ImageParseError},
            line::{Line, LineParseError},
            painting::{Painting, PaintingParseError},
            shape::{Shape, ShapeParseError},
            stroke::{Stroke, StrokeParseError},
            text::{Text, TextParseError},
            web::{Web, WebParseError},
        },
    },
};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek};
use thiserror::Error;

mod audio;
mod header;
mod image;
mod line;
mod painting;
mod shape;
mod shape_base;
mod shared;
mod stroke;
pub mod text;
mod text_core;
mod web;

#[derive(Debug, Default)]
struct DocBundle {
    strings: IndexMap<String, String>,
    integers: IndexMap<String, u32>,
    string_vecs: IndexMap<String, Vec<String>>,
    byte_vecs: IndexMap<String, OpaqueBytes>,
}

impl DocBundle {
    /// Like `read_short_u8_string`, but removes a single trailing `\0` from the string if
    /// it finds one.
    ///
    /// This is useful because the keys for these maps tend to end with a nul for some reason.
    /// (This seems intentional (?): in the JVM code, we see things
    /// like `"SPEN_SDK_KEY_SYSTEM_RESERVED_EXTRA_DATA\u0000"`.)
    fn read_short_u8_string_without_nul(stream: &mut impl ByteStreamLe) -> Result<String> {
        let mut s = stream.read_short_u8_string()?;

        if !s.is_empty() && s.ends_with('\0') {
            s.pop();
        }

        Ok(s)
    }

    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<DocBundle> {
        let map_presence_flags = stream.read_u8()?;

        let mut bundle = DocBundle::default();

        if map_presence_flags & 1 != 0 {
            let entry_count: usize = stream.read_u16_le()?.into();
            bundle.strings.reserve_exact(entry_count);

            for _ in 0..entry_count {
                bundle.strings.insert(
                    Self::read_short_u8_string_without_nul(stream)?,
                    stream.read_short_u16_string()?,
                );
            }
        }

        if map_presence_flags & 2 != 0 {
            let entry_count: usize = stream.read_u16_le()?.into();
            bundle.integers.reserve_exact(entry_count);

            for _ in 0..entry_count {
                bundle.integers.insert(
                    Self::read_short_u8_string_without_nul(stream)?,
                    stream.read_u32_le()?,
                );
            }
        }

        if map_presence_flags & 4 != 0 {
            let entry_count: usize = stream.read_u16_le()?.into();
            bundle.string_vecs.reserve_exact(entry_count);

            for _ in 0..entry_count {
                let key = Self::read_short_u8_string_without_nul(stream)?;

                let string_count: usize = stream.read_u16_le()?.into();
                let mut strings = Vec::with_capacity(string_count);

                for _ in 0..string_count {
                    strings.push(stream.read_short_u16_string()?);
                }

                bundle.string_vecs.insert(key, strings);
            }
        }

        if map_presence_flags & 8 != 0 {
            let entry_count: usize = stream.read_u16_le()?.into();
            bundle.byte_vecs.reserve_exact(entry_count);

            for _ in 0..entry_count {
                bundle.byte_vecs.insert(
                    Self::read_short_u8_string_without_nul(stream)?,
                    OpaqueBytes::try_parse_exclusive(stream)?,
                );
            }
        }

        Ok(bundle)
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ObjectBase {
    rotatable: bool,
    selectable: bool,
    movable: bool,
    visible: bool,
    replayable: bool,
    clippable: bool,
    is_template_object: bool,
    flippable: bool,
    has_been_to_att: bool,
    lock_state: bool,
    removable: bool,

    format_version: u32,
    uuid: String,
    modified_time: DateTime<Utc>,
    rect: Rect,
    timestamp_int: u32,
    resize_type: u8,

    rotation_degree: f32,
    unknown_somethings: Option<Vec<[u8; 16]>>,
    ao_info: Option<String>,
    sor_bundle: Option<DocBundle>,
    plugin_link: Option<String>,
    extra_bundle: Option<DocBundle>,
    attached_file_id: Option<u32>,
    min_width_height: Option<(f32, f32)>,
    max_width_height: Option<(f32, f32)>,
    append_time: Option<DateTime<Utc>>,
    owner_page_width_height: Option<(u32, u32)>,
    layout_type: u8,
    unknown_20: Option<[u8; 20]>,
    thumbnail_bind_id: Option<u32>,
    pivot: Option<Point>,
    group_id: Option<String>,
}

pub trait HasObjectBase {
    fn object_base(&self) -> &ObjectBase;
}

pub type ObjectBaseParseError = color_eyre::Report;

impl<R: Read + Seek> TryParse<R> for ObjectBase {
    type ParseError = ObjectBaseParseError;

    fn try_parse(stream: &mut R) -> Result<ObjectBase, ObjectBaseParseError> {
        let expected_end = {
            let start = stream.stream_position()?;
            let base_size: u64 = stream.read_u32_le()?.into();

            start + base_size
        };

        let data_type = stream.read_u16_le()?;

        if data_type != 0 {
            return Err(eyre!("Data type should be 0, not {data_type}"));
        }

        let flex_offset = stream.read_u32_le()?;
        let has_flex_data = flex_offset != 0;

        let property_flags = stream.read_variable_length_bitfield()?;

        let rotatable = property_flags & 1 != 0;
        let selectable = property_flags & 2 != 0;
        let movable = property_flags & 4 != 0;
        let visible = property_flags & 8 != 0;
        let replayable = property_flags & 16 != 0;
        let clippable = property_flags & 32 != 0;
        let is_template_object = property_flags & 64 != 0;
        let flippable = property_flags & 128 != 0;
        let has_been_to_att = property_flags & 256 != 0;
        let lock_state = property_flags & 512 != 0;

        // Note: This is encoded as its inverse.
        let removable = property_flags & 4096 == 0;

        // These are the "stated" field check flags because if the flex offset is zero, we treat
        // them all as unset.
        let stated_field_check_flags = stream.read_variable_length_bitfield()?;
        let format_version = stream.read_u32_le()?;

        if format_version > 5034 {
            eprintln!("Warning: Version {format_version} newer than expected (5034)");
        }

        let uuid = stream.read_short_u8_string()?;
        let modified_time = stream.read_timestamp()?;
        let rect = Rect::try_parse_f64(stream)?;
        let timestamp_int = stream.read_u32_le()?;
        let resizable_b = stream.read_u8()?;

        let field_check_flags = if has_flex_data {
            stated_field_check_flags
        } else {
            0
        };

        let rotation_degree = (field_check_flags & 1 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?
            .unwrap_or(0.);

        let unknown_somethings = if field_check_flags & 2 != 0 {
            let count = stream.read_u16_le()?;

            // JVM code skips over these once it knows how many there are.
            let mut somethings = Vec::with_capacity(count.into());

            for _ in 0..count {
                let mut bytes = [0_u8; 16];
                stream.read_exact(&mut bytes)?;
                somethings.push(bytes);
            }

            Some(somethings)
        } else {
            None
        };

        let ao_info = (field_check_flags & 4 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()?;

        let sor_bundle = (field_check_flags & 8 != 0)
            .then(|| DocBundle::try_parse(stream))
            .transpose()?;

        let plugin_link = (field_check_flags & 16 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()?;

        let extra_bundle = (field_check_flags & 32 != 0)
            .then(|| DocBundle::try_parse(stream))
            .transpose()?;

        let attached_file_id = (field_check_flags & 64 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let min_width_height = if field_check_flags & 128 != 0 {
            Some((stream.read_f32_le()?, stream.read_f32_le()?))
        } else {
            None
        };

        let max_width_height = if field_check_flags & 256 != 0 {
            Some((stream.read_f32_le()?, stream.read_f32_le()?))
        } else {
            None
        };

        let append_time = (field_check_flags & 8192 != 0)
            .then(|| stream.read_timestamp())
            .transpose()?;

        let owner_page_width_height = if field_check_flags & 16384 != 0 {
            Some((stream.read_u32_le()?, stream.read_u32_le()?))
        } else {
            None
        };

        let layout_type = (field_check_flags & 32768 != 0)
            .then(|| stream.read_u8())
            .transpose()?
            .unwrap_or(0);

        let unknown_20 = if field_check_flags & 65536 != 0 {
            let mut bytes = [0_u8; 20];
            stream.read_exact(&mut bytes)?;
            Some(bytes)
        } else {
            None
        };

        let thumbnail_bind_id = (field_check_flags & 131072 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let pivot = (field_check_flags & 262144 != 0)
            .then(|| Point::try_parse_f64(stream))
            .transpose()?;

        let group_id = (field_check_flags & 524288 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()?;

        let position_now = stream.stream_position()?;

        if position_now != expected_end {
            return Err(eyre!(
                "Position after parsing ObjectBase is {position_now}, \
            but it should be {expected_end}. Field check flags: {field_check_flags:#x}"
            ));
        }

        Ok(ObjectBase {
            rotatable,
            selectable,
            movable,
            visible,
            replayable,
            clippable,
            is_template_object,
            flippable,
            has_been_to_att,
            lock_state,
            removable,
            format_version,
            uuid,
            modified_time,
            rect,
            timestamp_int,
            resize_type: resizable_b,
            rotation_degree,
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
    pub fn hash(&self) -> [u8; 32] {
        let s = format!("{}{}", self.uuid, self.modified_time.timestamp_micros());
        Sha256::digest(s.as_bytes()).into()
    }
}

pub type OpaqueObjectParseError = color_eyre::Report;

#[derive(Debug)]
#[allow(dead_code)]
pub struct OpaqueObject {
    object_base: ObjectBase,
    inner: OpaqueBytes,
}

impl<R: Read + Seek> TryParse<R> for OpaqueObject {
    type ParseError = OpaqueObjectParseError;

    fn try_parse(reader: &mut R) -> Result<Self, OpaqueObjectParseError> {
        Ok(OpaqueObject {
            object_base: ObjectBase::try_parse(reader)?,
            inner: OpaqueBytes::try_parse_inclusive(reader)?,
        })
    }
}

impl HasObjectBase for OpaqueObject {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum DocObjectParseError {
    Io(#[from] std::io::Error),

    Image(#[from] ImageParseError),
    Line(#[from] LineParseError),
    Painting(#[from] PaintingParseError),
    Shape(#[from] ShapeParseError),
    Stroke(#[from] StrokeParseError),
    Text(#[from] TextParseError),
    Voice(#[from] AudioParseError),
    Web(#[from] WebParseError),

    Opaque(#[from] OpaqueObjectParseError),

    #[error("object type {0} is not supported")]
    BadType(u8),
}

#[derive(Debug)]
pub enum DocObject {
    /// `WCon_ObjectStroke`; extends `WCon_ObjectBase`
    Stroke(Box<Stroke>),

    /// `WCon_ObjectTextBoxOrImage` (my name; variant 1) extends `WCon_ObjectShape` (`Shape`)
    Text(Box<Text>),

    /// `WCon_ObjectTextBoxOrImage` (my name; variant 0) extends `WCon_ObjectShape` (`Shape`)
    Image(Box<Image>),

    /// `WCon_ObjectContainer`; extends `WCon_ObjectBase`
    Container(OpaqueObject),

    /// `WCon_ObjectShape`; extends `WCon_ObjectShapeBase`, which extends `WCon_ObjectBase`
    Shape(Box<Shape>),

    /// `WCon_ObjectLine`; extends `WCon_ObjectShapeBase` (see `Shape`)
    Line(Box<Line>),

    /// `WCon_ObjectVoice`; extends `WCon_ObjectBase`
    Audio(Box<Audio>),

    /// `WCon_ObjectFormula`; extends `WCon_ObjectBase`
    Formula(OpaqueObject),

    /// `WCon_ObjectTable`; extends `WCon_ObjectBase`
    Table(OpaqueObject),

    /// `WCon_ObjectWeb`; extends `WCon_ObjectBase`
    Web(Box<Web>),

    /// `WCon_ObjectPainting`; extends `WCon_ObjectBase`
    Painting(Box<Painting>),

    /// `WCon_ObjectLink`; extends `WCon_ObjectBase`
    Link(OpaqueObject),

    /// `WCon_ObjectMath`; extends `WCon_ObjectBase`
    Maths(OpaqueObject),

    /// `WCon_ObjectPlot`; extends `WCon_ObjectBase`
    Plot(OpaqueObject),

    /// `WCon_ObjectUnknown`; extends `WCon_ObjectBase`
    Generic(OpaqueObject),
}

impl DocObject {
    // We use dynamic dispatch for the stream because object parsing can be recursive, and we don't
    // want to end up with recursive stream types ("Take<&mut Take<&mut Take<...>>>").
    pub fn try_parse_with_type(
        mut stream: &mut dyn SeekableByteStreamLe,
        object_type: u8,
    ) -> Result<DocObject, DocObjectParseError> {
        // Because `dyn SeekableByteStreamLe` is not `Sized`:
        let stream = &mut stream;

        Ok(match object_type {
            1 => DocObject::Stroke(Box::new(TryParse::try_parse(stream)?)),
            2 => DocObject::Text(Box::new(TryParse::try_parse(stream)?)),
            3 => DocObject::Image(Box::new(TryParse::try_parse(stream)?)),
            7 => DocObject::Shape(Box::new(Shape::try_parse_as_final(stream)?)),
            8 => DocObject::Line(Box::new(TryParse::try_parse(stream)?)),
            10 => DocObject::Audio(Box::new(TryParse::try_parse(stream)?)),
            13 => DocObject::Web(Box::new(TryParse::try_parse(stream)?)),
            14 => DocObject::Painting(Box::new(TryParse::try_parse(stream)?)),

            _ => {
                let object = OpaqueObject::try_parse(stream)?;

                match object_type {
                    4 => DocObject::Container({
                        eprintln!("Warning: Containers are not yet supported");
                        object
                    }),
                    11 => DocObject::Formula({
                        eprintln!("Warning: Formulas are not yet supported");
                        object
                    }),
                    17 => DocObject::Link({
                        eprintln!("Warning: Links are not yet supported");
                        object
                    }),
                    19 => DocObject::Generic({
                        eprintln!("Warning: Generic objects are not yet supported");
                        object
                    }),
                    20 => DocObject::Plot({
                        eprintln!("Warning: Plots are not yet supported");
                        object
                    }),
                    21 => DocObject::Maths({
                        eprintln!("Warning: Maths objects are not yet supported");
                        object
                    }),
                    22 => DocObject::Table({
                        eprintln!("Warning: Tables are not yet supported");
                        object
                    }),

                    unknown => return Err(DocObjectParseError::BadType(unknown)),
                }
            }
        })
    }

    pub fn object_base(&self) -> &ObjectBase {
        match self {
            DocObject::Line(line_object) => line_object.object_base(),
            DocObject::Shape(shape_object) => shape_object.object_base(),
            DocObject::Stroke(stroke_object) => stroke_object.object_base(),
            DocObject::Text(text_object) => text_object.object_base(),
            DocObject::Image(image_object) => image_object.object_base(),
            DocObject::Audio(voice_object) => voice_object.object_base(),
            DocObject::Web(web_object) => web_object.object_base(),
            DocObject::Painting(painting_object) => painting_object.object_base(),

            DocObject::Container(object)
            | DocObject::Formula(object)
            | DocObject::Table(object)
            | DocObject::Link(object)
            | DocObject::Maths(object)
            | DocObject::Plot(object)
            | DocObject::Generic(object) => &object.object_base,
        }
    }
}
