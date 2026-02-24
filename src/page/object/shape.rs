use crate::{
    byte_stream::{ByteStreamLe, ReadBitfieldError, ReadStringError, WrongEndOffsetError},
    page::{
        Point, Rect,
        object::{
            ConcreteInheritsObjectBase, InheritsObjectBase, ObjectBase,
            shape_base::ShapeBase,
            shared::{ColourType, GradientColour, GradientType, Path, PathParseError},
            text,
        },
    },
};
use num::FromPrimitive;
use num_derive::FromPrimitive;
use std::{
    hint,
    io::{self, Seek, SeekFrom},
};
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

#[derive(Error, Debug)]
enum FillColourEffectParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("failed to read property flags")]
    PropertyFlags(#[from] ReadBitfieldError),

    #[error("invalid gradient type ID {0}")]
    BadGradientType(u8),
}

// C.f. `LineColourEffect`, which is very similar but is serialised differently.
#[derive(Debug)]
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
enum FillEffectParseError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("invalid fill effect type {0}")]
    BadEffectType(u8),

    #[error("failed to parse fill colour effect")]
    ColourEffect(#[from] FillColourEffectParseError),
}

#[derive(Debug)]
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

#[derive(Debug)]
struct Template {
    is_flipped_horizontally: bool,
    is_flipped_vertically: bool,
    owner_rect: Rect,
    rotation: f32,
    path: Path,
}

#[derive(Debug)]
struct Data {
    shape_type: ShapeType,
    fill_effect: Option<FillEffect>,
    template: Option<Template>,
    border_colour: Option<[u8; 4]>,
    border_width: Option<f32>,
    border_type: Option<BorderType>,
    original_drawn_rect: Rect,
    original_rect: Rect,
    original_angle: f32,
}

#[derive(Debug)]
struct Pen {
    pen_name_id: Option<u32>,
    default_pen_name_id: Option<u32>,
    file_id: Option<u32>,
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

#[derive(Debug, FromPrimitive)]
enum EllipsisType {
    /// `ELLIPSIS_TYPE_NONE`
    None = 0,
    /// `ELLIPSIS_TYPE_DOTS`
    Dots = 1,
    /// `ELLIPSIS_TYPE_TRIANGLE`
    Triangle = 2,
}

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

#[derive(Debug)]
struct Text {
    text_common: Option<text::Common>,
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
}

#[derive(Debug)]
struct Image {
    transparency: bool,

    border_image_hash: Option<String>,
    border_image_nine_patch_width: Option<u32>,
    original_image_hash: Option<String>,
    crop_rect: Option<Rect>,
    image_border_line_width: Option<Rect>,
    border_image_id: Option<u32>,
    border_image_nine_patch_rect: Option<Rect>,
    compat_image_id: Option<u32>,
    original_image_id: Option<u32>,
    original_rect: Option<Rect>,
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

    #[error("invalid border type ID {0}")]
    BadBorderType(u8),

    #[error("failed to parse template path")]
    TemplatePath(PathParseError),

