use crate::{
    byte_stream::{
        BoundedStream, ByteStreamLe, ReadBitfieldError, ReadStringError, TryParse,
        UnfinishedParsingError,
    },
    context::{DocumentContext, TryParseWithContext},
    impl_try_from_for_optional_from,
    page::{
        Point, Rect,
        object::{
            base::{HasObjectBase, ObjectBase},
            header::{ObjectHeader, ObjectHeaderError},
            shape_base::{ShapeBase, ShapeBaseParseError},
            shared::{ColourType, GradientColour, GradientType, Path, PathParseError},
            text_core::{self, CommonParseContext},
        },
    },
    read_size_and_vec, unpack_bool_flags, unpack_field_flags,
};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{self, Read, Seek};
use thiserror::Error;

#[derive(Debug, FromPrimitive)]
enum ShapeType {
    /// `TYPE_UNKNOWN`
    Unknown = 0,
    /// `TYPE_OVAL`
    Oval = 1,
    /// `TYPE_TRIANGLE`
    Triangle = 2,
    /// `TYPE_RIGHT_TRIANGLE`
    RightTriangle = 3,
    /// `TYPE_RECTANGLE`
    Rectangle = 4,
    /// `TYPE_ROUNDED_RECTANGLE`
    RoundedRectangle = 5,
    /// `TYPE_HEXAGON`
    Hexagon = 6,
    /// `TYPE_PARALLELOGRAM`
    Parallelogram = 7,
    /// `TYPE_DIAMOND`
    Diamond = 8,
    /// `TYPE_TRAPEZOID`
    Trapezoid = 9,
    /// `TYPE_PENTAGON`
    Pentagon = 10,
    /// `TYPE_REGULAR_PENTAGON`
    RegularPentagon = 11,
    /// `TYPE_4_POINT_STAR`
    Star4 = 12,
    /// `TYPE_5_POINT_STAR`
    Star5 = 13,
    /// `TYPE_8_POINT_STAR`
    Star8 = 14,
    /// `TYPE_10_POINT_STAR`
    Star10 = 15,
    /// `TYPE_32_POINT_STAR`
    Star32 = 16,
    /// `TYPE_CROSS`
    Cross = 17,
    /// `TYPE_L_SHAPE`
    LShape = 18,
    /// `TYPE_CHEVRON`
    Chevron = 19,
    /// `TYPE_ARC`
    Arc = 20,
    /// `TYPE_MOON`
    Moon = 21,
    /// `TYPE_SMILEY_FACE`
    SmileyFace = 22,
    /// `TYPE_HEART`
    Heart = 23,
    /// `TYPE_PIE`
    Pie = 24,
    /// `TYPE_CHORD`
    Chord = 25,
    /// `TYPE_CAN`
    Can = 26,
    /// `TYPE_CUBE`
    Cube = 27,
    /// `TYPE_EXPLOSION_1`
    Explosion1 = 28,
    /// `TYPE_EXPLOSION_2`
    Explosion2 = 29,
    /// `TYPE_FOLDED_CORNER`
    FoldedCorner = 30,
    /// `TYPE_LIGHTNING_BOLT`
    LightningBolt = 31,
    /// `TYPE_PLAQUE`
    Plaque = 32,
    /// `TYPE_UP_RIBBON`
    Ribbon = 33,
    /// `TYPE_DOWN_RIBBON`
    DownRibbon = 34,
    /// `TYPE_DONUT`
    Donut = 35,
    /// `TYPE_NO_SYMBOL`
    NoSymbol = 36,
    /// `TYPE_HORIZONTAL_SCROLL`
    HorizontalScroll = 37,
    /// `TYPE_VERTICAL_SCROLL`
    VerticalScroll = 38,
    /// `TYPE_BLOCK_ARC`
    BlockArc = 39,
    /// `TYPE_BEVEL`
    Bevel = 40,
    /// `TYPE_SUN`
    Sun = 41,
    /// `TYPE_WAVE`
    Wave = 42,
    /// `TYPE_DOUBLE_WAVE`
    DoubleWave = 43,
    /// `TYPE_LEFT_ARROW`
    LeftArrow = 44,
    /// `TYPE_RIGHT_ARROW`
    RightArrow = 45,
    /// `TYPE_UP_ARROW`
    UpArrow = 46,
    /// `TYPE_DOWN_ARROW`
    DownArrow = 47,
    /// `TYPE_LEFT_RIGHT_ARROW`
    LeftRightArrow = 48,
    /// `TYPE_UP_DOWN_ARROW`
    UpDownArrow = 49,
    /// `TYPE_LEFT_UP_ARROW`
    LeftUpArrow = 50,
    /// `TYPE_LEFT_RIGHT_UP_ARROW`
    LeftRightUpArrow = 51,
    /// `TYPE_QUAD_ARROW`
    QuadArrow = 52,
    /// `TYPE_BENT_ARROW`
    BentArrow = 53,
    /// `TYPE_BENT_UP_ARROW`
    BentUpArrow = 54,
    /// `TYPE_CURVED_LEFT_ARROW`
    CurvedLeftArrow = 55,
    /// `TYPE_CURVED_RIGHT_ARROW`
    CurvedRightArrow = 56,
    /// `TYPE_CURVED_UP_ARROW`
    CurvedUpArrow = 57,
    /// `TYPE_CURVED_DOWN_ARROW`
    CurvedDownArrow = 58,
    /// `TYPE_STRIPED_RIGHT_ARROW`
    StripedRightArrow = 59,
    /// `TYPE_NOTCHED_RIGHT_ARROW`
    NotchedRightArrow = 60,
    /// `TYPE_U_TURN_ARROW`
    UTurnArrow = 61,
    /// `TYPE_CIRCULAR_ARROW`
    CircularArrow = 62,
    /// `TYPE_FLOWCHART_MANUAL_INPUT`
    FlowchartManualInput = 63,
    /// `TYPE_FLOWCHART_TERMINATOR`
    FlowchartTerminator = 64,
    /// `TYPE_FLOWCHART_PREDEFINED_PROCESS`
    FlowchartPredefinedProcess = 65,
    /// `TYPE_FLOWCHART_STORED_DATA`
    FlowchartStoredData = 66,
    /// `TYPE_FLOWCHART_DELAY`
    FlowchartDelay = 67,
    /// `TYPE_FLOWCHART_CARD`
    FlowchartCard = 68,
    /// `TYPE_FLOWCHART_OFF_PAGE_CONNECTOR`
    FlowchartOffPageConnector = 69,
    /// `TYPE_FLOWCHART_DISPLAY`
    FlowchartDisplay = 70,
    /// `TYPE_FLOWCHART_DOCUMENT`
    FlowchartDocument = 71,
    /// `TYPE_FLOWCHART_PUNCHED_TAPE`
    FlowchartPunchedTape = 72,
    /// `TYPE_FLOWCHART_SEQUENTIAL_ACCESS_STORAGE`
    FlowchartSequentialAccessStorage = 73,
    /// `TYPE_LEFT_BRACE`
    LeftBrace = 74,
    /// `TYPE_RIGHT_BRACE`
    RightBrace = 75,
    /// `TYPE_LEFT_BRACKET`
    LeftBracket = 76,
    /// `TYPE_RIGHT_BRACKET`
    RightBracket = 77,
    /// `TYPE_RECTANGULAR_CALLOUT`
    RectangularCallout = 78,
    /// `TYPE_ROUNDED_RECTANGULAR_CALLOUT`
    RoundedRectangularCallout = 79,
    /// `TYPE_OVAL_CALLOUT`
    OvalCallout = 80,
    /// `TYPE_LEFT_ARROW_CALLOUT`
    LeftArrowCallout = 81,
    /// `TYPE_UP_ARROW_CALLOUT`
    UpArrowCallout = 82,
    /// `TYPE_RIGHT_ARROW_CALLOUT`
    RightArrowCallout = 83,
    /// `TYPE_DOWN_ARROW_CALLOUT`
    DownArrowCallout = 84,
    /// `TYPE_LEFT_RIGHT_ARROW_CALLOUT`
    LeftRightArrowCallout = 85,
    /// `TYPE_UP_DOWN_ARROW_CALLOUT`
    UpDownArrowCallout = 86,
    /// `TYPE_QUAD_ARROW_CALLOUT`
    QuadArrowCallout = 87,
    /// `TYPE_POLYLINE`
    Polyline = 88,
    /// `TYPE_POLYGON`
    Polygon = 89,
    /// `TYPE_CURVE`
    Curve = 90,
}

