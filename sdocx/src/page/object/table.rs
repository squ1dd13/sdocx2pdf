use std::io::{self, Read, Seek};

use thiserror::Error;

use crate::{
    byte_stream::{BoundedStream, ByteStreamLe, TryParse, UnfinishedParsingError},
    context::{DocumentContext, TryParseWithContext},
    page::{
        Rect,
        object::{
            base::{HasObjectBase, ObjectBase},
            header::{FlagBlock, FlagBlockError, ObjectHeaderError, try_parse_object_header},
            shape::{
                InvalidTextAutoFitTypeError, Shape, ShapeParseContext, ShapeParseError,
                TextAutoFitType,
            },
            text::{Text, TextParseError},
        },
    },
    read_u32_sized_vec, unpack_bool_flags, unpack_field_flags,
};

#[derive(Debug)]
#[expect(dead_code)]
pub struct SingleBorderStyle {
    colour: u32,
    width: f32,
    start_radius: f32,
    end_radius: f32,
}

impl<R: Read> TryParse<R> for SingleBorderStyle {
    type ParseError = io::Error;

    fn try_parse(stream: &mut R) -> io::Result<SingleBorderStyle> {
        Ok(SingleBorderStyle {
            colour: stream.read_u32_le()?,
            width: stream.read_f32_le()?,
            start_radius: stream.read_f32_le()?,
            end_radius: stream.read_f32_le()?,
        })
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum FullBorderStyleParseError {
    Io(#[from] io::Error),
    FlagBlock(#[from] FlagBlockError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("failed to parse single border style")]
    SingleBorderStyle(#[source] io::Error),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct FullBorderStyle {
    left: SingleBorderStyle,
    top: SingleBorderStyle,
    right: SingleBorderStyle,
    bottom: SingleBorderStyle,
}

impl<R: Read + Seek> TryParse<R> for FullBorderStyle {
    type ParseError = FullBorderStyleParseError;

    fn try_parse(stream: &mut R) -> Result<FullBorderStyle, FullBorderStyleParseError> {
        let mut stream = stream.exclusive_blind_window()?;

        // Should be no flags set.
        FlagBlock::try_parse(&mut stream)?.ensure_flags_used()?;

        let border = {
            let mut read_one = || {
                SingleBorderStyle::try_parse(&mut stream)
                    .map_err(FullBorderStyleParseError::SingleBorderStyle)
            };

            FullBorderStyle {
                left: read_one()?,
                top: read_one()?,
                right: read_one()?,
                bottom: read_one()?,
            }
        };

        stream.ensure_eof()?;

        Ok(border)
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum CellParseError {
    Io(#[from] io::Error),
    FlagBlock(#[from] FlagBlockError),
    CellBorder(#[from] FullBorderStyleParseError),
    Content(#[from] TextParseError),

    #[error("cell content did not parse completely")]
    ContentUnfinished(#[source] UnfinishedParsingError),

    #[error("cell structure did not parse completely")]
    CellUnfinished(#[source] UnfinishedParsingError),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Cell {
    column_index: u32,
    row_span: u32,
    column_span: u32,
    background_colour: Option<u32>,
    rect: Rect,
    is_editable: bool,
    text: Text,
    border: Option<FullBorderStyle>,
}

impl<R: Read + Seek> TryParseWithContext<R, DocumentContext<'_, '_>> for Cell {
    type ParseError = CellParseError;

    fn try_parse_with_ctx(
        stream: &mut R,
        ctx: &DocumentContext<'_, '_>,
    ) -> Result<Cell, CellParseError> {
        let mut stream = stream.exclusive_blind_window()?;

        let mut flag_block = FlagBlock::try_parse(&mut stream)?;

        let column_index = stream.read_u32_le()?;
        let row_span = stream.read_u32_le()?;
        let column_span = stream.read_u32_le()?;
        let background_colour = stream.read_u32_le()?;
        let rect = Rect::try_parse_f64(&mut stream)?;
        let is_editable = stream.read_u8()? != 0;

        let text = {
            let mut stream = (&mut stream).exclusive_blind_window()?;

            let text = Text::try_parse_with_ctx(&mut stream, ctx)?;

            stream
                .ensure_eof()
                .map_err(CellParseError::ContentUnfinished)?;

            text
        };

        let property_flags = flag_block.property_flags_mut();

        unpack_bool_flags!(property_flags, {
            0 => is_background_colour_set;
        });

        let background_colour = is_background_colour_set.then_some(background_colour);

        let field_flags = flag_block.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => border: FullBorderStyle::try_parse(&mut stream)?;
        });

        flag_block.ensure_flags_used()?;

        stream
            .ensure_eof()
            .map_err(CellParseError::CellUnfinished)?;

        Ok(Cell {
            column_index,
            row_span,
            column_span,
            background_colour,
            rect,
            is_editable,
            text,
            border,
        })
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum RowParseError {
    Io(#[from] io::Error),
    Flags(#[from] FlagBlockError),
    Cell(#[from] CellParseError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("cell count {0} is too large")]
    TooManyCells(u32),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Row {
    cells: Vec<Cell>,
    index: u32,
    height: f32,
    max_height: Option<f32>,
    min_height: Option<f32>,
}

impl<R: Read + Seek> TryParseWithContext<R, DocumentContext<'_, '_>> for Row {
    type ParseError = RowParseError;

    fn try_parse_with_ctx(
        stream: &mut R,
        ctx: &DocumentContext<'_, '_>,
    ) -> Result<Row, RowParseError> {
        let mut stream = stream.exclusive_blind_window()?;

        let mut flag_block = FlagBlock::try_parse(&mut stream)?;

        let height = stream.read_f32_le()?;
        let index = stream.read_u32_le()?;

        let cells = read_u32_sized_vec!(
            stream,
            RowParseError::TooManyCells,
            Cell::try_parse_with_ctx(&mut stream, ctx)?,
        );

        let field_flags = flag_block.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            // If present, max height is written before min height, despite using a more
            // significant bit.
            9 => max_height: stream.read_f32_le()?;
            1 => min_height: stream.read_f32_le()?;
        });

        flag_block.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Row {
            cells,
            index,
            height,
            max_height,
            min_height,
        })
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum TableParseError {
    Io(#[from] io::Error),
    Shape(#[from] ShapeParseError),
    Header(#[from] ObjectHeaderError),
    FlagBlock(#[from] FlagBlockError),
    Row(#[from] RowParseError),
    BadTextAutoFitType(#[from] InvalidTextAutoFitTypeError),
    Border(#[from] FullBorderStyleParseError),
    Unfinished(#[from] UnfinishedParsingError),

    #[error("element count {0} is too large")]
    TooManyElements(u32),
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct Table {
    shape: Shape,
    is_heading_column_enabled: bool,
    is_heading_row_enabled: bool,
    is_max_height_enabled: bool,
    min_column_width: f32,
    min_row_height: f32,
    column_widths: Vec<f32>,
    rows: Vec<Row>,
    rect: Option<Rect>,
    border: Option<FullBorderStyle>,
    auto_fit_type: Option<TextAutoFitType>,
    column_min_widths: Vec<f32>,
    column_max_widths: Vec<f32>,
    max_height: Option<f32>,
    max_width: Option<f32>,
    default_cell_border: Option<FullBorderStyle>,
    heading_background_colour: Option<u32>,
    cell_background_colour: Option<u32>,
}

impl<R: Read + Seek> TryParseWithContext<R, DocumentContext<'_, '_>> for Table {
    type ParseError = TableParseError;

    fn try_parse_with_ctx(
        stream: &mut R,
        &doc_ctx: &DocumentContext<'_, '_>,
    ) -> Result<Table, TableParseError> {
        let shape = Shape::try_parse_with_ctx(
            stream,
            &ShapeParseContext {
                is_shape_only: false,
                doc_ctx,
            },
        )?;

        let (mut flag_block, mut stream) = try_parse_object_header(stream, 22)?;

        unpack_bool_flags!(flag_block.property_flags_mut(), {
            0 => is_heading_column_enabled;
            1 => is_heading_row_enabled;
            2 => is_max_height_enabled;
        });

        let field_flags = flag_block.init_flex(&mut stream)?;

        unpack_field_flags!(field_flags, {
            0 => min_column_width: stream.read_f32_le()?, else 10.0;
            1 => min_row_height: stream.read_f32_le()?, else 10.0;
            2 => column_widths: read_u32_sized_vec!(
                stream,
                TableParseError::TooManyElements,
                stream.read_f32_le()?,
            ), else Vec::new();
            3 => rows: read_u32_sized_vec!(
                stream,
                TableParseError::TooManyElements,
                Row::try_parse_with_ctx(&mut stream, &doc_ctx)?,
            ), else Vec::new();
            4 => rect: Rect::try_parse_f64(&mut stream)?;
            5 => border: FullBorderStyle::try_parse(&mut stream)?;
            6 => auto_fit_type: stream.read_u8()?.try_into()?;
            7 => column_min_widths: read_u32_sized_vec!(
                stream,
                TableParseError::TooManyElements,
                stream.read_f32_le()?,
            ), else Vec::new();
            8 => column_max_widths: read_u32_sized_vec!(
                stream,
                TableParseError::TooManyElements,
                stream.read_f32_le()?,
            ), else Vec::new();
            9 => max_height: stream.read_f32_le()?;
            10 => max_width: stream.read_f32_le()?;
            11 => default_cell_border: FullBorderStyle::try_parse(&mut stream)?;
            12 => heading_background_colour: stream.read_u32_le()?;
            13 => cell_background_colour: stream.read_u32_le()?;
        });

        flag_block.ensure_flags_used()?;
        stream.ensure_eof()?;

        Ok(Table {
            shape,
            is_heading_column_enabled,
            is_heading_row_enabled,
            is_max_height_enabled,
            min_column_width,
            min_row_height,
            column_widths,
            rows,
            rect,
            border,
            auto_fit_type,
            column_min_widths,
            column_max_widths,
            max_height,
            max_width,
            default_cell_border,
            heading_background_colour,
            cell_background_colour,
        })
    }
}

impl HasObjectBase for Table {
    fn object_base(&self) -> &ObjectBase {
        self.shape.object_base()
    }
}
