use crate::{
    byte_stream::{ByteStreamLe, ReadBitfieldError, ReadStringError, WrongEndOffsetError},
    page::{
        Point, Rect,
        object::{
            InheritsObjectBase, ObjectBase,
            shape_base::ShapeBase,
            shared::{ColourType, GradientColour, GradientType, Path, PathParseError},
            text,
        },
    },
};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::io::{self, Seek, SeekFrom};
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

struct FillBackgroundEffect {
    alpha: f32,
}

#[derive(Error, Debug)]
enum FillColourEffectParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("invalid colour type ID {0}")]
    BadColourType(u8),

    #[error("invalid gradient type ID {0}")]
    BadGradientType(u8),
}

// C.f. `LineColourEffect`, which is very similar but is serialised differently.
struct FillColourEffect {
    solid_colour: [u8; 4],
    colour_type: ColourType,
    gradient_rotatable: bool,
    gradient_type: GradientType,
    angle: f32,
    radial_gradient_pos: Point,
    colours: Vec<GradientColour>,
}

#[derive(Error, Debug)]
enum FillImageEffectParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("failed to read image hash")]
    BadHash(ReadStringError),
}

struct FillImageEffect {
    image_type: u8,
    image_id: u32,
    image_hash: String,
    nine_patch_rect: Rect,
    nine_patch_width: u32,
    stretch_offset: Rect,
    tiling_offset: Rect,
    tiling_scale_x: f32,
    tiling_scale_y: f32,
    alpha: f32,
    rotatable: bool,
}

#[derive(Error, Debug)]
enum FillPatternEffectParseError {
    #[error("io error")]
    Io(#[from] io::Error),
}

struct FillPatternEffect {
    pattern: Vec<u8>,
    foreground_colour: [u8; 4],
    background_colour: [u8; 4],
}

#[derive(Debug, FromPrimitive)]
enum BorderType {
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

struct Template {
    is_flipped_horizontally: bool,
    is_flipped_vertically: bool,
    owner_rect: Rect,
    rotation: f32,
    path: Path,
}

struct Data {
    shape_type: ShapeType,
    fill_background_effect: FillBackgroundEffect,
    fill_colour_effect: FillColourEffect,
    fill_image_effect: FillImageEffect,
    fill_pattern_effect: FillPatternEffect,
    template: Template,
    border_colour: [u8; 4],
    border_width: f32,
    border_type: BorderType,
    original_drawn_rect: Rect,
    original_rect: Rect,
    original_angle: f32,
}

struct Pen {
    pen_name_id: Option<u32>,
    default_pen_name_id: Option<u32>,
    file_id: Option<u32>,
}

enum TextAreaType {
    /// `TEXT_AREA_TYPE_MARGIN`
    Margin = 0,
    /// `TEXT_AREA_TYPE_FREE`
    Free = 1,
    /// `TEXT_AREA_TYPE_PATH`
    Path = 2,
}

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

enum EllipsisType {
    /// `ELLIPSIS_TYPE_NONE`
    None = 0,
    /// `ELLIPSIS_TYPE_DOTS`
    Dots = 1,
    /// `ELLIPSIS_TYPE_TRIANGLE`
    Triangle = 2,
}

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

struct Text {
    text_common: text::Common,
    text_area_type: TextAreaType,
    hint_text: String,
    hint_text_vertical_offset: f32,
    hint_text_style: HintTextStyle,
    text_binary_size: u32,
    is_hint_text_visible: bool,
    text_read_only: bool,
    is_text_editable: bool,
    hint_text_font_size: f32,
    hint_text_colour: [u8; 4],
    ime_action_type: ImeActionType,
    text_input_type: TextInputType,
    ellipsis_type: EllipsisType,
    text_auto_fit: TextAutoFitType,
}

struct Image {
    border_image_hash: String,
    border_image_nine_patch_width: u32,
    original_image_hash: String,
    crop_rect: Rect,
    image_border_line_width: Rect,
    border_image_id: u32,
    border_image_nine_patch_rect: Rect,
    compat_image_id: u32,
    original_image_id: u32,
    transparency: bool,
    original_rect: Rect,
}

#[derive(Error, Debug)]
enum ShapeParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("failed to parse shape base")]
    ShapeBase(color_eyre::Report),

    #[error("invalid data type {0} for shape object (should be 7)")]
    BadDataType(u16),

