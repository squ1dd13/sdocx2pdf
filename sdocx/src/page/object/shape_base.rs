use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BoundedStream, ByteStreamLe, ReadBitfieldError, ReadStringError,
        TakeInclusiveLengthPrefixedError, TryParse, UnfinishedParsingError,
    },
    impl_try_from_for_optional_from,
    page::{
        Point,
        object::{
            base::{HasObjectBase, ObjectBase, ObjectBaseParseError},
            header::{FlagBlockError, ObjectHeaderError, try_parse_object_header},
            shared::{
                ColourType, GradientColour, GradientType, InvalidColourTypeError,
                InvalidGradientTypeError,
            },
        },
    },
    read_size_and_vec, read_u32_sized_vec, unpack_bool_flag, unpack_field_flags,
};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{Read, Seek};
use thiserror::Error;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum LineColourEffectParseError {
    Io(#[from] std::io::Error),
    BadSize(#[from] TakeInclusiveLengthPrefixedError),
    PropertyFlags(#[from] ReadBitfieldError),
    UnhandledProperty(#[from] UnhandledBitsError),
    ColourType(#[from] InvalidColourTypeError),
    GradientType(#[from] InvalidGradientTypeError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct LineColourEffect {
    gradient_rotatable: bool,
    pub colour_type: ColourType,
    solid_colour: [u8; 4],
    gradient_type: GradientType,
    angle: u16,
    radial_gradient_pos: Point,
    colours: Vec<GradientColour>,
}

impl LineColourEffect {
    fn try_parse<R: Read>(stream: &mut R) -> Result<LineColourEffect, LineColourEffectParseError> {
        let mut stream = stream.take_exclusive_length_prefixed()?;

        let mut property_flags = CheckedBitfield::try_parse(&mut stream)?;

        unpack_bool_flag!(property_flags, 0 => gradient_rotatable);

        property_flags.ensure_none_set_unchecked()?;

        let effect = LineColourEffect {
            gradient_rotatable,
            colour_type: stream.read_u8()?.try_into()?,
            solid_colour: stream.read_4_bytes()?,
            gradient_type: stream.read_u8()?.try_into()?,
            angle: stream.read_u16_le()?,
            radial_gradient_pos: Point::try_parse_f32(&mut stream)?,
            colours: read_size_and_vec!(stream, u8, GradientColour::try_parse(&mut stream)?),
        };

        stream.ensure_eof()?;

        Ok(effect)
    }

    pub const fn solid_colour_bgra(&self) -> [u8; 4] {
        self.solid_colour
    }
}

impl Default for LineColourEffect {
    fn default() -> Self {
        Self {
            gradient_rotatable: false,
            colour_type: ColourType::Solid,
            solid_colour: [0, 0, 0, 255],
            gradient_type: GradientType::Linear,
            angle: 0,
            radial_gradient_pos: (0.0, 0.0).into(),
            colours: Vec::new(),
        }
    }
}

#[derive(Debug, FromPrimitive)]
pub enum CapType {
    /// `CAP_TYPE_BUTT`
    Butt = 0,
    /// `CAP_TYPE_ROUND`
    Round = 1,
    /// `CAP_TYPE_SQUARE`
    Square = 2,
}

impl_try_from_for_optional_from!(CapType, u8, from_u8, pub InvalidCapTypeError);

#[derive(Debug, FromPrimitive)]
pub enum CompoundType {
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

impl_try_from_for_optional_from!(CompoundType, u8, from_u8, pub InvalidCompoundTypeError);

#[derive(Debug, FromPrimitive)]
pub enum DashType {
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

impl_try_from_for_optional_from!(DashType, u8, from_u8, pub InvalidDashTypeError);

#[derive(Debug, FromPrimitive)]
pub enum ArrowSize {
    /// `ARROW_SIZE_NORMAL`
    Normal = 0,
    /// `ARROW_SIZE_SMALL`
    Small = 1,
    /// `ARROW_SIZE_BIG`
    Big = 2,
}

impl_try_from_for_optional_from!(ArrowSize, u8, from_u8, pub InvalidArrowSizeError);

#[derive(Debug, FromPrimitive)]
pub enum ArrowShape {
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

impl_try_from_for_optional_from!(ArrowShape, u8, from_u8, pub InvalidArrowShapeError);

#[derive(Debug, FromPrimitive)]
pub enum JoinType {
    /// `JOIN_TYPE_MITER`
    Miter = 0,
    /// `JOIN_TYPE_ROUND`
    Round = 1,
    /// `JOIN_TYPE_BEVEL`
    Bevel = 2,
}

impl_try_from_for_optional_from!(JoinType, u8, from_u8, pub InvalidJoinTypeError);

#[derive(Error, Debug)]
#[error(transparent)]
pub enum LineStyleEffectParseError {
    Io(#[from] std::io::Error),

    #[error("line style effect must be exactly 12 bytes, not {0}")]
    WrongSize(u32),

    CompoundType(#[from] InvalidCompoundTypeError),
    DashType(#[from] InvalidDashTypeError),
    CapType(#[from] InvalidCapTypeError),
    JoinType(#[from] InvalidJoinTypeError),
    ArrowShape(#[from] InvalidArrowShapeError),
    ArrowSize(#[from] InvalidArrowSizeError),
}

#[derive(Debug)]
pub struct LineStyleEffect {
    pub width: f32,
    pub compound_type: CompoundType,
    pub dash_type: DashType,
    pub cap_type: CapType,
    pub join_type: JoinType,
    pub begin_arrow_shape: ArrowShape,
    pub begin_arrow_size: ArrowSize,
    pub end_arrow_shape: ArrowShape,
    pub end_arrow_size: ArrowSize,
}

impl LineStyleEffect {
    fn try_parse<R: Read>(stream: &mut R) -> Result<LineStyleEffect, LineStyleEffectParseError> {
        match stream.read_u32_le()? {
            12 => (),
            bad => return Err(LineStyleEffectParseError::WrongSize(bad)),
        }

        let width = stream.read_f32_le()?;

        let mut buf = [0_u8; 8];
        stream.read_exact(&mut buf)?;

        Ok(LineStyleEffect {
            width,
            compound_type: buf[0].try_into()?,
            dash_type: buf[1].try_into()?,
            cap_type: buf[2].try_into()?,
            join_type: buf[3].try_into()?,
            begin_arrow_shape: buf[4].try_into()?,
            begin_arrow_size: buf[5].try_into()?,
            end_arrow_shape: buf[6].try_into()?,
            end_arrow_size: buf[7].try_into()?,
        })
    }
}

impl Default for LineStyleEffect {
    fn default() -> Self {
        Self {
            width: 2.0,
            compound_type: CompoundType::Simple,
            dash_type: DashType::Solid,
            cap_type: CapType::Butt,
            join_type: JoinType::Miter,
            begin_arrow_shape: ArrowShape::None,
            begin_arrow_size: ArrowSize::Normal,
            end_arrow_shape: ArrowShape::None,
            end_arrow_size: ArrowSize::Normal,
        }
    }
}

#[derive(Debug)]
#[expect(dead_code)]
struct ConnectionPoint {
    point: Point,
    uuids: Vec<String>,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ShapeBaseParseError {
    Io(#[from] std::io::Error),
    Base(#[from] ObjectBaseParseError),
    FlagBlock(#[from] FlagBlockError),
    Header(#[from] ObjectHeaderError),
    LineColourEffect(#[from] LineColourEffectParseError),
    LineStyleEffect(#[from] LineStyleEffectParseError),
    String(#[from] ReadStringError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("element count {0} is too large")]
    TooManyElements(u32),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct ShapeBase {
    object_base: ObjectBase,

    line_colour_effect: Option<LineColourEffect>,
    line_style_effect: Option<LineStyleEffect>,
    slave_uuids: Vec<String>,
    // Unclear on these. In the JVM code, both are `ArrayList`s of `ConnectionPoint`s, but
    // one uses only the `point` field, while the other uses both `point` and `uuids`.
    connection_points: Vec<ConnectionPoint>,
    points_of_connection: Vec<Point>,
}

impl ShapeBase {
    pub const fn line_colour_effect(&self) -> Option<&LineColourEffect> {
        self.line_colour_effect.as_ref()
    }

    pub const fn line_style(&self) -> Option<&LineStyleEffect> {
        self.line_style_effect.as_ref()
    }
}

impl<R: Read + Seek> TryParse<R> for ShapeBase {
    type ParseError = ShapeBaseParseError;

    fn try_parse(stream: &mut R) -> Result<ShapeBase, ShapeBaseParseError> {
        let object_base = ObjectBase::try_parse(stream)?;

        let (mut flag_block, mut stream) = try_parse_object_header(stream, 6)?;

        let points_of_connection = read_u32_sized_vec!(
            stream,
            ShapeBaseParseError::TooManyElements,
            Point::try_parse_f64(&mut stream)?
        );

        // Inclusive size. Living on the edge by not constructing a window here ;)
        let _connection_points_total_size = stream.read_u32_le()?;

        let conn_pts = read_size_and_vec!(stream, u32, ShapeBaseParseError::TooManyElements, {
            ConnectionPoint {
                point: Point::try_parse_f64(&mut stream)?,
                uuids: read_u32_sized_vec!(
                    stream,
                    ShapeBaseParseError::TooManyElements,
                    stream.read_short_u8_string()?
                ),
            }
        });

        let _skip = stream.read_u8()?;

        let field_flags = flag_block.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            // missing 0, 1

            2 => line_colour_effect: LineColourEffect::try_parse(&mut stream)?;
            3 => line_style_effect: LineStyleEffect::try_parse(&mut stream)?;

            // missing 4, 5, 6

            // SPen::FollowerManager::ApplyBinary
            7 => slave_uuids: read_size_and_vec!(
                stream,
                u16,
                stream.read_short_u8_string()?
            ), else vec![];
        });

        flag_block.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(ShapeBase {
            object_base,
            line_colour_effect,
            line_style_effect,
            slave_uuids,
            connection_points: conn_pts,
            points_of_connection,
        })
    }
}

impl HasObjectBase for ShapeBase {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}
