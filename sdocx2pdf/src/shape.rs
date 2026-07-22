use lopdf::{content::Operation, dictionary};
use sdocx::page::{
    Point,
    object::{
        ArrowShape, CapType, CompoundType, FillEffect, JoinType, LineColourEffect, LineStyleEffect,
        PathSegment,
    },
};
use thiserror::Error;

use crate::op_gen::{self, PdfPoint};

fn lce_to_solid_bgra(lce: &LineColourEffect) -> [u8; 4] {
    if !lce.colour_type.is_solid() {
        eprintln!(
            "Warning: Only solid line colours are supported; found '{}'",
            lce.colour_type
        );
    }

    lce.solid_colour_bgra()
}

fn fe_to_solid_bgra(fe: &FillEffect) -> Option<[u8; 4]> {
    match fe {
        FillEffect::Colour(fce) => {
            if !fce.colour_type.is_solid() {
                eprintln!(
                    "Warning: Only solid fill colours are supported; found colour type '{}'",
                    fce.colour_type
                );
            }

            Some(fce.solid_colour_bgra())
        }

        other => {
            eprintln!(
                "Warning: Only solid fill colours are supported; effect '{}' will be ignored",
                other
            );

            None
        }
    }
}

fn doc_point_to_pdf(p: sdocx::page::Point) -> PdfPoint {
    <(f64, f64)>::from(p).into()
}

fn create_fill_alpha_graphics_state(fill_alpha: u8) -> lopdf::Dictionary {
    lopdf::dictionary! {
        "Type" => "ExtGState",
        "ca" => (fill_alpha as f32) / 255.0,
    }
}

fn create_full_graphics_state(
    stroke_alpha: u8,
    fill_alpha: Option<u8>,
    line_style: &LineStyleEffect,
) -> lopdf::Dictionary {
    let mut dict = lopdf::dictionary! {
        "Type" => "ExtGState",
        "LW" => line_style.width,
        "LC" => match line_style.cap_type {
            CapType::Butt => 0,
            CapType::Round => 1,
            CapType::Square => 2,
        },
        "LJ" => match line_style.join_type {
            JoinType::Miter => 0,
            JoinType::Round => 1,
            JoinType::Bevel => 2,
        },
        "CA" => (stroke_alpha as f32) / 255.0,
    };

    if let Some(fill_alpha) = fill_alpha {
        dict.set("ca", (fill_alpha as f32) / 255.0);
    }

    if !matches!(
        (&line_style.begin_arrow_shape, &line_style.end_arrow_shape),
        (&ArrowShape::None, &ArrowShape::None)
    ) {
        // eprintln!(
        //     "Warning: Arrow shapes are not yet supported; found {:?} and {:?}",
        //     line_style.begin_arrow_shape, line_style.end_arrow_shape
        // );
    }

    if !matches!(line_style.compound_type, CompoundType::Simple) {
        // eprintln!(
        //     "Warning: Compound types are not yet supported; found {:?}",
        //     line_style.compound_type
        // );
    }

    dict
}

pub fn straight_line_segments(a: Point, b: Point) -> [PathSegment; 2] {
    [PathSegment::MoveTo(a), PathSegment::LineTo(b)]
}

// // ShapeDrawingLineEffect::setArrowSize
// fn arrow_size_unit(stroke_width: f32, size: ArrowSize) -> f32 {
//     match size {
//         ArrowSize::Normal => (stroke_width * 3.0) + 10.0,
//         ArrowSize::Small => (stroke_width * 2.5) + 5.0,
//         ArrowSize::Big => (stroke_width * 4.0) + 15.0,
//     }
// }

// // --▶
// // These Unicode pictures show the heads pointing to the right, but the points are for a head
// // pointing upwards.
// const NORMAL_ARROW: [Point2D<f32, ()>; 3] = [
//     // Bottom left
//     Point2D::new(-0.5, 1.0),
//     // Apex
//     Point2D::new(0.0, 0.0),
//     // Bottom right
//     Point2D::new(0.5, 1.0),
// ];

// // --➤
// const STEALTH_ARROW: [Point2D<f32, ()>; 4] = [
//     // Indented point in the middle
//     Point2D::new(0.0, 0.5),
//     // Bottom left
//     Point2D::new(-0.5, 1.0),
//     // Apex
//     Point2D::new(0.0, 0.0),
//     // Bottom right
//     Point2D::new(0.5, 1.0),
// ];

// // --◆
// const DIAMOND_ARROW: [Point2D<f32, ()>; 4] = [
//     // Top vertex
//     Point2D::new(0.0, -std::f32::consts::FRAC_1_SQRT_2),
//     // Right vertex
//     Point2D::new(std::f32::consts::FRAC_1_SQRT_2, 0.0),
//     // Bottom vertex
//     Point2D::new(0.0, std::f32::consts::FRAC_1_SQRT_2),
//     // Left vertex
//     Point2D::new(-std::f32::consts::FRAC_1_SQRT_2, 0.0),
// ];

// There's also the open arrow, but it also uses the stroke width, not just the unit size.

#[derive(Debug, Error)]
pub enum PathDrawingError {
    #[error("no stroke or fill style provided, so nothing to draw")]
    NothingToDo,

    #[error("found extra segment after closure")]
    SegmentAfterClose,

    #[error("arc path segments are currently unsupported")]
    ArcUnsupported,

    #[error("oval path segments are currently unsupported")]
    OvalUnsupported,

    #[error("invalid quadratic Bézier segment")]
    BadQuad,
}

