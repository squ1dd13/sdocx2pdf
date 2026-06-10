use crate::{
    OpaqueBytes, OpaqueBytesParseError,
    byte_stream::{SeekableByteStreamLe, TryParse},
    context::{DocumentContext, TryParseWithContext},
    page::object::{
        audio::{Audio, AudioParseError},
        base::{HasObjectBase, ObjectBase, ObjectBaseParseError},
        image::{Image, ImageParseError},
        line::{Line, LineParseError},
        painting::{Painting, PaintingParseError},
        shape::{Shape, ShapeParseContext, ShapeParseError},
        stroke::{Stroke, StrokeParseError},
        text::{Text, TextParseError},
        web::{Web, WebParseError},
    },
};
use std::io::{Read, Seek};
use thiserror::Error;

mod audio;
mod base;
mod header;
mod image;
mod line;
mod painting;
mod shape;
mod shape_base;
mod shared;
pub mod stroke;
pub mod text;
mod text_core;
mod web;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum OpaqueObjectParseError {
    Base(#[from] ObjectBaseParseError),
    Bytes(#[from] OpaqueBytesParseError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct OpaqueObject {
    object_base: ObjectBase,
    inner: OpaqueBytes,
}

impl<R: Read + Seek> TryParse<R> for OpaqueObject {
    type ParseError = OpaqueObjectParseError;

    fn try_parse(reader: &mut R) -> Result<OpaqueObject, OpaqueObjectParseError> {
        Ok(OpaqueObject {
            object_base: ObjectBase::try_parse(reader)?,
            inner: OpaqueBytes::try_parse_inclusive(reader)?,
        })
    }
}

impl HasObjectBase for OpaqueObject {
    fn object_base(&self) -> &ObjectBase {
        &self.object_base
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum DocObjectParseError {
    Io(#[from] std::io::Error),

    Image(#[from] ImageParseError),
    Line(#[from] LineParseError),
    Painting(#[from] PaintingParseError),
    Shape(#[from] ShapeParseError),
    Stroke(#[from] StrokeParseError),
    Text(#[from] TextParseError),
    Voice(#[from] AudioParseError),
    Web(#[from] WebParseError),

    Opaque(#[from] OpaqueObjectParseError),

    #[error("object type {0} is not supported")]
    BadType(u8),
}

pub(crate) struct DocObjectParseContext<'a, 'b> {
    pub object_type: u8,
    pub doc_ctx: DocumentContext<'a, 'b>,
}

#[derive(Debug)]
pub enum DocObject {
    /// `WCon_ObjectStroke`; extends `WCon_ObjectBase`
    Stroke(Box<Stroke>),

    /// `WCon_ObjectTextBoxOrImage` (my name; variant 1) extends `WCon_ObjectShape` (`Shape`)
    Text(Box<Text>),

    /// `WCon_ObjectTextBoxOrImage` (my name; variant 0) extends `WCon_ObjectShape` (`Shape`)
    Image(Box<Image>),

    /// `WCon_ObjectContainer`; extends `WCon_ObjectBase`
    Container(OpaqueObject),

    /// `WCon_ObjectShape`; extends `WCon_ObjectShapeBase`, which extends `WCon_ObjectBase`
    Shape(Box<Shape>),

    /// `WCon_ObjectLine`; extends `WCon_ObjectShapeBase` (see `Shape`)
    Line(Box<Line>),

    /// `WCon_ObjectVoice`; extends `WCon_ObjectBase`
    Audio(Box<Audio>),

    /// `WCon_ObjectFormula`; extends `WCon_ObjectBase`
    Formula(OpaqueObject),

    /// `WCon_ObjectTable`; extends `WCon_ObjectBase`
    Table(OpaqueObject),

    /// `WCon_ObjectWeb`; extends `WCon_ObjectBase`
    Web(Box<Web>),

    /// `WCon_ObjectPainting`; extends `WCon_ObjectBase`
    Painting(Box<Painting>),

    /// `WCon_ObjectLink`; extends `WCon_ObjectBase`
    Link(OpaqueObject),

    /// `WCon_ObjectMath`; extends `WCon_ObjectBase`
    Maths(OpaqueObject),

    /// `WCon_ObjectPlot`; extends `WCon_ObjectBase`
    Plot(OpaqueObject),

    /// `WCon_ObjectUnknown`; extends `WCon_ObjectBase`
    Generic(OpaqueObject),
}

// Only implemented for a `dyn` reader so that objects themselves can use this trait impl without
// errors related to recursive reader types.
impl<'a> TryParseWithContext<dyn SeekableByteStreamLe + 'a, DocObjectParseContext<'a, 'a>>
    for DocObject
{
    type ParseError = DocObjectParseError;

    fn try_parse_with_ctx(
        mut stream: &mut (dyn SeekableByteStreamLe + 'a),
        DocObjectParseContext {
            object_type,
            doc_ctx,
        }: &DocObjectParseContext<'a, 'a>,
    ) -> Result<DocObject, DocObjectParseError> {
        let object_type = *object_type;

        // Because `dyn SeekableByteStreamLe` is not `Sized`:
        let stream = &mut stream;

        Ok(match object_type {
            1 => DocObject::Stroke(Box::new(Stroke::try_parse_with_ctx(
                stream,
                doc_ctx.string_registry,
            )?)),

            2 => DocObject::Text(Box::new(Text::try_parse_with_ctx(stream, doc_ctx)?)),
            3 => DocObject::Image(Box::new(Image::try_parse_with_ctx(stream, doc_ctx)?)),

            7 => DocObject::Shape(Box::new(Shape::try_parse_with_ctx(
                stream,
                &ShapeParseContext {
                    is_shape_only: true,
                    doc_ctx: *doc_ctx,
                },
            )?)),

            8 => DocObject::Line(Box::new(TryParse::try_parse(stream)?)),
            10 => DocObject::Audio(Box::new(TryParse::try_parse(stream)?)),
            13 => DocObject::Web(Box::new(TryParse::try_parse(stream)?)),
            14 => DocObject::Painting(Box::new(TryParse::try_parse(stream)?)),

            _ => {
                let object = OpaqueObject::try_parse(stream)?;

                match object_type {
                    4 => DocObject::Container({
                        eprintln!("Warning: Containers are not yet supported");
                        object
                    }),
                    11 => DocObject::Formula({
                        eprintln!("Warning: Formulas are not yet supported");
                        object
                    }),
                    17 => DocObject::Link({
                        eprintln!("Warning: Links are not yet supported");
                        object
                    }),
                    19 => DocObject::Generic({
                        eprintln!("Warning: Generic objects are not yet supported");
                        object
                    }),
                    20 => DocObject::Plot({
                        eprintln!("Warning: Plots are not yet supported");
                        object
                    }),
                    21 => DocObject::Maths({
                        eprintln!("Warning: Maths objects are not yet supported");
                        object
                    }),
                    22 => DocObject::Table({
                        eprintln!("Warning: Tables are not yet supported");
                        object
                    }),

                    unknown => return Err(DocObjectParseError::BadType(unknown)),
                }
            }
        })
    }
}

impl DocObject {
    pub fn object_base(&self) -> &ObjectBase {
        match self {
            DocObject::Line(line_object) => line_object.object_base(),
            DocObject::Shape(shape_object) => shape_object.object_base(),
            DocObject::Stroke(stroke_object) => stroke_object.object_base(),
            DocObject::Text(text_object) => text_object.object_base(),
            DocObject::Image(image_object) => image_object.object_base(),
            DocObject::Audio(voice_object) => voice_object.object_base(),
            DocObject::Web(web_object) => web_object.object_base(),
            DocObject::Painting(painting_object) => painting_object.object_base(),

            DocObject::Container(object)
            | DocObject::Formula(object)
            | DocObject::Table(object)
            | DocObject::Link(object)
            | DocObject::Maths(object)
            | DocObject::Plot(object)
            | DocObject::Generic(object) => &object.object_base,
        }
    }
}
