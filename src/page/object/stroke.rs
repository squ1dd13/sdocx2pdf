use std::{
    f32,
    io::{self, Read, Seek},
};

use num::FromPrimitive;
use num_derive::FromPrimitive;
use thiserror::Error;

use crate::{
    byte_stream::{BoundedStream, ByteStreamLe, TryParse, UnfinishedParsingError},
    impl_try_from_for_optional_from,
    page::{
        Point,
        object::{
            base::{HasObjectBase, ObjectBase, ObjectBaseParseError},
            header::{ObjectHeader, ObjectHeaderError},
        },
    },
    unpack_bool_flags, unpack_field_flags,
};

/// See [Android developer website][1].
///
/// [1]: <https://developer.android.com/develop/ui/compose/touch-input/stylus-input/advanced-stylus-features#stylus_axis_data>
#[derive(Clone, Copy, Debug)]
struct TiltData {
    /// Tilt angle in radians. Range is \[0, pi/2\], where 0 means the pen is parallel to the axis
    /// coming out of the document towards the user, and pi/2 means the pen is parallel to the
    /// plane of the document.
    tilt: f32,

    /// Orientation angle in radians, relative to the document. Range is \[-pi, pi\], where
    ///  * 0 means the pen is oriented with its tip towards the top of the document;
    ///  * +/- pi means the tip is towards the bottom of the document;
    ///  * -pi/2 means ths tip is towards the left;
    ///  * pi/2 means the tip is towards the right.
    orientation: f32,
}

struct Event {
    point: Point,
    pressure: f32,
    timestamp: u32,
    tilt_data: Option<TiltData>,
}

impl Event {
    /// Converts `fixed` to `f64` from the 16-bit fixed-point format used for point component
    /// deltas in the compressed data.
    fn point_component_delta_to_float(fixed: u16) -> f64 {
        // | sign bit | 10 bit integer part | 5 bit fractional part |
        // 5 bits in the fractional part means a denominator of 2^5 = 32, so the maximum
        // representable value is 0b1111111111 + 0b11111 / 32 ≈ 1024.

        let is_negative = fixed & 0x8000 != 0;
        let integer = f64::from((fixed & 0x7fff) >> 5);
        let fraction = f64::from(fixed & 0x1f) / 32.0;

        let absolute = integer + fraction;
        if is_negative { -absolute } else { absolute }
    }

    /// Converts `fixed` to `f32` from the 16-bit fixed-point format used for pressure,
    /// tilt and orientation deltas in the compressed data.
    fn small_delta_to_float(fixed: u16) -> f32 {
        // | sign bit | 3 bit integer part | 12 bit fractional part |
        // 12 bit fractional part means the denominator is 2^12 = 4096, so the maximum
        // representable value is 0b111 + 0b111111111111 / 4096 ≈ 8.

        let is_negative = fixed & 0x8000 != 0;
        let integer = f32::from((fixed & 0x7fff) >> 12);
        let fraction = f32::from(fixed & 0xfff) / 4096.0;

        let absolute = integer + fraction;
        if is_negative { -absolute } else { absolute }
    }

