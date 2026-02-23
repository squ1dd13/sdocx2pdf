use crate::{
    OpaqueBytes,
    byte_stream::ByteStreamLe,
    page::{Point, Rect, object::line::LineObject},
};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};
use std::io::{Seek, SeekFrom};

mod line;
mod shape_base;

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
    /// (This seems intentional (?) in the JVM code; we see things
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
struct ObjectBase {
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

trait InheritsObjectBase: Sized {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> Result<Self>;
}

trait ConcreteInheritsObjectBase: InheritsObjectBase {}

impl ObjectBase {
    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<ObjectBase> {
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

    fn hash(&self) -> [u8; 32] {
        let s = format!("{}{}", self.uuid, self.modified_time.timestamp_micros());
        Sha256::digest(s.as_bytes()).into()
    }

    fn try_parse_wrapped<T: ByteStreamLe + Seek, I: ConcreteInheritsObjectBase>(
        stream: &mut T,
    ) -> Result<I> {
        let child_count = stream.read_u16_le()?;

        // This is the size of the `ObjectBase`, the inner object, and the hash.
        let total_size: u64 = stream.read_u32_le()?.into();
        let expected_end = stream.stream_position()? + total_size;

        let base = ObjectBase::try_parse(stream)?;
        let base_hash = base.hash();

        let parsed = I::try_parse(stream, base, child_count)?;

        let mut hash_read = [0_u8; 32];
        stream.read_exact(&mut hash_read)?;

        if base_hash != hash_read {
            eprintln!("Warning: Hash mismatch");
        } else {
            eprintln!("Hashes match!");
        }

        let here = stream.stream_position()?;

        if here != expected_end {
            eprintln!(
                "Warning: Object hash ended at {here}, not {expected_end}. Will `seek` to fix."
            );

            stream.seek(SeekFrom::Start(expected_end))?;
        } else {
            eprintln!("Object ended as expected");
        }

        Ok(parsed)
    }
}

#[derive(Debug)]
pub struct OpaqueObjectInner {
    child_count: u16,
    inner: OpaqueBytes,
}

impl DocObjectInner for OpaqueObjectInner {
    fn try_parse<T: ByteStreamLe>(stream: &mut T, child_count: u16) -> Result<OpaqueObjectInner> {
        Ok(OpaqueObjectInner {
            child_count,
            inner: OpaqueBytes::try_parse_inclusive(stream)?,
        })
    }
}

pub trait DocObjectInner: Sized + std::fmt::Debug {
    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T, child_count: u16) -> Result<Self>;
}

#[derive(Debug)]
pub struct ObjectBaseWrapper<I: DocObjectInner> {
    base: ObjectBase,
    inner: I,
}

impl<I: DocObjectInner> ObjectBaseWrapper<I> {
    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<ObjectBaseWrapper<I>> {
        let child_count = stream.read_u16_le()?;

        // This is the size of the `ObjectBase`, the inner object, and the hash.
        let total_size: u64 = stream.read_u32_le()?.into();
        let expected_end = stream.stream_position()? + total_size;

        let base = ObjectBase::try_parse(stream)?;
        let inner = I::try_parse(stream, child_count)?;

        let mut hash_read = [0_u8; 32];
        stream.read_exact(&mut hash_read)?;

        let hash_calculated = Sha256::digest(
            format!("{}{}", base.uuid, base.modified_time.timestamp_micros()).as_bytes(),
        );

        if hash_calculated[..] != hash_read {
            eprintln!("Warning: Hash mismatch");
        } else {
            eprintln!("Hashes match!");
        }

        let here = stream.stream_position()?;

        if here != expected_end {
            eprintln!(
                "Warning: Object hash ended at {here}, not {expected_end}. Will `seek` to fix."
            );

            stream.seek(SeekFrom::Start(expected_end))?;
        } else {
            eprintln!("Object ended as expected");
        }

        Ok(ObjectBaseWrapper { base, inner })
    }
}

// fixme: `ObjectBaseWrapper<OpaqueObjectInner>` is wrong for anything that is not a direct
// subclass of `WCon_ObjectBase`.

// todo: Remove boxes once everything is of a similar size.

#[derive(Debug)]
pub enum DocObject {
    /// `WCon_ObjectStroke`; extends `WCon_ObjectBase`
    Stroke {
        is_old_type: bool,
        object: ObjectBaseWrapper<OpaqueObjectInner>,
    },

    /// `WCon_ObjectTextBoxOrImage` (variant 1) extends `WCon_ObjectShape` (`Shape`)
    Text(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectTextBoxOrImage` (variant 0) extends `WCon_ObjectShape` (`Shape`)
    Image(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectContainer`; extends `WCon_ObjectBase`
    Container(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectShape`; extends `WCon_ObjectShapeBase`, which extends `WCon_ObjectBase`
    Shape(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectLine`; extends `WCon_ObjectShapeBase` (see `Shape`)
    Line(Box<LineObject>),

    /// `WCon_ObjectVoice`; extends `WCon_ObjectBase`
    Voice(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectFormula`; extends `WCon_ObjectBase`
    Formula(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectTable`; extends `WCon_ObjectBase`
    Table(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectWeb`; extends `WCon_ObjectBase`
    Web(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectPainting`; extends `WCon_ObjectBase`
    Painting(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectLink`; extends `WCon_ObjectBase`
    Link(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectMath`; extends `WCon_ObjectBase`
    Maths(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectPlot`; extends `WCon_ObjectBase`
    Plot(ObjectBaseWrapper<OpaqueObjectInner>),

    /// `WCon_ObjectUnknown`; extends `WCon_ObjectBase`
    Generic(ObjectBaseWrapper<OpaqueObjectInner>),
}

impl DocObject {
    pub fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<DocObject> {
        let object_type = stream.read_u8()?;

        eprintln!("Object type {object_type}");

        if object_type == 8 {
            return Ok(DocObject::Line(Box::new(ObjectBase::try_parse_wrapped(
                stream,
            )?)));
        }

        let object = ObjectBaseWrapper::try_parse(stream)?;

        Ok(match object_type {
            1 | 15 => DocObject::Stroke {
                is_old_type: object_type == 15,
                object,
            },

            2 => DocObject::Text(object),
            3 => DocObject::Image(object),
            4 => DocObject::Container(object),
            7 => DocObject::Shape(object),
            10 => DocObject::Voice(object),
            11 => DocObject::Formula(object),
            13 => DocObject::Web(object),
            14 => DocObject::Painting(object),
            17 => DocObject::Link(object),
            19 => DocObject::Generic(object),
            20 => DocObject::Plot(object),
            21 => DocObject::Maths(object),
            22 => DocObject::Table(object),

            unknown => return Err(eyre!("Unrecognised object type {unknown}")),
            // There is also an object type 100, for `WDocObjectStrokeGroup`.
            // As far as I can tell, this is not supposed to be written to disk, so we should
            // never read it.
        })
    }
}
