use std::io;

use crate::{
    byte_stream::ByteStreamLe,
    page::{Point, Rect},
};
use num_derive::FromPrimitive;
use thiserror::Error;

/// A segment of a `Path`. Variant names are based on `SpenPath` constants.
#[derive(Debug)]
enum PathSegment {
    /// `TYPE_MOVETO`; 1
    MoveTo(Point),

    /// `TYPE_LINETO`; 2
    LineTo(Point),

    /// `TYPE_QUADTO`; 3
    QuadTo(Point, Point),

    /// `TYPE_CUBICTO`; 4
    CubicTo(Point, Point, Point),

    /// `TYPE_ARCTO`; 5
    ArcTo(Rect, f64, f64),

    /// `TYPE_CLOSE`; 6
    Close,

    /// `TYPE_ADDOVAL`; 7
    AddOval(Rect),
}

#[derive(Error, Debug)]
pub enum PathParseError {
    #[error("io error")]
    Io(#[from] std::io::Error),

    #[error("segment count does not fit in `usize`")]
    TooManySegments(std::num::TryFromIntError),

    #[error("invalid segment type ID {0}")]
    BadSegmentType(u8),
}

#[derive(Debug)]
pub struct Path {
    segments: Vec<PathSegment>,
}

impl Path {
    pub fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<Path, PathParseError> {
        let segment_count: usize = stream
            .read_u32_le()?
            .try_into()
            .map_err(PathParseError::TooManySegments)?;

        let mut segments = Vec::with_capacity(segment_count);

        for _ in 0..segment_count {
            segments.push(match stream.read_u8()? {
                1 => PathSegment::MoveTo(Point::try_parse_f64(stream)?),

                2 => PathSegment::LineTo(Point::try_parse_f64(stream)?),

                3 => PathSegment::QuadTo(
                    Point::try_parse_f64(stream)?,
                    Point::try_parse_f64(stream)?,
                ),

                4 => PathSegment::CubicTo(
                    Point::try_parse_f64(stream)?,
                    Point::try_parse_f64(stream)?,
                    Point::try_parse_f64(stream)?,
                ),

                5 => PathSegment::ArcTo(
                    Rect::try_parse_f64(stream)?,
                    stream.read_f64_le()?,
                    stream.read_f64_le()?,
                ),

                6 => PathSegment::Close,

                7 => PathSegment::AddOval(Rect::try_parse_f64(stream)?),

                bad => return Err(PathParseError::BadSegmentType(bad)),
            });
        }

        Ok(Path { segments })
    }
}

#[derive(Debug, FromPrimitive)]
pub enum ColourType {
    /// `COLOR_SOLID`
    Solid = 0,
    /// `COLOR_GRADIENT`
    Gradient = 1,
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

#[derive(Debug)]
pub struct GradientColour {
    pub colour: [u8; 4],
    pub position: f32,
}

impl GradientColour {
    pub fn try_parse(stream: &mut impl ByteStreamLe) -> io::Result<GradientColour> {
        Ok(GradientColour {
            colour: stream.read_u32_le()?.to_le_bytes(),
            position: stream.read_f32_le()?,
        })
    }
}
