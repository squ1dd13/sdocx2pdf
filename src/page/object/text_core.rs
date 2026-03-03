use std::io::{self, Seek};

use num::FromPrimitive;
use num_derive::FromPrimitive;
use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ExactSizedStream, ReadStringError, UnfinishedParsingError},
    impl_try_from_for_optional_from,
    page::object::{DocObject, DocObjectParseError},
    read_u16_sized_vec, read_u32_sized_vec,
};

#[derive(Debug, FromPrimitive)]
enum Gravity {
    /// `GRAVITY_TOP`
    Top = 0,
    /// `GRAVITY_CENTER`
    Centre = 1,
    /// `GRAVITY_BOTTOM`
    Bottom = 2,
}

impl_try_from_for_optional_from!(Gravity, u8, from_u8, pub InvalidGravityError);

#[derive(Debug, FromPrimitive)]
enum SpanType {
    /// `TYPE_NONE`
    None = 0,
    /// `TYPE_FOREGROUND_COLOR`
    ForegroundColour = 1,
    /// `TYPE_FONT_SIZE`
    FontSize = 3,
    /// `TYPE_FONT_NAME`
    FontName = 4,
    /// `TYPE_BOLD`
    Bold = 5,
    /// `TYPE_ITALIC`
    Italic = 6,
    /// `TYPE_UNDERLINE`
    Underline = 7,
    /// `TYPE_HYPER_TEXT`
    Hypertext = 9,
    /// `TYPE_COMPOSING_BACKGROUND_COLOR`
    ComposingBackgroundColour = 15,
    /// `TYPE_COMPOSING`
    Composing = 16,
    /// `TYPE_BACKGROUND_COLOR`
    BackgroundColour = 17,
    /// `TYPE_COMPOSING_TAG`
    ComposingTag = 18,
    /// `TYPE_TIME_STAMP`
    TimeStamp = 19,
    /// `TYPE_STRIKETHROUGH`
    Strikethrough = 20,
    /// `TYPE_SUGGESTION`
    Suggestion = 21,
    /// `TYPE_SPELL_CORRECTION`
    SpellCorrection = 22,
    /// `TYPE_FORMULA`
    Formula = 23,
    /// `TYPE_MAX`
    Max = 24,
}

#[derive(Debug, FromPrimitive)]
enum IntervalType {
    //// `SPAN_INCLUSIVE_EXCLUSIVE`
    InclusiveExclusive = 0,
    /// `SPAN_INCLUSIVE_INCLUSIVE`
    InclusiveInclusive = 1,
    /// `SPAN_EXCLUSIVE_EXCLUSIVE`
    ExclusiveExclusive = 2,
    /// `SPAN_EXCLUSIVE_INCLUSIVE`
    ExclusiveInclusive = 3,
}

#[derive(Debug)]
#[allow(dead_code)]
struct SpanBase {
    span_type: SpanType,
    start_pos: u32,
    end_pos: u32,
    interval_type: IntervalType,
}

#[derive(Error, Debug)]
pub enum SpanParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("invalid span type {0}")]
    BadSpanType(u32),

    #[error("invalid span interval type {0}")]
    BadIntervalType(u32),
}

#[derive(Debug)]
#[allow(dead_code)]
struct Span {
    span_base: SpanBase,
    bytes: Vec<u8>,
}

impl Span {
    fn try_parse(stream: &mut impl ByteStreamLe) -> Result<Span, SpanParseError> {
        let data_size: usize = stream.read_u16_le()?.into();

        // >> `data_size` starts measuring from here <<

        let span_type = {
            let val = stream.read_u32_le()?;
            SpanType::from_u32(val).ok_or(SpanParseError::BadSpanType(val))?
        };

        let start_pos = stream.read_u32_le()?;
        let end_pos = stream.read_u32_le()?;

        let interval_type = {
            let val = stream.read_u32_le()?;
            IntervalType::from_u32(val).ok_or(SpanParseError::BadIntervalType(val))?
        };

        // >> 16 bytes to here, which is the end of the base <<

        // Anything left is specific to the span type. (There does not have to be anything left.)
        let bytes = stream.read_u8s(data_size - 16)?;

        Ok(Span {
            span_base: SpanBase {
                span_type,
                start_pos,
                end_pos,
                interval_type,
            },
            bytes,
        })
    }
}

#[derive(Debug, FromPrimitive)]
enum ParagraphType {
    /// `TYPE_INDENTLEVEL`
    IndentLevel = 2,
    /// `TYPE_ALIGN`
    Alignment = 3,
    /// `TYPE_LINE_SPACING`
    LineSpacing = 4,
    /// `TYPE_BULLET`
    Bullet = 5,
    /// `TYPE_PARSING_STATE`
    ParsingState = 6,
}

#[derive(Debug)]
#[allow(dead_code)]
struct ParagraphBase {
    paragraph_type: ParagraphType,
    start_pos: u32,
    end_pos: u32,
}

