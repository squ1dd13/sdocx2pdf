use crate::{
    OpaqueBytes,
    byte_stream::{ByteStreamLe, ExactSizedStream, TryParse},
    impl_try_from_for_optional_from,
    page::{
        header::{CanvasCacheEntry, CustomPageObject, PdfDataItem},
        object::DocObject,
    },
};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{self, Read, Seek, SeekFrom};

mod header;
pub mod object;

#[derive(Debug)]
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn try_parse_f64<T: ByteStreamLe>(stream: &mut T) -> io::Result<Point> {
        Ok(Point {
            x: stream.read_f64_le()?,
            y: stream.read_f64_le()?,
        })
    }

    fn try_parse_f32<T: ByteStreamLe>(stream: &mut T) -> io::Result<Point> {
        Ok(Point {
            x: stream.read_f32_le()?.into(),
            y: stream.read_f32_le()?.into(),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[expect(dead_code)]
pub struct Rect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Rect {
    fn try_parse_f64<T: ByteStreamLe>(stream: &mut T) -> io::Result<Rect> {
        Ok(Rect {
            left: stream.read_f64_le()?,
            top: stream.read_f64_le()?,
            right: stream.read_f64_le()?,
            bottom: stream.read_f64_le()?,
        })
    }

    fn try_parse_f32<T: ByteStreamLe>(stream: &mut T) -> io::Result<Rect> {
        Ok(Rect {
            left: stream.read_f32_le()?.into(),
            top: stream.read_f32_le()?.into(),
            right: stream.read_f32_le()?.into(),
            bottom: stream.read_f32_le()?.into(),
        })
    }

    fn try_parse_i32<T: ByteStreamLe>(stream: &mut T) -> io::Result<Rect> {
        Ok(Rect {
            left: stream.read_i32_le()?.into(),
            top: stream.read_i32_le()?.into(),
            right: stream.read_i32_le()?.into(),
            bottom: stream.read_i32_le()?.into(),
        })
    }
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Layer {
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

    pub objects: Vec<DocObject>,

    hash: [u8; 32],
}

impl Layer {
    fn try_parse_object<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<DocObject> {
        let object_type = stream.read_u8()?;
        let child_count = stream.read_u16_le()?;

        if child_count != 0 {
            return Err(eyre!("child count {child_count} > 0"));
        }

        let mut stream = stream.take_exclusive_length_prefixed()?;

        let doc_object = DocObject::try_parse_with_type(&mut stream, object_type)?;

        let mut hash_read = [0_u8; 32];
        stream.read_exact(&mut hash_read)?;

        if doc_object.object_base().hash() != hash_read {
            if object_type != 7 {
                return Err(eyre!("doc object hash mismatch (type {object_type})"));
            }

            eprintln!("Warning: Ignoring hash and size mismatch for shape object");
            stream.seek_relative(4)?;
        }

        stream.ensure_eof()?;

        Ok(doc_object)
    }

    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<Layer> {
        let _data_size = stream.read_u32_le()?;
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
                objects.push(Layer::try_parse_object(stream)?);
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

#[derive(Debug, FromPrimitive)]
pub enum TemplateType {
    /// `TYPE_NONE`
    None = 0,
    /// `TYPE_NARROW_LINE`
    NarrowLine = 1,
    /// `TYPE_MEDIUM_LINE`
    MediumLine = 2,
    /// `TYPE_WIDE_LINE`
    WideLine = 3,
    /// `TYPE_NARROW_GRID`
    NarrowGrid = 4,
    /// `TYPE_MEDIUM_GRID`
    MediumGrid = 5,
    /// `TYPE_WIDE_GRID`
    WideGrid = 6,
    /// `TYPE_NARROW_DOT`
    NarrowDot = 7,
    /// `TYPE_MEDIUM_DOT`
    MediumDot = 8,
    /// `TYPE_WIDE_DOT`
    WideDot = 9,
    /// `TYPE_TODO`
    Todo = 10,
    /// `TYPE_OXFORD_PAPER`
    OxfordPaper = 11,
    /// `TYPE_CUSTOM`
    Custom = 12,
    /// `TYPE_WEEKLY`
    Weekly = 13,
    /// `TYPE_MONTHLY`
    Monthly = 14,
    /// `TYPE_MANUSCRIPT`
    Manuscript = 15,
    /// `TYPE_PDF`
    Pdf = 16,
}

impl_try_from_for_optional_from!(TemplateType, u32, from_u32, pub InvalidTemplateTypeError);

#[derive(Debug, FromPrimitive, Default)]
pub enum BackgroundImageMode {
    /// `BACKGROUND_IMAGE_MODE_CENTER`
    #[default]
    Centre = 0,
    /// `BACKGROUND_IMAGE_MODE_STRETCH`
    Stretch = 1,
    /// `BACKGROUND_IMAGE_MODE_FIT`
    Fit = 2,
    /// `BACKGROUND_IMAGE_MODE_TILE`
    Tile = 3,
}

impl_try_from_for_optional_from!(
    BackgroundImageMode,
    u32,
    from_u32,
    pub InvalidBackgroundImageModeError
);

#[derive(Debug)]
#[expect(dead_code)]
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

    drawn_rect: Option<Rect>,
    tag_list: Option<Vec<String>>,
    template_uri: Option<String>,
    background_image_id: Option<i32>,
    background_image_mode: BackgroundImageMode,
    background_colour: [u8; 4],
    background_width: u32,
    background_rotation: u32,
    pdf_data_items: Option<Vec<PdfDataItem>>,
    template_type: Option<TemplateType>,
    canvas_cache_map: Vec<(u32, CanvasCacheEntry)>,
    imported_data_height: Option<u32>,
    theme: Option<u32>,
    recognised_data_modified_time: Option<DateTime<Utc>>,
    stroke_recognition_data: Option<Vec<OpaqueBytes>>,
    custom_objects: Vec<CustomPageObject>,
    current_layer_index: u16,

    hash: [u8; 32],

    pub layers: Vec<Layer>,
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

        let _page_size = stream.read_u32_le()?;
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
            .then(|| Rect::try_parse_f64(stream))
            .transpose()?;

        let tag_list: Option<Vec<String>> = if field_check_flags & 2 != 0 {
            let tag_count = stream.read_u16_le()?;

            Some(
                (0..tag_count)
                    .map(|_| stream.read_short_u16_string())
                    .collect::<Result<_, _>>()?,
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

        let background_image_mode: BackgroundImageMode = (field_check_flags & 16 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .map(TryInto::try_into)
            .transpose()?
            .unwrap_or_default();

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

        let template_type: Option<TemplateType> = (field_check_flags & 512 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .map(TryInto::try_into)
            .transpose()?;

        // The app uses a `LinkedHashMap` here, so entry order must be important.
        // Since we are unlikely to use this for much, a `Vec` is fine in place of a real map.
        let mut canvas_cache_map: Vec<(u32, CanvasCacheEntry)> = vec![];

        if field_check_flags & 1024 != 0 {
            let entry_count: i64 = stream.read_u32_le()?.into();
            let entry_size: i64 = stream.read_u16_le()?.into();

            if entry_size == 49 {
                canvas_cache_map.reserve_exact(entry_count.try_into()?);

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
            .transpose()?
            // This gets skipped by the libs.
            .inspect(|v| eprintln!("Warning: Read theme value {v} of unknown meaning"));

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
            custom_objects.reserve_exact(custom_object_count);

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
            current_layer_index,
            hash,
            layers,
        })
    }
}