impl_try_from_for_optional_from!(ShapeType, u32, from_u32, pub InvalidShapeTypeError);

#[derive(Error, Debug)]
pub enum FillColourEffectParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("failed to read property flags")]
    PropertyFlags(#[from] ReadBitfieldError),

    #[error("invalid gradient type ID {0}")]
    BadGradientType(u8),
}

// C.f. `LineColourEffect`, which is very similar but is serialised differently.
#[derive(Debug)]
#[expect(dead_code)]
struct FillColourEffect {
    solid_colour: [u8; 4],
    colour_type: ColourType,
    gradient_rotatable: bool,
    gradient_type: GradientType,
    angle: i16,
    radial_gradient_pos: Point,
    colours: Vec<GradientColour>,
}

impl FillColourEffect {
    fn try_parse<T: ByteStreamLe>(
        stream: &mut T,
    ) -> Result<FillColourEffect, FillColourEffectParseError> {
        let property_flags = stream.read_variable_length_bitfield()?;

        // `unwrap` is fine here because the first bit can only be 0 or 1.
        let colour_type = ColourType::from_u32(property_flags & 1).unwrap();
        let gradient_rotatable = property_flags & 2 != 0;

        let solid_colour = stream.read_u32_le()?.to_le_bytes();

        let gradient_type = {
            let val = stream.read_u8()?;
            GradientType::from_u8(val).ok_or(FillColourEffectParseError::BadGradientType(val))?
        };

        let angle = stream.read_i16_le()?;
        let radial_gradient_pos = Point::try_parse_f32(stream)?;

        let col_count: usize = stream.read_u8()?.into();
        let mut colours = Vec::with_capacity(col_count);

        for _ in 0..col_count {
            colours.push(GradientColour::try_parse(stream)?);
        }

        Ok(FillColourEffect {
            solid_colour,
            colour_type,
            gradient_rotatable,
            gradient_type,
            angle,
            radial_gradient_pos,
            colours,
        })
    }
}