pub fn draw_path_segments(
    segments: &[PathSegment],
    line_colour: Option<&LineColourEffect>,
    line_style: Option<&LineStyleEffect>,
    fill_effect: Option<&FillEffect>,
    graphics_states: &mut lopdf::Dictionary,
    ops: &mut Vec<Operation>,
) -> Result<(), PathDrawingError> {
    if line_colour.is_none() && line_style.is_none() && fill_effect.is_none() {
        return Err(PathDrawingError::NothingToDo);
    }

    segments_to_ops(segments, ops)?;

    ops.push(op_gen::save_graphics_state());

    // todo: Everything here would be simpler with a single type for LC/LS/FC which handles
    // defaults and the creation of graphics states - in other words, a "shape tool" type. We could
    // then reuse tools and graphics states like we do for strokes.
    let fill_colour_bgra = fill_effect.and_then(fe_to_solid_bgra);

    if line_colour.is_some() || line_style.is_some() {
        // We stroke if we have either a colour or style, but we might not have been given both.
        let [line_b, line_g, line_r, line_a] = line_colour
            .map(lce_to_solid_bgra)
            .unwrap_or_else(|| lce_to_solid_bgra(&LineColourEffect::default()));

        let line_style = match line_style {
            Some(ls) => ls,
            None => &LineStyleEffect::default(),
        };

        let graphics_state =
            create_full_graphics_state(line_a, fill_colour_bgra.map(|[.., a]| a), line_style);

        let gs_name = format!("egs{}", graphics_states.len());
        graphics_states.set(gs_name.clone(), graphics_state);

        ops.extend([
            op_gen::load_graphics_state(&gs_name),
            op_gen::set_stroke_colour(line_r, line_g, line_b),
        ]);

        if let Some([fb, fg, fr, _]) = fill_colour_bgra {
            ops.extend([
                op_gen::set_fill_colour(fr, fg, fb),
                op_gen::fill_and_stroke(),
            ]);
        } else {
            ops.push(op_gen::stroke());
        }
    } else {
        match fill_colour_bgra {
            // If we are not stroking, we only need an extended graphics state if the fill colour
            // is not opaque.
            #![allow(non_contiguous_range_endpoints)]
            Some([fb, fg, fr, fill_alpha @ ..255]) => {
                let gs_name = format!("egs{}", graphics_states.len());

                graphics_states.set(
                    gs_name.clone(),
                    create_fill_alpha_graphics_state(fill_alpha),
                );

                ops.extend([
                    op_gen::load_graphics_state(&gs_name),
                    op_gen::set_fill_colour(fr, fg, fb),
                    op_gen::fill(),
                ]);
            }

            Some([fb, fg, fr, _]) => {
                ops.extend([op_gen::set_fill_colour(fr, fg, fb), op_gen::fill()])
            }

            // No fill colour. Should be unreachable as we would only get here if neither stroking
            // nor filling.
            None => (),
        }
    }

    ops.push(op_gen::restore_graphics_state());

    Ok(())
}

fn segments_to_ops(
    segments: &[PathSegment],
    ops: &mut Vec<Operation>,
) -> Result<(), PathDrawingError> {
    let mut last_point: Option<PdfPoint> = None;

    // If we encounter an error, we need to know how many ops there were before so we can remove
    // all the ones we added.
    let op_count_pre = ops.len();

    let mut found_close = false;

    for s in segments {
        let op_res: Result<_, PathDrawingError> = 'op_block: {
            if found_close {
                break 'op_block Err(PathDrawingError::SegmentAfterClose);
            }

            Ok(match s {
                &PathSegment::MoveTo(p) => {
                    let p = doc_point_to_pdf(p);
                    last_point = Some(p);
                    op_gen::move_to(p)
                }

                &PathSegment::LineTo(p) => {
                    let p = doc_point_to_pdf(p);
                    last_point = Some(p);
                    op_gen::line_to(p)
                }

                &PathSegment::CubicTo { cp1, cp2, p3 } => {
                    let p3 = doc_point_to_pdf(p3);
                    last_point = Some(p3);
                    op_gen::cubic_to(doc_point_to_pdf(cp1), doc_point_to_pdf(cp2), p3)
                }

                &PathSegment::QuadTo {
                    cp1: quad_control,
                    p2: end,
                } => {
                    let Some(start) = last_point else {
                        break 'op_block Err(PathDrawingError::BadQuad);
                    };

                    let quad_control = doc_point_to_pdf(quad_control);
                    let end = doc_point_to_pdf(end);

                    // Convert the quadratic Bézier to a cubic one.
                    // https://fontforge.org/docs/techref/bezier.html#converting-truetype-to-postscript
                    let cp1 = (((quad_control - start) * 2.0) / 3.0 + start.to_vector()).to_point();
                    let cp2 = (((quad_control - end) * 2.0) / 3.0 + end.to_vector()).to_point();

                    last_point = Some(end);

                    op_gen::cubic_to(cp1, cp2, end)
                }

                PathSegment::ArcTo { .. } => break 'op_block Err(PathDrawingError::ArcUnsupported),
                PathSegment::AddOval(..) => break 'op_block Err(PathDrawingError::OvalUnsupported),

                PathSegment::Close => {
                    found_close = true;
                    op_gen::close_subpath()
                }
            })
        };

        match op_res {
            Ok(op) => ops.push(op),
            Err(err) => {
                // Remove any operations added.
                ops.truncate(op_count_pre);
                return Err(err);
            }
        }
    }

    Ok(())
}