    /// Parses compressed stroke event data from `stream`.
    ///
    /// In compressed data, instead of every event having each field stored in full, only full
    /// values are stored for the first event, and subsequent events are represented as deltas,
    /// with 16 bits for each field. Instead of floating-point values, two different fixed-point
    /// formats are used.
    fn parse_compressed_events(
        stream: &mut impl ByteStreamLe,
        event_count: usize,
        has_tilt_data: bool,
        origin_is_f64: bool,
    ) -> io::Result<Vec<Event>> {
        let Some(delta_count) = event_count.checked_sub(1) else {
            // No events.
            return Ok(vec![]);
        };

        let origin = match origin_is_f64 {
            true => Point::try_parse_f64(stream)?,
            false => Point::try_parse_f32(stream)?,
        };

        // Alternates x, y, x, y, ...
        let point_deltas_xy = stream.read_u16s(2 * delta_count)?;

        let origin_pressure = stream.read_f32_le()?;
        let pressure_deltas = stream.read_u16s(delta_count)?;

        let origin_timestamp = stream.read_u32_le()?;
        let timestamp_deltas = stream.read_u16s(delta_count)?;

        let (origin_tilt_data, tilt_deltas, orientation_deltas) = if has_tilt_data {
            let origin_tilt = stream.read_f32_le()?;
            let tilt_deltas = stream.read_u16s(delta_count)?;

            let origin_orientation = stream.read_f32_le()?;
            let orientation_deltas = stream.read_u16s(delta_count)?;

            (
                Some(TiltData {
                    tilt: origin_tilt,
                    orientation: origin_orientation,
                }),
                tilt_deltas,
                orientation_deltas,
            )
        } else {
            (None, vec![], vec![])
        };

        let mut events = Vec::with_capacity(event_count);

        events.push(Event {
            point: origin,
            pressure: origin_pressure,
            timestamp: origin_timestamp,
            tilt_data: origin_tilt_data,
        });

        for delta_i in 0..delta_count {
            // `unwrap` because there is always guaranteed to be an event.
            let previous_event = events.last().unwrap();

            let d_y = Event::point_component_delta_to_float(point_deltas_xy[2 * delta_i]);
            let d_x = Event::point_component_delta_to_float(point_deltas_xy[1 + 2 * delta_i]);

            let d_pressure = Event::small_delta_to_float(pressure_deltas[delta_i]);
            let d_timestamp = u32::from(timestamp_deltas[delta_i]);

            events.push(Event {
                point: Point {
                    x: previous_event.point.x + d_y,
                    y: previous_event.point.y + d_x,
                },

                pressure: previous_event.pressure + d_pressure,
                timestamp: previous_event.timestamp + d_timestamp,

                tilt_data: previous_event.tilt_data.map(|last| TiltData {
                    tilt: last.tilt + Event::small_delta_to_float(tilt_deltas[delta_i]),
                    orientation: last.orientation
                        + Event::small_delta_to_float(orientation_deltas[delta_i]),
                }),
            });
        }

        Ok(events)
    }

    fn parse_uncompressed_events(
        stream: &mut impl ByteStreamLe,
        event_count: usize,
        has_tilt_data: bool,
    ) -> io::Result<Vec<Event>> {
        // x, y, x, y, ...
        let points_xy = stream.read_f64s(2 * event_count)?;

        let pressures = stream.read_f32s(event_count)?;
        let timestamps = stream.read_u32s(event_count)?;

        let (tilts, orientations) = if has_tilt_data {
            let tilts = stream.read_f32s(event_count)?;
            let orientations = stream.read_f32s(event_count)?;

            (tilts, orientations)
        } else {
            (vec![], vec![])
        };

        let mut events = Vec::with_capacity(event_count);

        for i in 0..event_count {
            events.push(Event {
                point: Point {
                    x: points_xy[2 * i],
                    y: points_xy[1 + 2 * i],
                },

                pressure: pressures[i],
                timestamp: timestamps[i],

                tilt_data: tilts
                    .get(i)
                    .zip(orientations.get(i))
                    .map(|(&t, &o)| TiltData {
                        tilt: t,
                        orientation: o,
                    }),
            });
        }

        Ok(events)
    }
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(event @ t = {}, (x, y) = ({:.5}, {:.5}); p = {:.2}%; ",
            self.timestamp,
            self.point.x,
            self.point.y,
            self.pressure * 100.0
        )?;

        if let Some(TiltData { tilt, orientation }) = self.tilt_data {
            write!(
                f,
                "t.t = ({:.3})π, t.o = ({:.3})π)",
                tilt / f32::consts::PI,
                orientation / f32::consts::PI
            )
        } else {
            write!(f, "no tilt data)")
        }
    }
}

