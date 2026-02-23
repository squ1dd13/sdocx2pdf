use super::ObjectBase;
use crate::{
    byte_stream::ByteStreamLe,
    page::{
        Point,
        object::{
            DocObjectInner, InheritsObjectBase,
            shared::{ColourType, GradientColour, GradientType},
        },
    },
};
use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{Seek, SeekFrom};

#[derive(Debug)]
struct LineColourEffect {
    gradient_rotatable: bool,
    colour_type: ColourType,
    solid_colour: [u8; 4],
    gradient_type: GradientType,
    angle: u16,
    radial_gradient_pos: Point,
    colours: Vec<GradientColour>,
}

impl LineColourEffect {
    fn try_parse(stream: &mut impl ByteStreamLe) -> Result<LineColourEffect> {
        // (colour count) * 8 + 19
        let _structure_size = stream.read_u32_le()?;

        // Not really variable length, and also not really a bitfield: the size byte is always 1,
        // and the single byte that follows encodes the rotatability in the first bit (so it's
        // effectively just a Boolean). The result is [0x1, 0x1 if rotatable else 0x0], but the
        // first byte makes more sense if we assume the intention is to follow the variable-length
        // bitfield format.
        let property_flags = stream.read_variable_length_bitfield()?;

        Ok(LineColourEffect {
            gradient_rotatable: property_flags & 1 != 0,
            colour_type: {
                let val = stream.read_u8()?;
                ColourType::from_u8(val).ok_or_else(|| eyre!("Bad colour type {val}"))?
            },
            solid_colour: stream.read_u32_le()?.to_le_bytes(),
            gradient_type: {
                let val = stream.read_u8()?;
                GradientType::from_u8(val).ok_or_else(|| eyre!("Bad gradient type {val}"))?
            },
            angle: stream.read_u16_le()?,
            radial_gradient_pos: Point::try_parse_f32(stream)?,
            colours: {
                let count: usize = stream.read_u8()?.into();
                let mut colours = Vec::with_capacity(count);

                for _ in 0..count {
                    colours.push(GradientColour {
                        colour: stream.read_u32_le()?.to_le_bytes(),
                        position: stream.read_f32_le()?,
                    });
                }

                colours
            },
        })
    }
}

#[derive(Debug, FromPrimitive)]
enum CapType {
    /// `CAP_TYPE_BUTT`
    Butt = 0,
    /// `CAP_TYPE_ROUND`
    Round = 1,
    /// `CAP_TYPE_SQUARE`
    Square = 2,
}

#[derive(Debug, FromPrimitive)]
enum CompoundType {
    /// `COMPOUND_TYPE_SIMPLE`
    Simple = 0,
    /// `COMPOUND_TYPE_DOUBLE`
    Double = 1,
    /// `COMPOUND_TYPE_THICK_THIN`
    ThickThin = 2,
    /// `COMPOUND_TYPE_THIN_THICK`
    ThinThick = 3,
    /// `COMPOUND_TYPE_TRIPLE`
    Triple = 4,
}

#[derive(Debug, FromPrimitive)]
enum DashType {
    /// `DASH_TYPE_SOLID`
    Solid = 0,
    /// `DASH_TYPE_ROUND_DOT`
    RoundDot = 1,
    /// `DASH_TYPE_SQUARE_DOT`
    SquareDot = 2,
    /// `DASH_TYPE_DASH`
    Dash = 3,
    /// `DASH_TYPE_DASH_DOT`
    DashDot = 4,
    /// `DASH_TYPE_LONG_DASH`
    LongDash = 5,
    /// `DASH_TYPE_LONG_DASH_DOT`
    LongDashDot = 6,
    /// `DASH_TYPE_LONG_DASH_DOT_DOT`
    LongDashDotDot = 7,
}

#[derive(Debug, FromPrimitive)]
enum ArrowSize {
    /// `ARROW_SIZE_NORMAL`
    Normal = 0,
    /// `ARROW_SIZE_SMALL`
    Small = 1,
    /// `ARROW_SIZE_BIG`
    Big = 2,
}

#[derive(Debug, FromPrimitive)]
enum ArrowShape {
    /// `ARROW_TYPE_NONE`
    None = 0,
    /// `ARROW_TYPE_ARROW`
    Arrow = 1,
    /// `ARROW_TYPE_OPEN_ARROW`
    OpenArrow = 2,
    /// `ARROW_TYPE_STEALTH_ARROW`
    StealthArrow = 3,
    /// `ARROW_TYPE_DIAMOND_ARROW`
    DiamondArrow = 4,
    /// `ARROW_TYPE_OVAL_ARROW`
    OvalArrow = 5,
}

#[derive(Debug, FromPrimitive)]
enum JoinType {
    /// `JOIN_TYPE_MITER`
    Miter = 0,
    /// `JOIN_TYPE_ROUND`
    Round = 1,
    /// `JOIN_TYPE_BEVEL`
    Bevel = 2,
}

#[derive(Debug)]
struct LineStyleEffect {
    width: f32,
    compound_type: CompoundType,
    dash_type: DashType,
    cap_type: CapType,
    join_type: JoinType,
    begin_arrow_shape: ArrowShape,
    begin_arrow_size: ArrowSize,
    end_arrow_shape: ArrowShape,
    end_arrow_size: ArrowSize,
}

