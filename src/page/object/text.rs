use std::io::{self, Seek};

use num::FromPrimitive;
use num_derive::FromPrimitive;
use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ReadStringError, WrongEndOffsetError},
    page::object::DocObject,
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
struct Span {
    span_base: SpanBase,
    bytes: Vec<u8>,
}

impl Span {
    fn try_parse(stream: &mut impl ByteStreamLe) -> Result<Span, SpanParseError> {
        let data_size: usize = stream.read_u16_le()?.into();

        // >> `data_size` starts mesasuring from here <<

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
        let bytes = stream.read_u8_buf(data_size - 16)?;

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

        let bytes = stream.read_u8_buf(data_size - 12)?;

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

// todo: Contained DocObject may need to be boxed in the future to avoid recursion, depending on
// what DocObject ends up looking like.

#[derive(Debug)]
struct DocObjectSpan {
    object: DocObject,
    start: u32,

    // todo: Remove this
    _end: u32,
}

#[derive(Error, Debug)]
pub enum CommonParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("failed to read main text string")]
    MainText(ReadStringError),

    #[error("span count does not fit in `usize`")]
    TooManySpans,

    #[error("paragraph count does not fit in `usize`")]
    TooManyParagraphs,

    #[error("failed to parse a span")]
    Span(#[from] SpanParseError),

    #[error("failed to parse a paragraph")]
    Paragraph(#[from] ParagraphParseError),

    #[error("invalid gravity type {0}")]
    BadGravityType(u8),

    #[error("object span count does not fit in `usize`")]
    TooManyObjectSpans,

    #[error("failed to parse a doc object")]
    DocObject(color_eyre::Report),

    #[error("object span did not finish where expected")]
    BadObjectSpanEndOffset(WrongEndOffsetError),

    #[error("did not finish where expected")]
    BadEndOffset(WrongEndOffsetError),
}

#[derive(Debug)]
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
    pub fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        format_version: u32,
    ) -> Result<Common, CommonParseError> {
        let expected_end = {
            let data_size: u64 = stream.read_u32_le()?.into();

            // >> `data_size` starts here <<
            let data_start = stream.stream_position()?;

            data_start + data_size
        };

        let text = stream
            .read_long_u16_string()
            .map_err(CommonParseError::MainText)?;

        let spans = {
            let count: usize = stream
                .read_u32_le()?
                .try_into()
                .map_err(|_| CommonParseError::TooManySpans)?;

            let mut spans = Vec::with_capacity(count);

            for _ in 0..count {
                spans.push(Span::try_parse(stream)?);
            }

            spans
        };

        let paragraphs = {
            let count: usize = stream
                .read_u32_le()?
                .try_into()
                .map_err(|_| CommonParseError::TooManyParagraphs)?;

            let mut paragraphs = Vec::with_capacity(count);

            for _ in 0..count {
                paragraphs.push(Paragraph::try_parse(stream)?);
            }

            paragraphs
        };

        let (left_margin, top_margin, right_margin, bottom_margin) = (
            stream.read_f32_le()?,
            stream.read_f32_le()?,
            stream.read_f32_le()?,
            stream.read_f32_le()?,
        );

        let gravity = {
            let val = stream.read_u8()?;
            Gravity::from_u8(val).ok_or(CommonParseError::BadGravityType(val))?
        };

        let section_data = {
            let count: usize = stream.read_u16_le()?.into();

            let mut data = Vec::with_capacity(count);

            for _ in 0..count {
                data.push((stream.read_u32_le()?, stream.read_u32_le()?));
            }

            data
        };

        let mut object_spans = vec![];

        if format_version >= 2035 {
            // This is stored as a Boolean and written explicitly as a 32-bit integer.
            let object_spans_present = stream.read_u32_le()? != 0;
            let _zero = stream.read_u32_le()?;

            if object_spans_present {
                let object_span_count: usize = stream
                    .read_u32_le()?
                    .try_into()
                    .map_err(|_| CommonParseError::TooManyObjectSpans)?;

                object_spans.reserve_exact(object_span_count);

                for _ in 0..object_span_count {
                    let obj_span_expected_end = {
                        let size: u64 = stream.read_u32_le()?.into();
                        let start = stream.stream_position()?;

                        start + size
                    };

                    let doc_object_size = stream.read_u32_le()?;

                    // This could be a single byte...
                    let object_type = stream.read_u32_le()?;

                    // `doc_object_size` measures exactly the size of this:
                    let doc_object = DocObject::try_parse(stream, object_type, 0)
                        .map_err(CommonParseError::DocObject)?;

                    object_spans.push(DocObjectSpan {
                        object: doc_object,
                        start: stream.read_u32_le()?,
                        _end: 0,
                    });

                    // Since doc objects can be extremely complex (we're parsing one right now),
                    // check the end offset matches what we expected. There's plenty of room for
                    // bugs in the parsing.
                    let obj_span_actual_end = stream.stream_position()?;

                    if obj_span_actual_end != obj_span_expected_end {
                        return Err(CommonParseError::BadObjectSpanEndOffset(
                            WrongEndOffsetError {
                                actual_end: obj_span_actual_end,
                                expected_end: obj_span_expected_end,
                            },
                        ));
                    }
                }
            }
        }

        let actual_end = stream.stream_position()?;

        if actual_end != expected_end {
            return Err(CommonParseError::BadEndOffset(WrongEndOffsetError {
                actual_end,
                expected_end,
            }));
        }

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