#[derive(Debug, FromPrimitive)]
enum ToolType {
    /// `TOOL_TYPE_UNKNOWN`
    Unknown = 0,
    /// `TOOL_TYPE_FINGER`
    Finger = 1,
    /// `TOOL_TYPE_SPEN`
    Pen = 2,
    /// `TOOL_TYPE_MOUSE`
    Mouse = 3,
    /// `TOOL_TYPE_ERASER`
    Eraser = 4,
}

impl_try_from_for_optional_from!(ToolType, u16, from_u16, pub InvalidToolTypeError);

#[derive(Debug, FromPrimitive)]
enum DashType {
    /// `CONTINUOUS_LINE`
    Continuous = 0,
    /// `DASHED_LINE`
    Dashed = 1,
    /// `DASHED_SPACE_LINE`
    DashedSpace = 2,
    /// `LONG_DASHED_DOTTED_LINE`
    LongDashedDotted = 3,
    /// `LONG_DASHED_DOUBLE_DOTTED_LINE`
    LongDashedDoubleDotted = 4,
    /// `LONG_DASHED_TRIPLE_DOTTED_LINE`
    LongDashedTripleDotted = 5,
    /// `DOTTED_LINE`
    Dotted = 6,
    /// `LONG_DASHED_SHORT_DASHED_LINE`
    LongDashedShortDashed = 7,
    /// `LONG_DASHED_DOULBE_SHORT_DASHED_LINE`
    LongDashedDoulbeShortDashed = 8,
    /// `DASHED_DOTTED_LINE`
    DashedDotted = 9,
    /// `DOUBLE_DASHED_DOTTED_LINE`
    DoubleDashedDotted = 10,
    /// `DASHED_DOUBLE_DOTTED_LINE`
    DashedDoubleDotted = 11,
    /// `DOUBLE_DASHED_DOUBLE_DOTTED_LINE`
    DoubleDashedDoubleDotted = 12,
    /// `DASHED_TRIPLE_DOTTED_LINE`
    DashedTripleDotted = 13,
    /// `DOUBLE_DASHED_TRIPLE_DOTTED_LINE`
    DoubleDashedTripleDotted = 14,
}

impl_try_from_for_optional_from!(DashType, u16, from_u16, pub InvalidDashTypeError);

// `SPen::ObjectStroke::SetStrokeType` errors if the stroke type set is >= 3. Variant names
// are unknown as of right now.
#[derive(Debug, FromPrimitive)]
enum StrokeType {
    Zero = 0,
    One = 1,
    Two = 2,
}

impl_try_from_for_optional_from!(StrokeType, u16, from_u16, pub InvalidStrokeTypeError);

