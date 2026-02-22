use super::ObjectBase;
use crate::{
    byte_stream::ByteStreamLe,
    page::{
        Point, Rect,
        object::{
            ConcreteInheritsObjectBase, DocObjectInner, InheritsObjectBase, shape_base::ShapeBase,
        },
    },
};
use color_eyre::eyre::{Result, eyre};
use std::io::{Seek, SeekFrom};

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

#[derive(Debug)]
struct Path {
    segments: Vec<PathSegment>,
}

impl Path {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<Path> {
        let segment_count: usize = stream.read_u32_le()?.try_into()?;
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

                unknown => return Err(eyre!("Bad path segment type ID {unknown}")),
            });
        }

        Ok(Path { segments })
    }
}

#[derive(Debug)]
pub struct LineObject {
    shape_base: ShapeBase,
    connector_type: u8,
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

impl InheritsObjectBase for LineObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> Result<LineObject> {
        let shape_base = ShapeBase::try_parse(stream, object_base, child_count)?;

        let start_offset = stream.stream_position()?;

        let expected_end = {
            let size: u64 = stream.read_u32_le()?.into();
            start_offset + size
        };

        let data_type_id = stream.read_u16_le()?;

        if data_type_id != 8 {
            return Err(eyre!(
                "Line object data type ID should be 8, not {data_type_id}"
            ));
        }

        let flex_offset: u64 = stream.read_u32_le()?.into();

        let _property_flags = stream.read_variable_length_bitfield()?;
        let stated_field_check_flags = stream.read_variable_length_bitfield()?;

        let connector_type = stream.read_u8()?;
        let start_direction = stream.read_u8()?;

        let control_points = {
            let count: usize = stream.read_u8()?.into();
            let mut points = Vec::with_capacity(count);

            for _ in 0..count {
                points.push(Point::try_parse_f64(stream)?);
            }

            points
        };

        let start_point = Point::try_parse_f64(stream)?;
        let end_point = Point::try_parse_f64(stream)?;

        let original_drawn_rect = Rect::try_parse_f64(stream)?;
        let original_rect = Rect::try_parse_f64(stream)?;
        let original_angle = stream.read_f32_le()?;

        let field_check_flags = if flex_offset != 0 {
            // We have flex data, so the fields are active.
            stream.seek(SeekFrom::Start(start_offset + flex_offset))?;
            stated_field_check_flags
        } else {
            // No flex data.
            0
        };

        let default_pen_name_id = (field_check_flags & 1 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let pen_style_id = (field_check_flags & 2 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let pen_name_id = (field_check_flags & 4 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .or(default_pen_name_id);

        let path = (field_check_flags & 8 != 0)
            .then(|| Path::try_parse(stream))
            .transpose()?;

        let end_offset = stream.stream_position()?;

        if end_offset != expected_end {
            return Err(eyre!(
                "Expected end offset is {expected_end}, but ended at {end_offset}"
            ));
        }

        Ok(LineObject {
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

impl ConcreteInheritsObjectBase for LineObject {}