    #[error("failed to parse property flags")]
    PropertyFlags(ReadBitfieldError),

    #[error("failed to parse field check flags")]
    FieldCheckFlags(ReadBitfieldError),

    #[error("invalid shape type ID {0}")]
    BadShapeType(u32),

    #[error("failed to parse fill colour effect")]
    FillColourEffect(#[from] FillColourEffectParseError),

    #[error("failed to parse fill image effect")]
    FillImageEffect(#[from] FillImageEffectParseError),

    #[error("failed to parse fill pattern effect")]
    FillPatternEffect(#[from] FillPatternEffectParseError),

    #[error("invalid border type ID {0}")]
    BadBorderType(u8),

    #[error("failed to parse template path")]
    TemplatePath(PathParseError),

    #[error("failed to parse common text data")]
    TextCommon(#[from] text::CommonParseError),

    #[error("parsed wrong number of bytes")]
    BadEndOffset(#[from] WrongEndOffsetError),
}

struct Shape {
    base: ShapeBase,
    data: Data,
    pen: Pen,
    text: Text,
    image: Image,

    control_points: Vec<Point>,
    span_order_data: Vec<String>,
}

// fixme: This should be `impl InheritsObjectBase` (with `ConcreteInheritsObjectBase` after), but
// the error types are incompatible.
impl Shape {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> Result<Shape, ShapeParseError> {
        let shape_base = ShapeBase::try_parse(stream, object_base, child_count)
            .map_err(ShapeParseError::ShapeBase)?;

        let start_offset = stream.stream_position()?;

        let expected_end = {
            let size: u64 = stream.read_u32_le()?.into();
            start_offset + size
        };

        match stream.read_u16_le()? {
            7 => (),
            bad => return Err(ShapeParseError::BadDataType(bad)),
        }

        let flex_offset: u64 = stream.read_u32_le()?.into();

        let property_flags = stream
            .read_variable_length_bitfield()
            .map_err(ShapeParseError::PropertyFlags)?;

        let template_is_flipped_horizontally = property_flags & 1 != 0;
        let template_is_flipped_vertically = property_flags & 2 != 0;
        let text_is_editable = property_flags & 4 != 0;
        let text_is_hint_text_visible = property_flags & 8 != 0;
        let text_is_read_only = property_flags & 16 != 0;
        let image_transparency = property_flags & 32 != 0;

        let stated_field_check_flags = stream
            .read_variable_length_bitfield()
            .map_err(ShapeParseError::FieldCheckFlags)?;

        let data_shape_type = {
            let val = stream.read_u32_le()?;
            ShapeType::from_u32(val).ok_or(ShapeParseError::BadShapeType(val))?
        };

        let data_original_rect = Rect::try_parse_f64(stream)?;
        let data_original_rotation = stream.read_f32_le()?;

        let template_path_size = stream.read_u32_le()?;

        let template = if template_path_size > 0 {
            Some(Template {
                is_flipped_horizontally: template_is_flipped_horizontally,
                is_flipped_vertically: template_is_flipped_vertically,
                owner_rect: data_original_rect,
                rotation: data_original_rotation,
                path: Path::try_parse(stream).map_err(ShapeParseError::TemplatePath)?,
            })
        } else {
            None
        };

        let control_points = {
            let count: usize = stream.read_u8()?.into();
            let mut points = Vec::with_capacity(count);

            for _ in 0..count {
                points.push(Point::try_parse_f32(stream)?);
            }

            points
        };

        let data_original_draw_rect = Rect::try_parse_f64(stream)?;

        let field_check_flags = if flex_offset != 0 {
            stream.seek(SeekFrom::Start(start_offset + flex_offset))?;
            stated_field_check_flags
        } else {
            0
        };

        let text_common = (field_check_flags & 1 != 0)
            .then(|| text::Common::try_parse(stream, shape_base.object_base.format_version))
            .transpose()?;

        // todo: Continue from here:
        /*
           if ((fieldCheckFlags_ & 2) != 0) {
               wCon_ObjectShapeText.textAreaType = ((ByteBuffer) wDocBuffer.byteBuffer).get(i6);
               i6++;
           }
        */

        let actual_end = stream.stream_position()?;

        if actual_end != expected_end {
            return Err(WrongEndOffsetError {
                actual_end,
                expected_end,
            }
            .into());
        }

        todo!()
    }
}
