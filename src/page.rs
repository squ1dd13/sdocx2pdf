use crate::{
    OpaqueBytes, OpaqueBytesParseError,
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        ByteStreamLe, ExactSizedStream, ReadBitfieldError, ReadStringError, ReadTimestampError,
        TryParse,
    },
    context::{DocumentContext, TryParseWithContext},
    impl_try_from_for_optional_from,
    page::{
        header::{
            CanvasCacheEntry, CustomPageObject, CustomPageObjectParseError, PdfDataItemParseError,
            PdfPage,
        },
        object::DocObject,
    },
    read_size_and_vec, unpack_bool_flag, unpack_field_flags,
};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{self, Read, Seek, SeekFrom};
use thiserror::Error;

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

pub type LayerParseError = color_eyre::Report;

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

    fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<Layer, LayerParseError> {
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

#[derive(Error, Debug)]
#[error(transparent)]
pub enum PageParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
    Timestamp(#[from] ReadTimestampError),
    BackgroundImageMode(#[from] InvalidBackgroundImageModeError),
    CustomPageObject(#[from] CustomPageObjectParseError),
    Layer(#[from] LayerParseError),
    OpaqueBytes(#[from] OpaqueBytesParseError),
    PdfDataItem(#[from] PdfDataItemParseError),
    TemplateType(#[from] InvalidTemplateTypeError),

    #[error("failed to read property flags")]
    PropertyFlags(#[source] ReadBitfieldError),

    #[error("one or more properties were unhandled")]
    UnhandledProperty(#[source] UnhandledBitsError),

    #[error("failed to read field flags")]
    FieldFlags(#[source] ReadBitfieldError),

    #[error("one or more field flags were unhandled")]
    UnhandledField(#[source] UnhandledBitsError),

    #[error("too many entries ({0})")]
    TooManyEntries(u32),

    #[error("expected end string to be '{ex}', but it is '{0}'", ex = Page::END_STRING)]
    BadEndString(String),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Page {
    is_text_only: bool,

    orientation: u32,
    width: u32,
    height: u32,
    offset_x: u32,
    offset_y: u32,
    uuid: String,
    modified_time: DateTime<Utc>,
    format_version: u32,
    min_format_version: u32,

    drawn_rect: Option<Rect>,
    tags: Vec<String>,
    template_uri: Option<String>,
    background_image_id: Option<i32>,
    background_image_mode: Option<BackgroundImageMode>,
    background_colour: Option<[u8; 4]>,
    background_width: Option<u32>,
    background_rotation: Option<u32>,
    pdf_data_items: Vec<PdfPage>,
    template_type: Option<TemplateType>,
    canvas_cache_map: Vec<(u32, CanvasCacheEntry)>,
    imported_data_height: Option<u32>,
    theme: Option<u32>,
    recognised_data_modified_time: Option<DateTime<Utc>>,
    stroke_recognition_data: Vec<OpaqueBytes>,
    custom_objects: Vec<CustomPageObject>,
    current_layer_index: u16,

    pub layers: Vec<Layer>,
}

impl Page {
    const END_STRING: &str = "Page for SAMSUNG S-Pen SDK";
}

impl<R: Read + Seek> TryParseWithContext<R, DocumentContext<'_, '_>> for Page {
    type ParseError = PageParseError;

    fn try_parse_with_ctx(
        reader: &mut R,
        ctx: &DocumentContext<'_, '_>,
    ) -> Result<Page, PageParseError> {
        let start = reader.stream_position()?;

        let page_end_offset: u64 = start + u64::from(reader.read_u32_le()?);
        let flex_offset: u64 = start + u64::from(reader.read_u32_le()?);

        let mut property_flags =
            CheckedBitfield::try_parse(reader).map_err(PageParseError::PropertyFlags)?;

        unpack_bool_flag!(property_flags, 0 => is_text_only);

        property_flags
            .ensure_none_set_unchecked()
            .map_err(PageParseError::UnhandledProperty)?;

        let mut field_flags =
            CheckedBitfield::try_parse(reader).map_err(PageParseError::FieldFlags)?;

        let orientation = reader.read_u32_le()?;
        let width = reader.read_u32_le()?;
        let height = reader.read_u32_le()?;
        let offset_x = reader.read_u32_le()?;
        let offset_y = reader.read_u32_le()?;
        let uuid = reader.read_short_u16_string()?;
        let modified_time = reader.read_timestamp()?;
        let format_version = reader.read_u32_le()?;
        let min_format_version = reader.read_u32_le()?;

        {
            let here = reader.stream_position()?;

            if here != flex_offset {
                eprintln!(
                    "Warning: Did not reach page flex offset naturally. \
                Will seek from {here} to {flex_offset} (delta {}).",
                    flex_offset.wrapping_sub(here).cast_signed()
                );

                reader.seek(SeekFrom::Start(flex_offset))?;
            }
        }

        unpack_field_flags!(field_flags, {
            0 => drawn_rect: Rect::try_parse_f64(reader)?;
            1 => tags: {
                read_size_and_vec!(reader, u16, reader.read_short_u16_string()?)
            }, else vec![];

            2 => template_uri: reader.read_short_u16_string()?;
            3 => background_image_id: reader.read_i32_le()?;
            4 => background_image_mode: reader.read_u32_le()?.try_into()?;
            5 => background_colour: reader.read_4_bytes()?;
            6 => background_width: reader.read_u32_le()?;
            7 => background_rotation: reader.read_u32_le()?;

            8 => pdf_data_items: {
                let pdi_ctx = header::PdfDataItemParseCtx {
                    file_registry: ctx.file_registry,
                    format_version,
                };

                read_size_and_vec!(
                    reader,
                    u16,
                    PdfPage::try_parse_with_ctx(reader, &pdi_ctx)?,
                )
            }, else vec![];

            9 => template_type: reader.read_u32_le()?.try_into()?;
            10 => canvas_cache_map: {
                let entry_count = reader.read_u32_le()?;
                let entry_size = reader.read_u16_le()?;

                let mut canvas_cache_map: Vec<(u32, CanvasCacheEntry)> = vec![];

                if entry_size == 49 {
                    canvas_cache_map
                        .reserve_exact(entry_count.try_into()
                        .map_err(|_| PageParseError::TooManyEntries(entry_count))?);

                    // The app uses a `LinkedHashMap` here, so entry order must be important.
                    // We're unlikely to use this, so a `Vec` is fine in place of an `IndexMap`,
                    // and we avoid another dependency.
                    for _ in 0..entry_count {
                        let key = reader.read_u32_le()?;
                        let entry = CanvasCacheEntry::try_parse(reader)?;

                        canvas_cache_map.push((key, entry));
                    }
                } else {
                    eprintln!("Warning: Skipping CCM: entry size is {entry_size}, not 49.");
                    reader.seek_relative(i64::from(entry_count) * i64::from(entry_size))?;
                }

                canvas_cache_map
            }, else vec![];

            11 => imported_data_height: reader.read_u32_le()?;
            12 => theme: {
                // This gets skipped by the libs.
                reader
                    .read_u32_le()
                    .inspect(|v| eprintln!("Warning: Read theme value {v} of unknown meaning"))?
            };

            // missing 13, 14

            15 => recognised_data_modified_time: reader.read_timestamp()?;
            16 => stroke_recognition_data: read_size_and_vec!(
                reader,
                u32,
                PageParseError::TooManyEntries,
                OpaqueBytes::try_parse_exclusive(reader)?,
            ), else vec![];

            // missing 17

            18 => custom_objects: read_size_and_vec!(
                reader,
                u32,
                PageParseError::TooManyEntries,
                CustomPageObject::try_parse_with_ctx(reader, ctx.file_registry)?,
            ), else vec![];
        });

        field_flags
            .ensure_none_set_unchecked()
            .map_err(PageParseError::UnhandledField)?;

        {
            let here = reader.stream_position()?;

            if here != page_end_offset {
                eprintln!(
                    "Warning: Did not reach page end offset naturally. \
                Will seek from {here} to {page_end_offset} (delta {}).",
                    page_end_offset.wrapping_sub(here).cast_signed()
                );

                reader.seek(SeekFrom::Start(page_end_offset))?;
            }
        }

        let layer_count: usize = reader.read_u16_le()?.into();
        let current_layer_index = reader.read_u16_le()?;

        let mut layers = Vec::with_capacity(layer_count);

        for _ in 0..layer_count {
            layers.push(Layer::try_parse(reader)?);
        }

        // todo: Validate this.
        let _hash = {
            let mut h = [0_u8; 32];
            reader.read_exact(&mut h)?;
            h
        };

        let mut remaining = Vec::with_capacity(Page::END_STRING.len());
        reader.read_to_end(&mut remaining)?;

        if remaining != Page::END_STRING.as_bytes() {
            // string_from_utf8_lossy_owned is not stable yet:
            // https://github.com/rust-lang/rust/issues/129436
            return Err(PageParseError::BadEndString(
                String::from_utf8_lossy(&remaining).into(),
            ));
        }

        Ok(Page {
            is_text_only,
            orientation,
            width,
            height,
            offset_x,
            offset_y,
            uuid,
            modified_time,
            format_version,
            min_format_version,
            drawn_rect,
            tags,
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
            layers,
        })
    }
}
