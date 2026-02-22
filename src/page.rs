use crate::{OpaqueBytes, byte_stream::ByteStreamLe};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use indexmap::IndexMap;
use sha2::Digest;
use std::io::{Seek, SeekFrom};

#[derive(Debug)]
struct PointF64 {
    x: f64,
    y: f64,
}

impl PointF64 {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<PointF64> {
        Ok(PointF64 {
            x: stream.read_f64_le()?,
            y: stream.read_f64_le()?,
        })
    }
}

#[derive(Debug)]
struct RectF64 {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl RectF64 {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<RectF64> {
        Ok(RectF64 {
            left: stream.read_f64_le()?,
            top: stream.read_f64_le()?,
            right: stream.read_f64_le()?,
            bottom: stream.read_f64_le()?,
        })
    }
}

#[derive(Debug)]
struct PdfDataItem {
    bind_id: u32,
    page_index: u32,
    pdf_rect: RectF64,
}

impl PdfDataItem {
    fn try_parse<T: ByteStreamLe>(stream: &mut T, format_version: u32) -> Result<PdfDataItem> {
        Ok(PdfDataItem {
            bind_id: stream.read_u32_le()?,
            page_index: stream.read_u32_le()?,
            pdf_rect: if format_version < 2034 {
                RectF64::try_parse(stream)?
            } else {
                RectF64 {
                    left: stream.read_i32_le()?.into(),
                    top: stream.read_i32_le()?.into(),
                    right: stream.read_i32_le()?.into(),
                    bottom: stream.read_i32_le()?.into(),
                }
            },
        })
    }
}

#[derive(Debug)]
struct CanvasCacheEntry {
    file_id: u32,
    width: u32,
    height: u32,
    is_dark_mode: bool,
    background_colour: [u8; 4],
    version: [u32; 3],
    cache_version: u32,
    property: u32,
    locale_list_id: u32,
    system_font_path_hash: u32,
}

impl CanvasCacheEntry {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<CanvasCacheEntry> {
        Ok(CanvasCacheEntry {
            file_id: stream.read_u32_le()?,
            width: stream.read_u32_le()?,
            height: stream.read_u32_le()?,
            is_dark_mode: stream.read_u8()? == 1,
            background_colour: stream.read_u32_le()?.to_le_bytes(),
            version: [
                stream.read_u32_le()?,
                stream.read_u32_le()?,
                stream.read_u32_le()?,
            ],
            cache_version: stream.read_u32_le()?,
            property: stream.read_u32_le()?,
            locale_list_id: stream.read_u32_le()?,
            system_font_path_hash: stream.read_u32_le()?,
        })
    }
}

#[derive(Debug)]
struct CustomPageObject {
    object_type: u32,
    inner: OpaqueBytes,
}

impl CustomPageObject {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<CustomPageObject> {
        let object_type = stream.read_u32_le()?;

        Ok(CustomPageObject {
            object_type,
            inner: OpaqueBytes::try_parse_exclusive(stream)?,
        })
    }
}

#[derive(Debug, Default)]
struct DocBundle {
    strings: IndexMap<String, String>,
    integers: IndexMap<String, u32>,
    string_vecs: IndexMap<String, Vec<String>>,
    byte_vecs: IndexMap<String, OpaqueBytes>,
}

impl DocBundle {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<DocBundle> {
        let map_presence_flags = stream.read_u8()?;

        let mut bundle = DocBundle::default();

        if map_presence_flags & 1 != 0 {
            let entry_count: usize = stream.read_u16_le()?.into();
            bundle.strings.reserve(entry_count);

            for _ in 0..entry_count {
                bundle.strings.insert(
                    stream.read_short_u8_string()?,
                    stream.read_short_u16_string()?,
                );
            }
        }

        if map_presence_flags & 2 != 0 {
            let entry_count: usize = stream.read_u16_le()?.into();
            bundle.integers.reserve(entry_count);

            for _ in 0..entry_count {
                bundle
                    .integers
                    .insert(stream.read_short_u8_string()?, stream.read_u32_le()?);
            }
        }