    #[error("failed to parse common text data")]
    TextCommon(#[from] text::CommonParseError),

    #[error("invalid text area type {0}")]
    BadTextAreaType(u8),

    #[error("failed to parse fill effect")]
    FillEffect(#[from] FillEffectParseError),

    #[error("failed to read hint text")]
    HintText(ReadStringError),

    #[error("invalid hint text style {0}")]
    BadHintTextStyle(u8),

    #[error("invalid ellipsis type {0}")]
    BadEllipsisType(u8),

    #[error("invalid text auto-fit type {0}")]
    BadTextAutoFitType(u8),

    #[error("invalid ime action type {0}")]
    BadImeActionType(u8),

    #[error("invalid text input type {0}")]
    BadTextInputType(u8),

    #[error("parsed wrong number of bytes")]
    BadEndOffset(#[from] WrongEndOffsetError),
}

#[derive(Debug)]
pub struct ShapeObject {
    base: ShapeBase,
    data: Data,
    pen: Pen,
    text: Text,
    image: Image,

    control_points: Vec<Point>,

    unk_32_1: Option<u32>,
    unk_32_2: Option<u32>,
    unk_16: Option<u16>,
}

impl ShapeObject {
    fn try_parse_inner<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> Result<ShapeObject, ShapeParseError> {
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
        let is_hint_text_visible = property_flags & 8 != 0;
        let text_is_read_only = property_flags & 16 != 0;
        let image_transparency = property_flags & 32 != 0;

        let stated_field_check_flags = stream
            .read_variable_length_bitfield()
            .map_err(ShapeParseError::FieldCheckFlags)?;

        let shape_type = {
            let val = stream.read_u32_le()?;
            ShapeType::from_u32(val).ok_or(ShapeParseError::BadShapeType(val))?
        };

        let original_rect = Rect::try_parse_f64(stream)?;
        let original_angle = stream.read_f32_le()?;

        let template_path_size = stream.read_u32_le()?;

        let template = if template_path_size > 0 {
            Some(Template {
                is_flipped_horizontally: template_is_flipped_horizontally,
                is_flipped_vertically: template_is_flipped_vertically,
                owner_rect: original_rect,
                rotation: original_angle,
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

        let original_drawn_rect = Rect::try_parse_f64(stream)?;

        let field_check_flags = if flex_offset != 0 {
            stream.seek(SeekFrom::Start(start_offset + flex_offset))?;
            stated_field_check_flags
        } else {
            0
        };

        let text_common = (field_check_flags & 1 != 0)
            .then(|| text::Common::try_parse(stream, shape_base.object_base.format_version))
            .transpose()?;

        let text_area_type = if field_check_flags & 2 != 0 {
            let val = stream.read_u8()?;
            Some(TextAreaType::from_u8(val).ok_or(ShapeParseError::BadTextAreaType(val))?)
        } else {
            None
        };

        let pen = Pen {
            pen_name_id: (field_check_flags & 4 != 0)
                .then(|| stream.read_u32_le())
                .transpose()?,
            default_pen_name_id: (field_check_flags & 8 != 0)
                .then(|| stream.read_u32_le())
                .transpose()?,
            file_id: (field_check_flags & 16 != 0)
                .then(|| stream.read_u32_le())
                .transpose()?,
        };

        let fill_effect = (field_check_flags & 32 != 0)
            .then(|| FillEffect::try_parse(stream))
            .transpose()?;

        let unk_32_1 = (field_check_flags & 64 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let unk_32_2 = (field_check_flags & 128 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?;

        let unk_16 = (field_check_flags & 256 != 0)
            .then(|| stream.read_u16_le())
            .transpose()?;

        if unk_32_1.is_some() || unk_32_2.is_some() || unk_16.is_some() {
            eprintln!(
                "Warning: Read at least one unknown shape field: {:?}/{:?}/{:?}",
                unk_32_1, unk_32_2, unk_16
            );
        }

        let hint_text = (field_check_flags & 512 != 0)
            .then(|| stream.read_short_u16_string())
            .transpose()
            .map_err(ShapeParseError::HintText)?;

        let hint_text_colour = (field_check_flags & 1024 != 0)
            .then(|| stream.read_u32_le())
            .transpose()?
            .map(u32::to_le_bytes);

        let hint_text_font_size = (field_check_flags & 2048 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?;

        let hint_text_style = if field_check_flags & 0x400000 != 0 {
            let val = stream.read_u8()?;
            Some(HintTextStyle::from_u8(val).ok_or(ShapeParseError::BadHintTextStyle(val))?)
        } else {
            None
        };

        let ellipsis_type = if field_check_flags & 4096 != 0 {
            let val = stream.read_u8()?;
            Some(EllipsisType::from_u8(val).ok_or(ShapeParseError::BadEllipsisType(val))?)
        } else {
            None
        };

        let text_auto_fit_type = if field_check_flags & 8192 != 0 {
            let val = stream.read_u8()?;
            Some(TextAutoFitType::from_u8(val).ok_or(ShapeParseError::BadTextAutoFitType(val))?)
        } else {
            None
        };

        let ime_action_type = if field_check_flags & 16384 != 0 {
            let val = stream.read_u8()?;
            Some(ImeActionType::from_u8(val).ok_or(ShapeParseError::BadImeActionType(val))?)
        } else {
            None
        };

        let text_input_type = if field_check_flags & 32768 != 0 {
            let val = stream.read_u8()?;
            TextInputType::from_u8(val).ok_or(ShapeParseError::BadTextInputType(val))?
        } else {
            TextInputType::Text
        };

        let hint_text_vertical_offset = (field_check_flags & 0x200000 != 0)
            .then(|| stream.read_f32_le())
            .transpose()?;

        let actual_end = stream.stream_position()?;

        if actual_end != expected_end {
            return Err(WrongEndOffsetError {
                actual_end,
                expected_end,
            }
            .into());
        }

        Ok(ShapeObject {
            base: shape_base,
            data: Data {
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
            pen,
            text: Text {
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
            },
            image: Image {
                transparency: image_transparency,

                border_image_hash: None,
                border_image_nine_patch_width: None,
                original_image_hash: None,
                crop_rect: None,
                image_border_line_width: None,
                border_image_id: None,
                border_image_nine_patch_rect: None,
                compat_image_id: None,
                original_image_id: None,
                original_rect: None,
            },
            control_points,
            unk_32_1,
            unk_32_2,
            unk_16,
        })
    }
}

impl InheritsObjectBase for ShapeObject {
    fn try_parse<T: ByteStreamLe + Seek>(
        stream: &mut T,
        object_base: ObjectBase,
        child_count: u16,
    ) -> color_eyre::eyre::Result<ShapeObject> {
        Ok(ShapeObject::try_parse_inner(
            stream,
            object_base,
            child_count,
        )?)
    }

    fn object_base(&self) -> &ObjectBase {
        &self.base.object_base
    }
}

impl ConcreteInheritsObjectBase for ShapeObject {}