#[derive(Error, Debug)]
#[error(transparent)]
pub enum StrokeParseError {
    Io(#[from] io::Error),
    Base(#[from] ObjectBaseParseError),
    Header(#[from] ObjectHeaderError),
    BadToolType(#[from] InvalidToolTypeError),
    BadDashType(#[from] InvalidDashTypeError),
    BadStrokeType(#[from] InvalidStrokeTypeError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Stroke {
    object_base: ObjectBase,

    is_curve_enabled: bool,
    is_replay_only_enabled: bool,
    is_tilt_data_present: bool,
    is_eraser_enabled: bool,
    is_fixed_width_enabled: bool,
    is_millisecond_mode: bool,
    is_top_layer_pen: bool,
    is_alpha_locked: bool,
    is_binary_added: bool,
    is_generated: bool,

    events: Vec<Event>,

    tool_type: ToolType,

    advanced_pen_settings_str_id: Option<u32>,
    colour: Option<[u8; 4]>,
    pen_size: Option<f32>,
    unk: Option<u32>,
    pen_name_str_id: Option<u32>,
    fixed_width: Option<f32>,
    size_level: Option<u32>,
    particle_density: Option<u32>,
    rendering_level: Option<u32>,
    original_width: Option<u32>,
    initial_tolerance: Option<f32>,
    dash_type: Option<DashType>,
    dash_offset: Option<f32>,
    stroke_type: Option<StrokeType>,
    pen_repeat_distance: f32,
}

impl<R: Read + Seek> TryParse<R> for Stroke {
    type ParseError = StrokeParseError;

    fn try_parse(stream: &mut R) -> Result<Stroke, StrokeParseError> {
        let object_base = ObjectBase::try_parse(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 1)?;

        let property_flags = header.property_flags_mut();

        unpack_bool_flags!(property_flags, {
            0 => is_curve_enabled;
            1 => is_replay_only_enabled;
            2 => is_tilt_data_present;
            3 => is_eraser_enabled;
            4 => is_fixed_width_enabled;
            5 => is_millisecond_mode;
            6 => is_top_layer_pen;
            7 => is_alpha_locked;
            8 => !is_binary_added;
            // missing 9
            10 => !is_generated;
        });

        let event_count: usize = stream.read_u16_le()?.into();

        let events = if is_curve_enabled {
            Event::parse_compressed_events(&mut stream, event_count, is_tilt_data_present, true)?
        } else {
            Event::parse_uncompressed_events(&mut stream, event_count, is_tilt_data_present)?
        };

        let tool_type: ToolType = stream.read_u16_le()?.try_into()?;

        let field_check_flags = header.init_flex(&mut stream)?;

        // What follows is equivalent to
        // `SPen::ObjectStrokeBinaryHandler::m_ApplyBinary_FlexibleData`.
        //
        // `ObjectStrokeBinaryHandler` has a bool field that we'll call `strings_not_ids`. Iff
        // `strings_not_ids == true`, actual strings are read/written from/to the binary files
        // rather than string IDs for use with the `StringIDManager`. This field is set to `true`
        // in `SPen::ObjectStroke::GetBinaryByCoedit`, and `false` everywhere(?) else, including
        // the code paths we are most interested in. For this reason, we currently assume that only
        // string IDs are used. Depending on when `SPen::ObjectStroke::GetBinaryByCoedit` is
        // actually used, this may need revisiting in the future.

        unpack_field_flags!(field_check_flags, {
            // missing 0
            1 => advanced_pen_settings_str_id: stream.read_u32_le()?;
            2 => colour: stream.read_4_bytes()?;
            3 => pen_size: stream.read_f32_le()?;
            4 => unk: stream.read_u32_le()?;
            // missing 5 and 6
            7 => pen_name_str_id: stream.read_u32_le()?;
            8 => fixed_width: stream.read_f32_le()?;
            9 => size_level: stream.read_u32_le()?;
            10 => particle_density: stream.read_u32_le()?;
            11 => rendering_level: stream.read_u32_le()?;
            12 => original_width: stream.read_u32_le()?;
            13 => initial_tolerance: stream.read_f32_le()?;
            14 => dash_type: stream.read_u16_le()?.try_into()?;
            15 => dash_offset: stream.read_f32_le()?;
            16 => stroke_type: stream.read_u16_le()?.try_into()?;
            17 => pen_repeat_distance: stream.read_f32_le()?, else 0.5;
        });

        if let Some(unk) = unk {
            eprintln!("Warning: Read unknown stroke field (value {unk})");
        };

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Stroke {
            object_base,
            is_curve_enabled,
            is_replay_only_enabled,
            is_tilt_data_present,
            is_eraser_enabled,
            is_fixed_width_enabled,
            is_millisecond_mode,
            is_top_layer_pen,
            is_alpha_locked,
            is_binary_added,
            is_generated,
            events,
            tool_type,
            advanced_pen_settings_str_id,
            colour,
            pen_size,
            unk,
            pen_name_str_id,
            fixed_width,
            size_level,
            particle_density,
            rendering_level,
            original_width,
            initial_tolerance,
            dash_type,
            dash_offset,
            stroke_type,
            pen_repeat_distance,
        })
    }
}

impl HasObjectBase for Stroke {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}
