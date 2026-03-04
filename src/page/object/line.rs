use crate::{
    byte_stream::{ByteStreamLe, ExactSizedStream, TryParse, UnfinishedParsingError},
    impl_try_from_for_optional_from,
    page::{
        Point, Rect,
        object::{
            base::{HasObjectBase, ObjectBase},
            header::{ObjectHeader, ObjectHeaderError},
            shape_base::{ShapeBase, ShapeBaseParseError},
            shared::{Path, PathParseError},
        },
    },
    read_size_and_vec, unpack_field_flags,
};
use color_eyre::eyre::Result;
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{Read, Seek};
use thiserror::Error;

#[derive(Debug, FromPrimitive)]
pub enum ConnectorType {
    /// `CONNECTOR_BEGIN`
    Begin = 0,

    /// `CONNECTOR_END`
    End = 1,
}

impl_try_from_for_optional_from!(ConnectorType, u8, from_u8, pub InvalidConnectorTypeError);

#[derive(Error, Debug)]
#[error(transparent)]
pub enum LineParseError {
    Io(#[from] std::io::Error),
    Base(#[from] ShapeBaseParseError),
    Header(#[from] ObjectHeaderError),
    ConnectorType(#[from] InvalidConnectorTypeError),
    Path(#[from] PathParseError),
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Line {
    shape_base: ShapeBase,
    connector_type: ConnectorType,
    start_direction: u8,
    control_points: Vec<Point>,
    start_point: Point,
    end_point: Point,
    original_drawn_rect: Rect,
    original_rect: Rect,
    original_angle: f32,
    default_pen_name_id: Option<u32>,
    pen_style_id: Option<u32>,
    pen_name_id: Option<u32>,
    path: Option<Path>,
}

impl<R: Read + Seek> TryParse<R> for Line {
    type ParseError = LineParseError;

    fn try_parse(stream: &mut R) -> Result<Line, LineParseError> {
        let shape_base = ShapeBase::try_parse(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 8)?;

        let connector_type: ConnectorType = stream.read_u8()?.try_into()?;
        let start_direction = stream.read_u8()?;

        let control_points = read_size_and_vec!(stream, u8, Point::try_parse_f64(&mut stream)?);

        let start_point = Point::try_parse_f64(&mut stream)?;
        let end_point = Point::try_parse_f64(&mut stream)?;

        let original_drawn_rect = Rect::try_parse_f64(&mut stream)?;
        let original_rect = Rect::try_parse_f64(&mut stream)?;
        let original_angle = stream.read_f32_le()?;

        let field_flags = header.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => default_pen_name_id: stream.read_u32_le()?;
            1 => pen_style_id: stream.read_u32_le()?;
            2 => pen_name_id: stream.read_u32_le()?;
            3 => path: Path::try_parse(&mut stream)?;
        });

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Line {
            shape_base,
            connector_type,
            start_direction,
            control_points,
            start_point,
            end_point,
            original_drawn_rect,
            original_rect,
            original_angle,
            default_pen_name_id,
            pen_style_id,
            pen_name_id,
            path,
        })
    }
}

impl HasObjectBase for Line {
    fn object_base(&self) -> &ObjectBase {
        self.shape_base.object_base()
    }
}
