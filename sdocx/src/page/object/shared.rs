use std::io;

use crate::{
    byte_stream::ByteStreamLe,
    impl_try_from_for_optional_from,
    page::{Point, Rect},
    read_u32_sized_vec,
};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use strum::Display;
use thiserror::Error;

/// A segment of a `Path`. Variant names are based on `SpenPath` constants.
///
/// Variants here correspond to methods on
/// [`android.graphics.Path`](https://developer.android.com/reference/android/graphics/Path).
#[derive(Debug, Clone, Copy)]
pub enum PathSegment {
    /// `TYPE_MOVETO`; 1
    MoveTo(Point),

    /// `TYPE_LINETO`; 2
    LineTo(Point),

    /// `TYPE_QUADTO`; 3
    QuadTo { cp1: Point, p2: Point },

    /// `TYPE_CUBICTO`; 4
    CubicTo { cp1: Point, cp2: Point, p3: Point },

    /// `TYPE_ARCTO`; 5
    ArcTo {
        oval: Rect,
        start_angle: f64,
        sweep_angle: f64,
    },

    /// `TYPE_CLOSE`; 6
    Close,

    /// `TYPE_ADDOVAL`; 7
    AddOval(Rect),
}

#[derive(Error, Debug)]
pub enum PathParseError {
    #[error("io error")]
    Io(#[from] std::io::Error),

    #[error("segment count {0} does not fit in `usize`")]
    TooManySegments(u32),

    #[error("invalid segment type ID {0}")]
    BadSegmentType(u8),
}

#[derive(Debug)]
pub struct Path {
    segments: Vec<PathSegment>,
}

impl Path {
    pub(crate) fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<Path, PathParseError> {
        Ok(Path {
            segments: read_u32_sized_vec!(stream, PathParseError::TooManySegments, {
                match stream.read_u8()? {
                    1 => PathSegment::MoveTo(Point::try_parse_f64(stream)?),
                    2 => PathSegment::LineTo(Point::try_parse_f64(stream)?),

                    3 => PathSegment::QuadTo {
                        cp1: Point::try_parse_f64(stream)?,
                        p2: Point::try_parse_f64(stream)?,
                    },

                    4 => PathSegment::CubicTo {
                        cp1: Point::try_parse_f64(stream)?,
                        cp2: Point::try_parse_f64(stream)?,
                        p3: Point::try_parse_f64(stream)?,
                    },

                    5 => PathSegment::ArcTo {
                        oval: Rect::try_parse_f64(stream)?,
                        start_angle: stream.read_f64_le()?,
                        sweep_angle: stream.read_f64_le()?,
                    },

                    6 => PathSegment::Close,
                    7 => PathSegment::AddOval(Rect::try_parse_f64(stream)?),

                    bad => return Err(PathParseError::BadSegmentType(bad)),
                }
            }),
        })
    }

    pub fn segments(&self) -> &[PathSegment] {
        &self.segments
    }
}

#[derive(Clone, Copy, Debug, FromPrimitive, Display)]
pub enum ColourType {
    /// `COLOR_SOLID`
    Solid = 0,
    /// `COLOR_GRADIENT`
    Gradient = 1,
    /// `COLOR_NONE`
    None = 2,
}

impl_try_from_for_optional_from!(ColourType, u8, from_u8, pub InvalidColourTypeError);

impl ColourType {
    pub const fn is_solid(self) -> bool {
        matches!(self, ColourType::Solid)
    }
}

#[derive(Debug, FromPrimitive)]
pub enum GradientType {
    /// `GRADIENT_LINEAR`
    Linear = 0,
    /// `GRADIENT_RADIAL`
    Radial = 1,
    /// `GRADIENT_RECTANGULAR`
    Rectangular = 2,
    /// `GRADIENT_PATH`
    Path = 3,
}

impl_try_from_for_optional_from!(GradientType, u8, from_u8, pub InvalidGradientTypeError);

#[derive(Debug)]
pub struct GradientColour {
    pub colour: [u8; 4],
    pub position: f32,
}

impl GradientColour {
    pub fn try_parse(stream: &mut impl ByteStreamLe) -> io::Result<GradientColour> {
        Ok(GradientColour {
            colour: stream.read_4_bytes()?,
            position: stream.read_f32_le()?,
        })
    }
}
