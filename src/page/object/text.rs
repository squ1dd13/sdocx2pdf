use crate::page::object::DocObject;

enum Gravity {
    /// `GRAVITY_TOP`
    Top = 0,
    /// `GRAVITY_CENTER`
    Centre = 1,
    /// `GRAVITY_BOTTOM`
    Bottom = 2,
}

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

enum SpanIntervalType {
    //// `SPAN_INCLUSIVE_EXCLUSIVE`
    InclusiveExclusive = 0,
    /// `SPAN_INCLUSIVE_INCLUSIVE`
    InclusiveInclusive = 1,
    /// `SPAN_EXCLUSIVE_EXCLUSIVE`
    ExclusiveExclusive = 2,
    /// `SPAN_EXCLUSIVE_INCLUSIVE`
    ExclusiveInclusive = 3,
}

struct SpanBase {
    span_type: SpanType,
    start_pos: u32,
    end_pos: u32,
    interval_type: SpanIntervalType,
}

struct Span {
    span_base: SpanBase,
    bytes: Vec<u8>,
}

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

struct ParagraphBase {
    paragraph_type: ParagraphType,
    start_pos: u32,
    end_pos: u32,
}

struct Paragraph {
    paragraph_base: ParagraphBase,
    bytes: Vec<u8>,
}

// todo: Contained DocObject may need to be boxed in the future to avoid recursion, depending on
// what DocObject ends up looking like.

struct DocObjectSpan {
    object: DocObject,
    start: u32,
    end: u32,
}

pub struct Common {
    text: String,
    left_margin: f32,
    top_margin: f32,
    right_margin: f32,
    bottom_margin: f32,
    gravity: Gravity,

    spans: Vec<Span>,
    paragraphs: Vec<Paragraph>,
    object_spans: Vec<DocObjectSpan>,
    section_data: Vec<(u32, u32)>,
}