#[derive(Error, Debug)]
pub enum ParagraphParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("invalid paragraph type {0}")]
    BadParagraphType(u32),
}

#[derive(Debug)]
#[allow(dead_code)]
struct Paragraph {
    paragraph_base: ParagraphBase,
    bytes: Vec<u8>,
}

impl Paragraph {
    fn try_parse(stream: &mut impl ByteStreamLe) -> Result<Paragraph, ParagraphParseError> {
        // See `Span` parsing logic.

        let data_size: usize = stream.read_u16_le()?.into();

        let paragraph_type = {
            let val = stream.read_u32_le()?;
            ParagraphType::from_u32(val).ok_or(ParagraphParseError::BadParagraphType(val))?
        };

        let start_pos = stream.read_u32_le()?;
        let end_pos = stream.read_u32_le()?;

        let bytes = stream.read_u8s(data_size - 12)?;

        Ok(Paragraph {
            paragraph_base: ParagraphBase {
                paragraph_type,
                start_pos,
                end_pos,
            },
            bytes,
        })
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct DocObjectSpan {
    object: DocObject,
    start: u32,

    // todo: Remove this
    _end: u32,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum CommonParseError {
    Io(#[from] io::Error),

    #[error("failed to read main text string")]
    MainText(#[source] ReadStringError),

    #[error("span count does not fit in `usize`")]
    TooManySpans,

    #[error("paragraph count does not fit in `usize`")]
    TooManyParagraphs,

    Span(#[from] SpanParseError),
    Paragraph(#[from] ParagraphParseError),
    BadGravityType(#[from] InvalidGravityError),

    #[error("object span count does not fit in `usize`")]
    TooManyObjectSpans,

    #[error("{0} is too big to be a valid object type")]
    ObjectTypeTooBig(u32),

    DocObject(#[from] Box<DocObjectParseError>),

    #[error("bytes left over after parsing object span")]
    ObjectSpanUnfinished(#[source] UnfinishedParsingError),

    #[error(transparent)]
    Unfinished(#[from] UnfinishedParsingError),
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Common {
    text: String,
    left_margin: f32,
    top_margin: f32,
    right_margin: f32,
    bottom_margin: f32,
    gravity: Gravity,

    spans: Vec<Span>,
    paragraphs: Vec<Paragraph>,
    section_data: Vec<(u32, u32)>,
    object_spans: Vec<DocObjectSpan>,
}

impl Common {
    pub fn try_parse_with_version(
        stream: &mut (impl ByteStreamLe + Seek),
        format_version: u32,
    ) -> Result<Common, CommonParseError> {
        let mut stream = stream.take_exclusive_length_prefixed()?;

        let text = stream
            .read_long_u16_string()
            .map_err(CommonParseError::MainText)?;

        let spans = read_u32_sized_vec!(
            stream,
            |_| CommonParseError::TooManySpans,
            Span::try_parse(&mut stream)?
        );

        let paragraphs = read_u32_sized_vec!(
            stream,
            |_| CommonParseError::TooManyParagraphs,
            Paragraph::try_parse(&mut stream)?
        );

        let (left_margin, top_margin, right_margin, bottom_margin) = (
            stream.read_f32_le()?,
            stream.read_f32_le()?,
            stream.read_f32_le()?,
            stream.read_f32_le()?,
        );

        let gravity = Gravity::try_from(stream.read_u8()?)?;

        let section_data =
            read_u16_sized_vec!(stream, (stream.read_u32_le()?, stream.read_u32_le()?));

        let object_spans = if format_version >= 2035 && {
            // This is stored as a Boolean but written explicitly as a 32-bit integer.
            let object_spans_present = stream.read_u32_le()? != 0;

            // todo: ??
            let _zero = stream.read_u32_le()?;

            object_spans_present
        } {
            read_u32_sized_vec!(stream, |_| CommonParseError::TooManyObjectSpans, {
                let mut span_stream = (&mut stream).take_exclusive_length_prefixed()?;

                let _doc_object_size = span_stream.read_u32_le()?;

                // This could be a single byte...
                let object_type = span_stream.read_u32_le()?;

                // `doc_object_size` measures exactly the size of this:
                let doc_object = DocObject::try_parse_with_type(
                    &mut span_stream,
                    object_type
                        .try_into()
                        .map_err(|_| CommonParseError::ObjectTypeTooBig(object_type))?,
                )
                .map_err(|e| CommonParseError::DocObject(Box::new(e)))?;

                let span = DocObjectSpan {
                    object: doc_object,
                    start: span_stream.read_u32_le()?,
                    _end: 0,
                };

                span_stream
                    .ensure_eof()
                    .map_err(CommonParseError::ObjectSpanUnfinished)?;

                span
            })
        } else {
            vec![]
        };

        stream.ensure_eof()?;

        Ok(Common {
            text,
            left_margin,
            top_margin,
            right_margin,
            bottom_margin,
            gravity,
            spans,
            paragraphs,
            section_data,
            object_spans,
        })
    }
}