        if map_presence_flags & 4 != 0 {
            let entry_count: usize = stream.read_u16_le()?.into();
            bundle.string_vecs.reserve(entry_count);

            for _ in 0..entry_count {
                let key = stream.read_short_u8_string()?;

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
            bundle.byte_vecs.reserve(entry_count);

            for _ in 0..entry_count {
                bundle.byte_vecs.insert(
                    stream.read_short_u8_string()?,
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
    rect: RectF64,
    timestamp_int: u32,
    resizable_b: u8,

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
    layout_type: u32,
    unknown_20: Option<[u8; 20]>,
    thumbnail_bind_id: Option<u32>,
    pivot: Option<PointF64>,
    group_id: Option<String>,
}

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
        let rect = RectF64::try_parse(stream)?;
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
            .then(|| stream.read_u32_le())
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
            .then(|| PointF64::try_parse(stream))
            .transpose()?;

        let group_id = (field_check_flags & 524288 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()?;

        let position_now = stream.stream_position()?;

        // Higher-level objects have data after the `ObjectBase`. For them to read this correctly,
        // the stream needs to be in exactly the right position.
        if position_now != expected_end {
            eprintln!(
                "Warning: Position after parsing ObjectBase is {position_now}, \
                 but {expected_end} expected. Will seek to correct this."
            );

            stream.seek(SeekFrom::Start(expected_end))?;
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
            resizable_b,
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

#[derive(Debug)]
struct OpaqueObjectInner {
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

trait DocObjectInner: Sized + std::fmt::Debug {
    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T, child_count: u16) -> Result<Self>;
}

#[derive(Debug)]
struct ObjectBaseWrapper<I: DocObjectInner> {
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

        let hash_calculated = sha2::Sha256::digest(
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

#[derive(Debug)]
enum DocObject {
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
    Line(ObjectBaseWrapper<OpaqueObjectInner>),

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
    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<DocObject> {
        let object_type = stream.read_u8()?;

        eprintln!("Object type {object_type}");

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
            8 => DocObject::Line(object),
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

#[derive(Debug)]
struct Layer {
    visible: bool,
    lock_state: bool,
    event_forwardable: bool,

    layer_id: u32,

    alpha: u8,
    background_colour: [u8; 4],
    name: Option<String>,
    uuid: Option<String>,
    modified_time: Option<DateTime<Utc>>,
    thumbnail_media_id: Option<u32>,
    shadow_effect: Option<OpaqueBytes>,

    objects: Vec<DocObject>,

    hash: [u8; 32],
}

impl Layer {
    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<Layer> {
        let data_size = stream.read_u32_le()?;
        let flex_offset: u64 = stream.read_u32_le()?.into();

        let property_flags = stream.read_variable_length_bitfield()?;
        let field_check_flags = stream.read_variable_length_bitfield()?;

        // The first property flag is for invisibility, so visibility is its inverse.
        let visible = property_flags & 1 == 0;
        let lock_state = property_flags & 4 != 0;
        let event_forwardable = property_flags & 2 != 0;

        let layer_id = stream.read_u32_le()?;

        stream.seek(SeekFrom::Start(flex_offset))?;

        let alpha = (field_check_flags & 1 != 0)
            .then(|| stream.read_u8())
            .transpose()?
            .unwrap_or(255);

        let background_colour = (field_check_flags & 2 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .map_or([0xff, 0xff, 0xff, 0xff], u32::to_le_bytes);

        let name = (field_check_flags & 4 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()?;

        let uuid = (field_check_flags & 8 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()?;

        let modified_time = (field_check_flags & 16 != 0)
            .then(|| stream.read_timestamp())
            .transpose()?;

        let thumbnail_media_id = (field_check_flags & 32 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let shadow_effect = (field_check_flags & 64 != 0)
            .then(|| OpaqueBytes::try_parse_exclusive(stream))
            .transpose()?;

        let objects = {
            let object_count: usize = stream.read_u32_le()?.try_into()?;

            let mut objects = Vec::with_capacity(object_count);

            for _ in 0..object_count {
                objects.push(DocObject::try_parse(stream)?);
            }

            objects
        };

        let mut hash = [0_u8; 32];
        stream.read_exact(&mut hash)?;

        Ok(Layer {
            visible,
            lock_state,
            event_forwardable,
            layer_id,
            alpha,
            background_colour,
            name,
            uuid,
            modified_time,
            thumbnail_media_id,
            shadow_effect,
            objects,
            hash,
        })
    }
}

#[derive(Debug)]
pub struct Page {
    is_text_only: bool,

    orientation: u32,
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    page_id: String,
    modified_time: DateTime<Utc>,
    format_version: u32,
    min_format_version: u32,

    drawn_rect: Option<RectF64>,
    tag_list: Option<Vec<String>>,
    template_uri: Option<String>,
    background_image_id: Option<i32>,
    background_image_mode: u32,
    background_colour: [u8; 4],
    background_width: u32,
    background_rotation: u32,
    pdf_data_items: Option<Vec<PdfDataItem>>,
    template_type: Option<u32>,
    canvas_cache_map: Vec<(u32, CanvasCacheEntry)>,
    imported_data_height: Option<u32>,
    theme: Option<u32>,
    recognised_data_modified_time: Option<DateTime<Utc>>,
    stroke_recognition_data: Option<Vec<OpaqueBytes>>,
    custom_objects: Vec<CustomPageObject>,

    hash: [u8; 32],

    layers: Vec<Layer>,
}

impl Page {
    const CLOSING_STRING: &str = "Page for SAMSUNG S-Pen SDK";

    /// Parses a single page using all of `stream`.
    ///
    /// `stream` must not have anything after the end of the page data, because this method
    /// seeks to the end of `stream` and expects it to match the correct format for a page.
    pub fn try_parse_full<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<Page> {
        let data_start_pos = stream.stream_position()?;
        let closing_string_size: i64 = Self::CLOSING_STRING.len().try_into()?;

        // Seek to where the closing string should begin.
        stream.seek(SeekFrom::End(-closing_string_size))?;

        let closing_string = stream.read_u8_string(Self::CLOSING_STRING.len())?;

        if closing_string != Self::CLOSING_STRING {
            return Err(eyre!(
                "Closing string '{closing_string}' does not match expected '{}'",
                Self::CLOSING_STRING
            ));
        }

        // Return to the beginning.
        stream.seek(SeekFrom::Start(data_start_pos))?;

        let page_size = stream.read_u32_le()?;
        let flex_data_offset: u64 = stream.read_u32_le()?.into();

        let property_flags = stream.read_variable_length_bitfield()?;
        let is_text_only = property_flags & 0x1 != 0;

        let field_check_flags = stream.read_variable_length_bitfield()?;

        // == "Fixed area" ==
        let orientation = stream.read_u32_le()?;
        let width = stream.read_u32_le()?;
        let height = stream.read_u32_le()?;
        let offset_x = stream.read_u32_le()?;
        let offset_y = stream.read_u32_le()?;
        let page_id = stream.read_short_u16_string()?;
        let modified_time = stream.read_timestamp()?;
        let format_version = stream.read_u32_le()?;
        let min_format_version = stream.read_u32_le()?;
        // == End ==

        stream.seek(SeekFrom::Start(flex_data_offset))?;

        // == "Flexible area" ==
        let drawn_rect = (field_check_flags & 1 != 0)
            .then(|| RectF64::try_parse(stream))
            .transpose()?;

        let tag_list: Option<Vec<String>> = if field_check_flags & 2 != 0 {
            let tag_count = stream.read_u16_le()?;

            Some(
                (0..tag_count)
                    .map(|_| stream.read_short_u16_string())
                    .collect::<Result<_>>()?,
            )
        } else {
            None
        };

        let template_uri = (field_check_flags & 4 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()?;

        let background_image_id = (field_check_flags & 8 != 0)
            .then(|| stream.read_i32_le())
            .transpose()?;

        let background_image_mode = (field_check_flags & 16 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .unwrap_or(0);

        let background_colour = (field_check_flags & 32 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .map_or([0xff, 0xff, 0xff, 0xff], u32::to_le_bytes);

        let background_width = (field_check_flags & 64 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .unwrap_or(0);

        let background_rotation = (field_check_flags & 128 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .unwrap_or(0);

        let pdf_data_items: Option<Vec<PdfDataItem>> = if field_check_flags & 256 != 0 {
            let item_count = stream.read_u16_le()?;

            let mut items = Vec::with_capacity(item_count.into());

            for _ in 0..item_count {
                items.push(PdfDataItem::try_parse(stream, format_version)?);
            }

            Some(items)
        } else {
            None
        };

        let template_type = (field_check_flags & 512 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        // The app uses a `LinkedHashMap` here, so entry order must be important.
        // Since we are unlikely to use this for much, a `Vec` is fine in place of a real map.
        let mut canvas_cache_map: Vec<(u32, CanvasCacheEntry)> = vec![];

        if field_check_flags & 1024 != 0 {
            let entry_count: i64 = stream.read_u32_le()?.into();
            let entry_size: i64 = stream.read_u16_le()?.into();

            if entry_size == 49 {
                canvas_cache_map.reserve(entry_count.try_into()?);

                for _ in 0..entry_count {
                    let key = stream.read_u32_le()?;
                    let entry = CanvasCacheEntry::try_parse(stream)?;

                    canvas_cache_map.push((key, entry));
                }
            } else {
                eprintln!("Skipping canvas cache map: entry size is {entry_size}, not 49.");
                stream.seek_relative(entry_count * entry_size)?;
            }
        }

        let imported_data_height = (field_check_flags & 2048 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let theme = (field_check_flags & 4096 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let recognised_data_modified_time = (field_check_flags & 32768 != 0)
            .then(|| stream.read_timestamp())
            .transpose()?;

        let stroke_recognition_data: Option<Vec<OpaqueBytes>> = if field_check_flags & 65536 != 0 {
            let entry_count = stream.read_u32_le()?;

            let mut entries = Vec::with_capacity(entry_count as usize);

            for _ in 0..entry_count {
                entries.push(OpaqueBytes::try_parse_exclusive(stream)?);
            }

            Some(entries)
        } else {
            None
        };

        let mut custom_objects: Vec<CustomPageObject> = vec![];

        if field_check_flags & 262144 != 0 {
            let custom_object_count: usize = stream.read_u32_le()?.try_into()?;
            custom_objects.reserve(custom_object_count);

            for _ in 0..custom_object_count {
                custom_objects.push(CustomPageObject::try_parse(stream)?);
            }
        }

        // == End flexible ==

        // fixme: The hash could be read at basically any point, since we seek back after.
        // Not sure why it's done here.
        let hash = {
            let pos = stream.stream_position()?;

            // The hash comes before the closing string, so seek before both.
            stream.seek(SeekFrom::End(-closing_string_size - 32))?;

            let mut hash = [0_u8; 32];
            stream.read_exact(&mut hash)?;

            // Return to where we were before.
            stream.seek(SeekFrom::Start(pos))?;

            hash
        };

        let layer_count: usize = stream.read_u16_le()?.into();
        let current_layer_index = stream.read_u16_le()?;

        let mut layers = Vec::with_capacity(layer_count);

        for _ in 0..layer_count {
            layers.push(Layer::try_parse(stream)?);
        }

        let mut remaining_bytes = vec![];
        stream.read_to_end(&mut remaining_bytes)?;

        let expected_remaining_count: usize = (32 + closing_string_size).try_into()?;

        if remaining_bytes.len() != expected_remaining_count {
            return Err(eyre!(
                "Wrong number of bytes remaining: found {}, not {}",
                remaining_bytes.len(),
                expected_remaining_count
            ));
        }

        Ok(Page {
            is_text_only,
            orientation,
            width,
            height,
            offset_x,
            offset_y,
            page_id,
            modified_time,
            format_version,
            min_format_version,
            drawn_rect,
            tag_list,
            template_uri,
            background_image_id,
            background_image_mode,
            background_colour,
            background_width,
            background_rotation,
            pdf_data_items,
            template_type,
            canvas_cache_map,
            imported_data_height,
            theme,
            recognised_data_modified_time,
            stroke_recognition_data,
            custom_objects,
            hash,
            layers,
        })
    }
}
