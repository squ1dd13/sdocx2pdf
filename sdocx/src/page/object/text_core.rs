use std::io::{self, Read, Seek};

use num::FromPrimitive;
use num_derive::FromPrimitive;
use thiserror::Error;

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{
        BlindWindow, BoundedStream, ByteStreamLe, ReadStringError, TryParse,
        UnfinishedParsingError, Window,
    },
    context::{DocumentContext, TryParseWithContext},
    if_any_left, impl_try_from_for_optional_from,
    page::object::{DocObject, DocObjectParseContext, DocObjectParseError},
    read_u16_sized_vec, read_u32_sized_vec, unpack_bool_flags,
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
}

impl_try_from_for_optional_from!(SpanType, u32, from_u32, pub InvalidSpanTypeError);

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

impl_try_from_for_optional_from!(IntervalType, u32, from_u32, pub InvalidIntervalTypeError);

#[derive(Debug)]
#[expect(dead_code)]
struct SpanBase {
    span_type: SpanType,
    start_pos: u32,
    end_pos: u32,
    interval_type: IntervalType,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum SpanParseError {
    Io(#[from] io::Error),
    SpanType(#[from] InvalidSpanTypeError),
    IntervalTypes(#[from] InvalidIntervalTypeError),

    #[error("data size {0} is too small")]
    BadSize(u16),
}

#[derive(Debug)]
#[expect(dead_code)]
struct Span {
    span_base: SpanBase,
    bytes: Vec<u8>,
}

impl<R: Read> TryParse<R> for Span {
    type ParseError = SpanParseError;

    fn try_parse(stream: &mut R) -> Result<Span, SpanParseError> {
        // First `u16` is the size of the span data that follows.
        let extra_data_size: usize = match stream.read_u16_le()? {
            // The `SpanBase` is 16 bytes.
            data_size @ 16.. => (data_size - 16).into(),
            data_size => return Err(SpanParseError::BadSize(data_size)),
        };

        Ok(Span {
            span_base: SpanBase {
                span_type: stream.read_u32_le()?.try_into()?,
                start_pos: stream.read_u32_le()?,
                end_pos: stream.read_u32_le()?,
                interval_type: stream.read_u32_le()?.try_into()?,
            },
            bytes: stream.read_u8s(extra_data_size)?,
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

impl_try_from_for_optional_from!(ParagraphType, u32, from_u32, pub InvalidParagraphTypeError);

#[derive(Debug)]
#[expect(dead_code)]
struct ParagraphBase {
    paragraph_type: ParagraphType,
    start_pos: u32,
    end_pos: u32,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ParagraphParseError {
    Io(#[from] io::Error),
    ParagraphType(#[from] InvalidParagraphTypeError),

    #[error("data size {0} is too small")]
    BadSize(u16),
}

#[derive(Debug)]
#[expect(dead_code)]
struct Paragraph {
    paragraph_base: ParagraphBase,
    bytes: Vec<u8>,
}

impl<R: Read> TryParse<R> for Paragraph {
    type ParseError = ParagraphParseError;

    fn try_parse(stream: &mut R) -> Result<Paragraph, ParagraphParseError> {
        // First `u16` is the size of the paragraph data that follows.
        let extra_data_size: usize = match stream.read_u16_le()? {
            // The `ParagraphBase` is 12 bytes.
            data_size @ 12.. => (data_size - 12).into(),
            data_size => return Err(ParagraphParseError::BadSize(data_size)),
        };

        Ok(Paragraph {
            paragraph_base: ParagraphBase {
                paragraph_type: stream.read_u32_le()?.try_into()?,
                start_pos: stream.read_u32_le()?,
                end_pos: stream.read_u32_le()?,
            },
            bytes: stream.read_u8s(extra_data_size)?,
        })
    }
}

#[derive(Debug, FromPrimitive)]
enum InTextLayoutOption {
    /// `LAYOUT_OPTION_BLOCK`
    Block = 0,
    /// `LAYOUT_OPTION_INLINE`
    Inline = 1,
    /// `LAYOUT_OPTION_BLOCK_WITH_MARGIN_SMALL`
    BlockSmallMargin = 2,
    /// `LAYOUT_OPTION_BLOCK_WITH_MARGIN_MEDIUM`
    BlockMediumMargin = 3,
}

impl_try_from_for_optional_from!(
    InTextLayoutOption,
    u32,
    from_u32,
    pub InvalidInTextLayoutOptionError,
);

#[derive(Debug, FromPrimitive)]
enum InTextLayoutConstraint {
    /// `LAYOUT_CONSTRAINT_NORMAL`
    Normal = 0,
    /// `LAYOUT_CONSTRAINT_OVER_PAGES_OVERLAP_PADDING`
    OverPagesOverlapPadding = 1,
    /// `LAYOUT_CONSTRAINT_OVER_PAGES`
    OverPages = 2,
}

impl_try_from_for_optional_from!(
    InTextLayoutConstraint,
    u32,
    from_u32,
    pub InvalidInTextLayoutConstraintError,
);

#[derive(Debug)]
#[expect(dead_code)]
struct InlineObject {
    index_in_text: u32,
    layout_option: Option<InTextLayoutOption>,
    layout_constraint: Option<InTextLayoutConstraint>,
    object: DocObject,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum CommonParseError {
    Io(#[from] io::Error),

    #[error("failed to read main text string")]
    MainText(#[source] ReadStringError),

    #[error("element count {0} is too large")]
    TooManyElements(u32),

    Span(#[from] SpanParseError),
    Paragraph(#[from] ParagraphParseError),
    BadGravityType(#[from] InvalidGravityError),

    #[error("{0} is too big to be a valid object type")]
    ObjectTypeTooBig(u32),

    #[error("one or more inline object list flags were set but not checked")]
    UnhandledInlineObjectListFlags(#[source] UnhandledBitsError),

    // The object we were trying to parse could include text, in which case the error may contain a
    // `CommonParseError`. `Box` here is to avoid recursion.
    InlineObject(#[from] Box<DocObjectParseError>),

    BadLayoutOption(#[from] InvalidInTextLayoutOptionError),
    BadLayoutConstraint(#[from] InvalidInTextLayoutConstraintError),

    #[error("object span was unfinished")]
    ObjectSpanUnfinished(#[source] UnfinishedParsingError),

    #[error("object in span was unfinished")]
    ObjectInObjectSpanUnfinished(#[source] UnfinishedParsingError),

    #[error(transparent)]
    Unfinished(#[from] UnfinishedParsingError),
}

pub struct CommonParseContext<'fr, 'sr> {
    pub format_version: u32,
    pub doc_ctx: DocumentContext<'fr, 'sr>,
}

#[derive(Debug)]
#[expect(dead_code)]
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

    // a.k.a. "object spans", but they're not really spans.
    inline_objects: Vec<InlineObject>,
}

impl Common {
    pub fn raw_string(&self) -> &str {
        &self.text
    }
}

impl<'a, R: Read + Seek> TryParseWithContext<R, CommonParseContext<'a, 'a>> for Common {
    type ParseError = CommonParseError;

    fn try_parse_with_ctx(
        stream: &mut R,
        &CommonParseContext {
            format_version,
            doc_ctx,
        }: &CommonParseContext<'a, 'a>,
    ) -> Result<Common, CommonParseError> {
        let mut stream = stream.take_exclusive_length_prefixed()?;

        let text = stream
            .read_long_u16_string()
            .map_err(CommonParseError::MainText)?;

        let spans = read_u32_sized_vec!(
            stream,
            CommonParseError::TooManyElements,
            Span::try_parse(&mut stream)?
        );

        let paragraphs = read_u32_sized_vec!(
            stream,
            CommonParseError::TooManyElements,
            Paragraph::try_parse(&mut stream)?
        );

        let (left_margin, top_margin, right_margin, bottom_margin) = (
            stream.read_f32_le()?,
            stream.read_f32_le()?,
            stream.read_f32_le()?,
            stream.read_f32_le()?,
        );

        let gravity: Gravity = stream.read_u8()?.try_into()?;

        let section_data =
            read_u16_sized_vec!(stream, (stream.read_u32_le()?, stream.read_u32_le()?));

        let inline_objects = if format_version >= 2035 && {
            let mut flags = CheckedBitfield::from(stream.read_u32_le()?);

            unpack_bool_flags!(flags, {
                0 => any_present;
            });

            flags
                .ensure_none_set_unchecked()
                .map_err(CommonParseError::UnhandledInlineObjectListFlags)?;

            // It's 64 bits really, so read a second 32-bit bitfield. None of the bits should be
            // set.
            CheckedBitfield::from(stream.read_u32_le()?)
                .ensure_none_set_unchecked()
                .map_err(CommonParseError::UnhandledInlineObjectListFlags)?;

            any_present
        } {
            read_u32_sized_vec!(stream, CommonParseError::TooManyElements, {
                let mut obj_stream = (&mut stream).take_exclusive_length_prefixed()?;

                let obj_size = obj_stream.read_u32_le()?;

                let object_type: u8 = {
                    // This could be a single byte...
                    let object_type = obj_stream.read_u32_le()?;
                    object_type
                        .try_into()
                        .map_err(|_| CommonParseError::ObjectTypeTooBig(object_type))?
                };

                let object = {
                    let mut inner_stream: BlindWindow<_> =
                        Window::new(&mut obj_stream, obj_size.into()).into();

                    let obj = DocObject::try_parse_with_ctx(
                        &mut inner_stream,
                        &DocObjectParseContext {
                            object_type,
                            doc_ctx,
                        },
                    )
                    .map_err(Box::new)?;

                    inner_stream
                        .ensure_eof()
                        .map_err(CommonParseError::ObjectSpanUnfinished)?;

                    obj
                };

                let span = InlineObject {
                    index_in_text: obj_stream.read_u32_le()?,
                    layout_option: if_any_left!(obj_stream, obj_stream.read_u32_le()?.try_into()?),
                    layout_constraint: if_any_left!(
                        obj_stream,
                        obj_stream.read_u32_le()?.try_into()?
                    ),
                    object,
                };

                obj_stream
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
            inline_objects,
        })
    }
}