#[derive(Debug)]
#[expect(dead_code)]
struct FillImageEffect {
    image_type: u8,
    image_id: i32,
    nine_patch_rect: Rect,
    nine_patch_width: u32,
    stretch_offset: Rect,
    tiling_offset: Point,
    tiling_scale_x: f32,
    tiling_scale_y: f32,
    alpha: f32,
    rotatable: bool,
}

impl FillImageEffect {
    fn try_parse(stream: &mut impl ByteStreamLe) -> io::Result<FillImageEffect> {
        Ok(FillImageEffect {
            image_type: stream.read_u8()?,
            image_id: stream.read_i32_le()?,
            stretch_offset: Rect::try_parse_f32(stream)?,
            tiling_offset: Point::try_parse_f32(stream)?,
            tiling_scale_x: stream.read_f32_le()?,
            tiling_scale_y: stream.read_f32_le()?,
            alpha: stream.read_f32_le()?,
            rotatable: stream.read_u8()? != 0,
            nine_patch_rect: Rect::try_parse_i32(stream)?,
            nine_patch_width: stream.read_u32_le()?,
        })
    }
}

#[derive(Error, Debug)]
pub enum FillEffectParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("invalid fill effect type {0}")]
    BadEffectType(u8),

    #[error("failed to parse fill colour effect")]
    ColourEffect(#[from] FillColourEffectParseError),
}

#[derive(Debug)]
#[expect(dead_code)]
enum FillEffect {
    Background {
        transparency: f32,
    },

    Colour(FillColourEffect),
    Image(FillImageEffect),

    Pattern {
        pattern: [u8; 8],
        foreground_colour: [u8; 4],
        background_colour: [u8; 4],
    },
}

impl FillEffect {
    fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<FillEffect, FillEffectParseError> {
        let _effect_size = stream.read_u32_le()?;
        let effect_type = stream.read_u8()?;

        match effect_type {
            1 => Ok(FillEffect::Colour(FillColourEffect::try_parse(stream)?)),
            2 => Ok(FillEffect::Image(FillImageEffect::try_parse(stream)?)),

            3 => Ok(FillEffect::Pattern {
                pattern: stream.read_u64_le()?.to_le_bytes(),
                foreground_colour: stream.read_u32_le()?.to_le_bytes(),
                background_colour: stream.read_u32_le()?.to_le_bytes(),
            }),

            4 => Ok(FillEffect::Background {
                transparency: stream.read_f32_le()?,
            }),

            bad => Err(FillEffectParseError::BadEffectType(bad)),
        }
    }
}

#[derive(Debug, FromPrimitive)]
pub enum BorderType {
    /// `BORDER_TYPE_NONE`
    None = 0,
    /// `BORDER_TYPE_SQUARE`
    Square = 1,
    /// `BORDER_TYPE_SHADOW`
    Shadow = 2,
    /// `BORDER_TYPE_DOT`
    Dot = 3,
    /// `BORDER_TYPE_IMAGE`
    Image = 4,
}