impl LineStyleEffect {
    fn try_parse(stream: &mut impl ByteStreamLe) -> Result<LineStyleEffect> {
        let size = stream.read_u32_le()?;

        if size != 12 {
            return Err(eyre!("Line style effect size should be 12, not {size}"));
        }

        let width = stream.read_f32_le()?;

        let compound_type = stream.read_u8()?;
        let dash_type = stream.read_u8()?;
        let cap_type = stream.read_u8()?;
        let join_type = stream.read_u8()?;
        let begin_arrow_shape = stream.read_u8()?;
        let begin_arrow_size = stream.read_u8()?;
        let end_arrow_shape = stream.read_u8()?;
        let end_arrow_size = stream.read_u8()?;

        Ok(LineStyleEffect {
            width,

            compound_type: CompoundType::from_u8(compound_type)
                .ok_or_else(|| eyre!("Bad compound type {compound_type}"))?,

            dash_type: DashType::from_u8(dash_type)
                .ok_or_else(|| eyre!("Bad dash type {dash_type}"))?,

            cap_type: CapType::from_u8(cap_type).ok_or_else(|| eyre!("Bad cap type {cap_type}"))?,

            join_type: JoinType::from_u8(join_type)
                .ok_or_else(|| eyre!("Bad join type {join_type}"))?,

            begin_arrow_shape: ArrowShape::from_u8(begin_arrow_shape)
                .ok_or_else(|| eyre!("Bad arrow shape {begin_arrow_shape}"))?,

            begin_arrow_size: ArrowSize::from_u8(begin_arrow_size)
                .ok_or_else(|| eyre!("Bad arrow size {begin_arrow_size}"))?,

            end_arrow_shape: ArrowShape::from_u8(end_arrow_shape)
                .ok_or_else(|| eyre!("Bad arrow shape {end_arrow_shape}"))?,

            end_arrow_size: ArrowSize::from_u8(end_arrow_size)
                .ok_or_else(|| eyre!("Bad arrow size {end_arrow_size}"))?,
        })
    }
}

#[derive(Debug)]
struct ConnectionPoint {
    point: Point,
    uuids: Vec<String>,
}

#[derive(Debug)]
pub struct Base {
    object_base: ObjectBase,

    line_colour_effect: Option<LineColourEffect>,
    line_style_effect: Option<LineStyleEffect>,
    slave_uuids: Vec<String>,
    // Unclear on these. In the JVM code, both are `ArrayList`s of `ConnectionPoint`s, but
    // one uses only the `point` field, while the other uses both `point` and `uuids`.
    connection_points: Vec<ConnectionPoint>,
    points_of_connection: Vec<Point>,
}

impl InheritsObjectBase for Base {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> Result<Base> {
        if child_count != 0 {
            return Err(eyre!(
                "Shape base should not have children, but {child_count} declared"
            ));
        }

        // The declared size is inclusive of the size field, so take the offset before
        // reading the size.
        let start_offset = stream.stream_position()?;

        let expected_end = {
            let size: u64 = stream.read_u32_le()?.into();
            start_offset + size
        };

        let data_type_id = stream.read_u16_le()?;

        if data_type_id != 6 {
            return Err(eyre!(
                "Shape base data type ID should be 6, not {data_type_id}"
            ));
        }

        let flex_offset: u64 = stream.read_u32_le()?.into();

        // Again, these aren't really variable-length because they are hardcoded at 1 byte each.
        // Also, there are no property flags, so that field is always 0.
        let _property_flags = stream.read_variable_length_bitfield()?;
        let stated_field_check_flags = stream.read_variable_length_bitfield()?;

        let points_of_connection = {
            let count: usize = stream.read_u32_le()?.try_into()?;
            let mut points = Vec::with_capacity(count);

            for _ in 0..count {
                points.push(Point::try_parse_f64(stream)?);
            }

            points
        };

        let _connection_points_total_size = stream.read_u32_le()?;

        let connection_points = {
            let count: usize = stream.read_u32_le()?.try_into()?;
            let mut points = Vec::with_capacity(count);

            for _ in 0..count {
                let point = Point::try_parse_f64(stream)?;

                let uuid_count: usize = stream.read_u32_le()?.try_into()?;
                let mut uuids = Vec::with_capacity(uuid_count);

                for _ in 0..uuid_count {
                    uuids.push(stream.read_short_u8_string()?);
                }

                points.push(ConnectionPoint { point, uuids });
            }

            points
        };

        let field_check_flags = if flex_offset != 0 {
            // There is flex data, so seek to where it starts and use the field check flags we were
            // given.
            stream.seek(SeekFrom::Start(start_offset + flex_offset))?;
            stated_field_check_flags
        } else {
            // No flex data, so disable all the fields.
            0
        };

        let line_colour_effect = (stated_field_check_flags & 4 != 0)
            .then(|| LineColourEffect::try_parse(stream))
            .transpose()?;

        let line_style_effect = (stated_field_check_flags & 8 != 0)
            .then(|| LineStyleEffect::try_parse(stream))
            .transpose()?;

        let mut slave_uuids = vec![];

        if stated_field_check_flags & 128 != 0 {
            let count: usize = stream.read_u16_le()?.into();
            slave_uuids.reserve_exact(count);

            for _ in 0..count {
                slave_uuids.push(stream.read_short_u8_string()?);
            }
        }

        let end_offset = stream.stream_position()?;

        if end_offset != expected_end {
            return Err(eyre!(
                "Expected end offset is {expected_end}, but ended at {end_offset}"
            ));
        }

        Ok(Base {
            object_base,
            line_colour_effect,
            line_style_effect,
            slave_uuids,
            connection_points,
            points_of_connection,
        })
    }
}
