use std::io::{self, Seek, SeekFrom};

use byteorder::LittleEndian;
use num::FromPrimitive;
use num_derive::FromPrimitive;
use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ReadBitfieldError, WrongEndOffsetError},
    page::{
        Point,
        object::{ConcreteInheritsObjectBase, InheritsObjectBase, ObjectBase},
    },
};

#[derive(Clone, Copy, Debug)]
struct TiltData {
    tilt: f32,
    orientation: f32,
}

#[derive(Debug)]
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
        // representable value is 0b1111111111 + 0b11111 / 32 = 1023.96875.

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
        // representable value is 0b111 + 0b111111111111 / 4096 = 7.999755859375.

        let is_negative = fixed & 0x8000 != 0;
        let integer = f32::from((fixed & 0x7fff) >> 12);
        let fraction = f32::from(fixed & 0xfff) / 4096.0;

        let absolute = integer + fraction;
        if is_negative { -absolute } else { absolute }
    }

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
        let point_deltas_xy = {
            let mut deltas = vec![0_u16; 2 * delta_count];
            stream.read_u16_into::<LittleEndian>(&mut deltas)?;
            deltas
        };

        let origin_pressure = stream.read_f32_le()?;

        // todo: Add method to ByteStreamLe for reading many u16s
        let pressure_deltas = {
            let mut deltas = vec![0_u16; delta_count];
            stream.read_u16_into::<LittleEndian>(&mut deltas)?;
            deltas
        };

        let origin_timestamp = stream.read_u32_le()?;

        let timestamp_deltas = {
            let mut deltas = vec![0_u16; delta_count];
            stream.read_u16_into::<LittleEndian>(&mut deltas)?;
            deltas
        };

        let (origin_tilt_data, tilt_deltas, orientation_deltas) = if has_tilt_data {
            let origin_tilt = stream.read_f32_le()?;

            let tilt_deltas = {
                let mut deltas = vec![0_u16; delta_count];
                stream.read_u16_into::<LittleEndian>(&mut deltas)?;
                deltas
            };

            let origin_orientation = stream.read_f32_le()?;

            let orientation_deltas = {
                let mut deltas = vec![0_u16; delta_count];
                stream.read_u16_into::<LittleEndian>(&mut deltas)?;
                deltas
            };

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
        let points_xy = {
            let mut xy = vec![0.0; 2 * event_count];
            stream.read_f64_into::<LittleEndian>(&mut xy)?;
            xy
        };

        let pressures = {
            let mut pressures = vec![0.0; event_count];
            stream.read_f32_into::<LittleEndian>(&mut pressures)?;

            pressures
        };

        let timestamps = {
            let mut timestamps = vec![0; event_count];
            stream.read_u32_into::<LittleEndian>(&mut timestamps)?;

            timestamps
        };

        let (tilts, orientations) = if has_tilt_data {
            let tilts = {
                let mut tilts = vec![0.0; event_count];
                stream.read_f32_into::<LittleEndian>(&mut tilts)?;

                tilts
            };

            let orientations = {
                let mut orientations = vec![0.0; event_count];
                stream.read_f32_into::<LittleEndian>(&mut orientations)?;

                orientations
            };

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

// `SPen::ObjectStroke::SetStrokeType` errors if the stroke type set is >= 3. Variant names
// are unknown as of right now.
#[derive(Debug, FromPrimitive)]
enum StrokeType {
    Zero = 0,
    One = 1,
    Two = 2,
}

#[derive(Error, Debug)]
pub enum StrokeParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("invalid data type {0} for stroke object (should be 1)")]
    BadDataType(u16),

    #[error("failed to parse property flags")]
    PropertyFlags(ReadBitfieldError),

    #[error("failed to parse field check flags")]
    FieldCheckFlags(ReadBitfieldError),

    #[error("invalid tool type {0}")]
    BadToolType(u16),

    #[error("invalid dash type {0}")]
    BadDashType(u16),

    #[error("invalid stroke type {0}")]
    BadStrokeType(u16),

    #[error("parsed wrong number of bytes")]
    BadEndOffset(#[from] WrongEndOffsetError),
}

#[derive(Debug)]
pub struct StrokeObject {
    object_base: ObjectBase,

    is_curve_enabled: bool,
    is_replay_only_enabled: bool,
    is_tilt_data_present: bool,
    is_eraser_enabled: bool,
    is_fixed_width_enabled: bool,
    flag_millisecond_mode: bool,
    is_top_layer_pen: bool,
    flag_alpha_lock: bool,
    flag_binary_added: bool,
    flag_generated: bool,

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

impl StrokeObject {
    fn try_parse_inner<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> Result<StrokeObject, StrokeParseError> {
        let start_offset = stream.stream_position()?;

        let expected_end = {
            let size: u64 = stream.read_u32_le()?.into();
            start_offset + size
        };

        let data_type = stream.read_u16_le()?;

        if data_type != 1 {
            return Err(StrokeParseError::BadDataType(data_type));
        }

        let flex_offset: u64 = stream.read_u32_le()?.into();

        let property_flags = stream
            .read_variable_length_bitfield()
            .map_err(StrokeParseError::FieldCheckFlags)?;

        let is_curve_enabled = property_flags & 1 != 0;
        let is_replay_only_enabled = property_flags & 2 != 0;
        let is_tilt_data_present = property_flags & 4 != 0;
        let is_eraser_enabled = property_flags & 8 != 0;
        let is_fixed_width_enabled = property_flags & 16 != 0;
        let flag_millisecond_mode = property_flags & 32 != 0;
        let is_top_layer_pen = property_flags & 64 != 0;
        let flag_alpha_lock = property_flags & 128 != 0;

        // Inverted
        let flag_binary_added = property_flags & 256 == 0;
        let flag_generated = property_flags & 1024 == 0;

        let stated_field_check_flags = stream
            .read_variable_length_bitfield()
            .map_err(StrokeParseError::FieldCheckFlags)?;

        let event_count: usize = stream.read_u16_le()?.into();

        let events = if is_curve_enabled {
            Event::parse_compressed_events(stream, event_count, is_tilt_data_present, true)?
        } else {
            Event::parse_uncompressed_events(stream, event_count, is_tilt_data_present)?
        };

        let tool_type = {
            let val = stream.read_u16_le()?;
            ToolType::from_u16(val).ok_or(StrokeParseError::BadToolType(val))?
        };

        let field_check_flags = if flex_offset != 0 {
            stream.seek(SeekFrom::Start(start_offset + flex_offset))?;
            stated_field_check_flags
        } else {
            0
        };

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

        let advanced_pen_settings_str_id = (field_check_flags & 2 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let colour = (field_check_flags & 4 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .map(u32::to_le_bytes);

        let pen_size = (field_check_flags & 8 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?;

        let unk = (field_check_flags & 16 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .inspect(|val| eprintln!("Warning: Read unknown stroke field (value {val})"));

        let pen_name_str_id = (field_check_flags & 0x80 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let fixed_width = (field_check_flags & 0x100 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?;

        let size_level = (field_check_flags & 0x200 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let particle_density = (field_check_flags & 0x400 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let rendering_level = (field_check_flags & 0x800 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let original_width = (field_check_flags & 0x1000 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let initial_tolerance = (field_check_flags & 0x2000 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?;

        let dash_type = (field_check_flags & 0x4000 != 0)
            .then(|| stream.read_u16_le())
            .transpose()?
            .map(|v| DashType::from_u16(v).ok_or(StrokeParseError::BadDashType(v)))
            .transpose()?;

        let dash_offset = (field_check_flags & 0x8000 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?;

        let stroke_type = (field_check_flags & 0x10000 != 0)
            .then(|| stream.read_u16_le())
            .transpose()?
            .map(|v| StrokeType::from_u16(v).ok_or(StrokeParseError::BadStrokeType(v)))
            .transpose()?;

        let pen_repeat_distance = (field_check_flags & 0x20000 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?
            .unwrap_or(0.5);

        let actual_end = stream.stream_position()?;

        if actual_end != expected_end {
            return Err(WrongEndOffsetError {
                actual_end,
                expected_end,
            }
            .into());
        }

        Ok(StrokeObject {
            object_base,
            is_curve_enabled,
            is_replay_only_enabled,
            is_tilt_data_present,
            is_eraser_enabled,
            is_fixed_width_enabled,
            flag_millisecond_mode,
            is_top_layer_pen,
            flag_alpha_lock,
            flag_binary_added,
            flag_generated,
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

impl InheritsObjectBase for StrokeObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> color_eyre::eyre::Result<StrokeObject> {
        Ok(StrokeObject::try_parse_inner(
            stream,
            object_base,
            child_count,
        )?)
    }

    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}

impl ConcreteInheritsObjectBase for StrokeObject {}