impl_try_from_for_optional_from!(BorderType, u16, from_u16, pub InvalidBorderTypeError);

#[derive(Debug)]
#[expect(dead_code)]
struct Template {
    is_flipped_horizontally: bool,
    is_flipped_vertically: bool,
    owner_rect: Rect,
    rotation: f32,
    path: Path,
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct ShapeData {
    shape_type: ShapeType,
    fill_effect: Option<FillEffect>,
    template: Option<Template>,
    pub border_colour: Option<[u8; 4]>,
    pub border_width: Option<f32>,
    pub border_type: Option<BorderType>,
    original_drawn_rect: Option<Rect>,
    pub original_rect: Rect,
    pub original_angle: f32,
}

#[derive(Debug)]
#[expect(dead_code)]
struct Pen {
    pen_name_id: Option<u32>,
    default_pen_name_id: Option<u32>,
    style_id: Option<u32>,
}

#[derive(Debug, FromPrimitive)]
enum TextAreaType {
    /// `TEXT_AREA_TYPE_MARGIN`
    Margin = 0,
    /// `TEXT_AREA_TYPE_FREE`
    Free = 1,
    /// `TEXT_AREA_TYPE_PATH`
    Path = 2,
}

impl_try_from_for_optional_from!(TextAreaType, u8, from_u8, pub InvalidTextAreaTypeError);

#[derive(Debug, FromPrimitive)]
enum HintTextStyle {
    /// `HINT_TEXT_STYLE_NONE` = 0;
    None = 0,
    /// `HINT_TEXT_STYLE_BOLD` = 1;
    Bold = 1,
    /// `HINT_TEXT_STYLE_ITALIC` = 2;
    Italic = 2,
    /// `HINT_TEXT_STYLE_UNDERLINE` = 4;
    Underline = 4,
    /// `HINT_TEXT_STYLE_MASK` = 7;
    Mask = 7,
}

impl_try_from_for_optional_from!(HintTextStyle, u8, from_u8, pub InvalidHintTextStyleError);

#[derive(Debug, FromPrimitive)]
enum ImeActionType {
    /// `IME_ACTION_TYPE_UNSPECIFIED`
    Unspecified = 0,
    /// `IME_ACTION_TYPE_NONE`
    None = 1,
    /// `IME_ACTION_TYPE_GO`
    Go = 2,
    /// `IME_ACTION_TYPE_SEARCH`
    Search = 3,
    /// `IME_ACTION_TYPE_DONE`
    Done = 4,
    /// `IME_ACTION_TYPE_SEND`
    Send = 5,
    /// `IME_ACTION_TYPE_NEXT`
    Next = 6,
    /// `IME_ACTION_TYPE_PREVIOUS`
    Previous = 7,
}

impl_try_from_for_optional_from!(ImeActionType, u8, from_u8, pub InvalidImeActionTypeError);

#[derive(Debug, FromPrimitive)]
enum TextInputType {
    /// `INPUT_TYPE_NONE`
    None = 0,
    /// `INPUT_TYPE_TEXT`
    Text = 1,
    /// `INPUT_TYPE_NUMBER`
    Number = 2,
    /// `INPUT_TYPE_PHONE`
    Phone = 3,
    /// `INPUT_TYPE_DATETIME`
    Datetime = 4,
}

impl_try_from_for_optional_from!(TextInputType, u8, from_u8, pub InvalidTextInputTypeError);

#[derive(Debug, FromPrimitive)]
enum EllipsisType {
    /// `ELLIPSIS_TYPE_NONE`
    None = 0,
    /// `ELLIPSIS_TYPE_DOTS`
    Dots = 1,
    /// `ELLIPSIS_TYPE_TRIANGLE`
    Triangle = 2,
}

impl_try_from_for_optional_from!(EllipsisType, u8, from_u8, pub InvalidEllipsisTypeError);

#[derive(Debug, FromPrimitive)]
enum TextAutoFitType {
    /// `AUTO_FIT_OPTION_NONE`
    None = 0,
    /// `AUTO_FIT_OPTION_HORIZONTAL`
    Horizontal = 1,
    /// `AUTO_FIT_OPTION_VERTICAL`
    Vertical = 2,
    /// `AUTO_FIT_OPTION_BOTH`
    Both = 3,
}

impl_try_from_for_optional_from!(TextAutoFitType, u8, from_u8, pub InvalidTextAutoFitTypeError);

#[derive(Debug)]
#[expect(dead_code)]
pub struct TextData {
    pub text_common: Option<text_core::Common>,
    text_area_type: Option<TextAreaType>,
    hint_text: Option<String>,
    hint_text_vertical_offset: Option<f32>,
    hint_text_style: Option<HintTextStyle>,
    is_hint_text_visible: bool,
    is_read_only: bool,
    is_text_editable: bool,
    hint_text_font_size: Option<f32>,
    hint_text_colour: Option<[u8; 4]>,
    ime_action_type: Option<ImeActionType>,
    text_input_type: TextInputType,
    ellipsis_type: Option<EllipsisType>,
    text_auto_fit_type: Option<TextAutoFitType>,
    lined_paper_thickness: Option<f32>,
    lined_paper_colour: Option<[u8; 4]>,
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct ImageData {
    transparency: bool,

    border_image_hash: Option<String>,
    pub border_image_nine_patch_width: Option<u32>,
    original_image_hash: Option<String>,
    pub crop_rect: Option<Rect>,
    pub border_line_width: Option<Rect>,
    pub border_image_bind_id: Option<u32>,
    pub border_image_nine_patch_rect: Option<Rect>,
    compat_image_id: Option<u32>,
    pub original_image_id: Option<u32>,
    pub original_rect: Option<Rect>,
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ShapeParseError {
    Io(#[from] io::Error),
    Base(#[from] ShapeBaseParseError),
    Header(#[from] ObjectHeaderError),
    BadShapeType(#[from] InvalidShapeTypeError),

    #[error("failed to parse template path")]
    TemplatePath(#[source] PathParseError),

    #[error("template path stream was not exhausted")]
    UnfinishedPath(#[source] UnfinishedParsingError),

    TextCommon(#[from] text_core::CommonParseError),
    BadTextAreaType(#[from] InvalidTextAreaTypeError),
    FillEffect(#[from] FillEffectParseError),

    #[error("failed to read hint text")]
    HintText(#[source] ReadStringError),

    BadHintTextStyle(#[from] InvalidHintTextStyleError),
    BadEllipsisType(#[from] InvalidEllipsisTypeError),
    BadTextAutoFitType(#[from] InvalidTextAutoFitTypeError),
    BadImeActionType(#[from] InvalidImeActionTypeError),
    BadTextInputType(#[from] InvalidTextInputTypeError),
    Unfinished(#[from] UnfinishedParsingError),
}

pub struct ShapeParseContext<'fr, 'sr> {
    pub is_shape_only: bool,
    pub doc_ctx: DocumentContext<'fr, 'sr>,
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Shape {
    shape_base: ShapeBase,
    pub(crate) shape_data: ShapeData,
    pen: Pen,
    pub(crate) text_data: TextData,
    pub(crate) image_data: ImageData,

    control_points: Vec<Point>,
}

impl<'a, R: Read + Seek> TryParseWithContext<R, ShapeParseContext<'a, 'a>> for Shape {
    type ParseError = ShapeParseError;

    fn try_parse_with_ctx(
        stream: &mut R,
        &ShapeParseContext {
            is_shape_only,
            doc_ctx,
        }: &ShapeParseContext<'a, 'a>,
    ) -> Result<Shape, ShapeParseError> {
        let shape_base = ShapeBase::try_parse(stream)?;

        let (mut header, mut stream) = ObjectHeader::try_parse(stream, 7)?;

        let property_flags = header.property_flags_mut();

        unpack_bool_flags!(property_flags, {
            0 => template_is_flipped_horizontally;
            1 => template_is_flipped_vertically;
            2 => text_is_editable;
            3 => is_hint_text_visible;
            4 => text_is_read_only;
            5 => image_transparency;
        });

        let shape_type: ShapeType = stream.read_u32_le()?.try_into()?;
        let original_rect = Rect::try_parse_f64(&mut stream)?;
        let original_angle = stream.read_f32_le()?;

        // Only read the path if its size is >0.
        let template = if let path_bin_size @ 1.. = stream.read_u32_le()? {
            let mut stream = (&mut stream).take(path_bin_size.into());
            let path = Path::try_parse(&mut stream).map_err(ShapeParseError::TemplatePath)?;
            stream
                .ensure_eof()
                .map_err(ShapeParseError::UnfinishedPath)?;

            Some(Template {
                is_flipped_horizontally: template_is_flipped_horizontally,
                is_flipped_vertically: template_is_flipped_vertically,
                owner_rect: original_rect,
                rotation: original_angle,
                path,
            })
        } else {
            None
        };

        let control_points = read_size_and_vec!(stream, u8, Point::try_parse_f64(&mut stream)?);

        // This field exists iff the object type is 7, i.e. iff this is a pure shape object, and
        // not a subclass (like a text box or image).
        let original_drawn_rect = is_shape_only
            .then(|| Rect::try_parse_f64(&mut stream))
            .transpose()?;

        let field_flags = header.init_flex(&mut stream)?;

        // todo: SPen::ObjectShapeTemplateFactory::NewTemplate

        unpack_field_flags!(field_flags, {
            0 => text_common: text_core::Common::try_parse_with_ctx(
                &mut stream,
                &CommonParseContext {
                    format_version: shape_base.object_base().format_version,
                    doc_ctx
                },
            )?;

            1 => text_area_type: stream.read_u8()?.try_into()?;

            2 => pen_name_id: stream.read_u32_le()?;
            3 => default_pen_name_id: stream.read_u32_le()?;
            4 => style_id: stream.read_u32_le()?;

            5 => fill_effect: FillEffect::try_parse(&mut stream)?;

            // SPen::ObjectShapeImage::ApplyBinary_BorderData
            6 => _border_1: stream.seek_relative(4)?;
            7 => _border_2: stream.seek_relative(4)?;
            8 => _border_3: stream.seek_relative(2)?;

            9 => hint_text: stream.read_short_u16_string().map_err(ShapeParseError::HintText)?;
            10 => hint_text_colour: stream.read_4_bytes()?;
            11 => hint_text_font_size: stream.read_f32_le()?;
            22 => hint_text_style: stream.read_u8()?.try_into()?;

            12 => ellipsis_type: stream.read_u8()?.try_into()?;
            13 => text_auto_fit_type: stream.read_u8()?.try_into()?;
            14 => ime_action_type: stream.read_u8()?.try_into()?;
            15 => text_input_type: stream.read_u8()?.try_into()?, else TextInputType::Text;

            // SPen::ObjectShapeImage::ApplyBinary_Deprecated
            16 => _dep_1: stream.seek_relative(16)?;
            17 => _dep_2: stream.seek_relative(4)?;
            18 => _dep_3: stream.seek_relative(16)?;
            19 => _dep_4: stream.seek_relative(16)?;
            20 => _dep_5: stream.seek_relative(4)?;

            21 => hint_text_vertical_offset: stream.read_f32_le()?;
            // already checked 22
            23 => lined_paper_thickness: stream.read_f32_le()?;
            24 => lined_paper_colour: stream.read_4_bytes()?;
        });

        // todo: See SPen::ObjectShapeBinaryHandler::ApplyOwnBinary_ShapeRefresh (_document.dll)

        header.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Shape {
            shape_base,
            shape_data: ShapeData {
                shape_type,
                fill_effect,
                template,
                border_colour: None,
                border_width: None,
                border_type: None,
                original_drawn_rect,
                original_rect,
                original_angle,
            },
            pen: Pen {
                pen_name_id,
                default_pen_name_id,
                style_id,
            },
            text_data: TextData {
                text_common,
                text_area_type,
                hint_text,
                hint_text_vertical_offset,
                hint_text_style,
                is_hint_text_visible,
                is_read_only: text_is_read_only,
                is_text_editable: text_is_editable,
                hint_text_font_size,
                hint_text_colour,
                ime_action_type,
                text_input_type,
                ellipsis_type,
                text_auto_fit_type,
                lined_paper_thickness,
                lined_paper_colour,
            },
            image_data: ImageData {
                transparency: image_transparency,

                border_image_hash: None,
                border_image_nine_patch_width: None,
                original_image_hash: None,
                crop_rect: None,
                border_line_width: None,
                border_image_bind_id: None,
                border_image_nine_patch_rect: None,
                compat_image_id: None,
                original_image_id: None,
                original_rect: None,
            },
            control_points,
        })
    }
}

impl Shape {
    pub fn raw_text_string(&self) -> Option<&str> {
        self.text_data
            .text_common
            .as_ref()
            .map(|tc| tc.raw_string())
    }
}

impl HasObjectBase for Shape {
    fn object_base(&self) -> &ObjectBase {
        self.shape_base.object_base()
    }
}
