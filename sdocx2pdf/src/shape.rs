use euclid::{Angle, Point2D, Transform2D};
use lopdf::{content::Operation, dictionary};
use sdocx::page::{
    Point,
    object::{
        ArrowShape, CapType, FillEffect, JoinType, LineColourEffect, LineStyleEffect, PathSegment,
    },
};
use thiserror::Error;

use crate::op_gen::{self, PdfPoint, PdfSpace, PdfVector};

// --▶
// Vertices are named for an arrowhead pointing to the right.
const NORMAL_ARROW: [PdfPoint; 3] = [
    // Top
    PdfPoint::new(-1.0, -0.5),
    // Apex
    PdfPoint::new(0.0, 0.0),
    // Bottom
    PdfPoint::new(-1.0, 0.5),
];

/// Returns `[top, apex, bottom]` if the angle to the horizontal is zero, and the transformed
/// equivalents otherwise.
fn normal_arrow_vertices_ordered(
    apex_at: Point2D<f64, PdfSpace>,
    angle_to_horizontal: Angle<f64>,
    size_unit: f64,
) -> [PdfPoint; 3] {
    let tx = Transform2D::<f64, PdfSpace, PdfSpace>::scale(size_unit, size_unit)
        .then_rotate(angle_to_horizontal)
        .then_translate(apex_at.to_vector());

    NORMAL_ARROW.map(|p| tx.transform_point(p))
}

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

struct StrokeStyle {
    bgra: [u8; 4],
    width: f32,
    join: JoinType,
    cap: CapType,
}

fn draw_path_with_shape_style(
    stroke_style: Option<StrokeStyle>,
    fill_bgra: Option<[u8; 4]>,
    graphics_states: &mut lopdf::Dictionary,
    ops: &mut Vec<Operation>,
    path_fn: impl FnOnce(&mut Vec<Operation>) -> Result<(), PathDrawingError>,
) -> Result<(), PathDrawingError> {
    let (filling, stroking) = (fill_bgra.is_some(), stroke_style.is_some());

    if !filling && !stroking {
        return Err(PathDrawingError::NothingToDo);
    }

    let op_count_pre = ops.len();
    ops.push(op_gen::save_graphics_state());

    let mut graphics_state = None;

    if let Some(StrokeStyle {
        bgra: [b, g, r, a],
        width,
        join,
        cap,
    }) = stroke_style
    {
        ops.push(op_gen::set_stroke_colour(r, g, b));

        graphics_state = Some(lopdf::dictionary! {
            "Type" => "ExtGState",
            "LW" => width,
            "CA" => (a as f32) / 255.0,
            "LC" => match cap {
                CapType::Butt => 0,
                CapType::Round => 1,
                CapType::Square => 2,
            },
            "LJ" => match join {
                JoinType::Miter => 0,
                JoinType::Round => 1,
                JoinType::Bevel => 2,
            },
        });
    }

    if let Some([b, g, r, a]) = fill_bgra {
        ops.push(op_gen::set_fill_colour(r, g, b));

        // If the fill is not opaque, we need an extended graphics state for the alpha.
        if a < 255 {
            let ca = (a as f32) / 255.0;

            if let Some(graphics_state) = graphics_state.as_mut() {
                graphics_state.set("ca", ca);
            } else {
                graphics_state = Some(lopdf::dictionary! {
                    "Type" => "ExtGState",
                    "ca" => ca,
                });
            }
        }
    }

    if let Some(graphics_state) = graphics_state {
        let name = format!("egs{}", graphics_states.len());
        ops.push(op_gen::load_graphics_state(&name));
        graphics_states.set(name, graphics_state);
    }

    if let Err(err) = path_fn(ops) {
        // Remove the operations added.
        ops.truncate(op_count_pre);
        return Err(err);
    };

    match (filling, stroking) {
        (true, true) => ops.push(op_gen::fill_and_stroke()),
        (true, false) => ops.push(op_gen::fill()),
        (false, true) => ops.push(op_gen::stroke()),
        _ => (),
    }

    ops.push(op_gen::restore_graphics_state());

    Ok(())
}

fn doc_point_to_pdf(p: sdocx::page::Point) -> PdfPoint {
    <(f64, f64)>::from(p).into()
}

fn specify_path_by_segments(
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

                // todo: Implement these
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

fn check_line_style(ls: &LineStyleEffect) {
    match (ls.compound_type, ls.dash_type) {
        (sdocx::page::object::CompoundType::Simple, sdocx::page::object::DashType::Solid) => (),
        (_, _) => {
            eprintln!(
                "Warning: Alternative compound line types and dash types are not yet supported"
            );
        }
    }
}

pub fn draw_line(
    start: Point,
    end: Point,
    lc: Option<&LineColourEffect>,
    ls: Option<&LineStyleEffect>,
    graphics_states: &mut lopdf::Dictionary,
    ops: &mut Vec<Operation>,
) -> Result<(), PathDrawingError> {
    let start: PdfPoint = (start.x, start.y).into();
    let end: PdfPoint = (end.x, end.y).into();

    let op_count_pre = ops.len();
    ops.push(op_gen::save_graphics_state());

    let lc = match lc {
        Some(lc) => lc,
        None => &LineColourEffect::default(),
    };

    let ls = match ls {
        Some(ls) => {
            check_line_style(ls);
            ls
        }
        None => &LineStyleEffect::default(),
    };

    let line_vec = end - start;
    let line_angle = line_vec.angle_from_x_axis();

    let start_arrow_points = match ls.begin_arrow_shape {
        ArrowShape::None => None,
        arrow_shape => {
            if !matches!(arrow_shape, ArrowShape::Arrow) {
                // todo: Implement other arrowheads
                eprintln!("Warning: Only the basic arrowhead is implemented");
            }

            Some(normal_arrow_vertices_ordered(
                start,
                // Arrow at the start points away from the end, so rotate 180 degrees.
                line_angle + Angle::pi(),
                ls.begin_arrow_size.unit(ls.width as _),
            ))
        }
    };

    let end_arrow_points = match ls.end_arrow_shape {
        ArrowShape::None => None,
        arrow_shape => {
            if !matches!(arrow_shape, ArrowShape::Arrow) {
                eprintln!("Warning: Only the basic arrowhead is implemented");
            }

            Some(normal_arrow_vertices_ordered(
                end,
                line_angle,
                ls.end_arrow_size.unit(ls.width as _),
            ))
        }
    };

    match (start_arrow_points, end_arrow_points) {
        (None, None) => (),

        // If there are arrowheads, we need to clip the line so that it does not poke out from
        // behind them. At an end where there is an arrowhead, the clipping path conforms to the
        // shape of the arrow. At an end where there is not, we need the line to fit inside the
        // clipping path. To do that, we assume the line has butt caps and think of it as a
        // rectangle. The corners of the clipping path at the non-arrow end are found by padding
        // this rectangle with some multiple of the stroke width such that regardless of the line
        // caps, the padded rectangle contains the line.
        (sap, eap) => {
            let width = ls.width as f64;
            let no_arrow_clip_pad = width * 3.0;

            let line_dir = line_vec.normalize();

            let up = PdfVector::new(line_dir.y, -line_dir.x) * width;
            let down = PdfVector::new(-line_dir.y, line_dir.x) * width;

            match sap {
                // Start is rotated, so order is bottom, apex, top
                Some([bottom, apex, top]) => {
                    ops.extend([
                        op_gen::move_to(bottom),
                        op_gen::line_to(apex),
                        op_gen::line_to(top),
                    ]);
                }
                None => {
                    let start_plus_pad = start - line_dir * no_arrow_clip_pad;

                    ops.extend([
                        op_gen::move_to(start_plus_pad + down * no_arrow_clip_pad),
                        op_gen::line_to(start_plus_pad + up * no_arrow_clip_pad),
                    ]);
                }
            };

            match eap {
                Some([top, apex, bottom]) => {
                    ops.extend([
                        op_gen::line_to(top),
                        op_gen::line_to(apex),
                        op_gen::line_to(bottom),
                    ]);
                }
                None => {
                    let end_plus_pad = end + line_dir * no_arrow_clip_pad;

                    ops.extend([
                        op_gen::line_to(end_plus_pad + up * no_arrow_clip_pad),
                        op_gen::line_to(end_plus_pad + down * no_arrow_clip_pad),
                    ]);
                }
            };

            // Clip the current graphics state with the path just specified.
            ops.extend(op_gen::clip(op_gen::WindingRule::NonZero));
        }
    };

    let draw_res = 'draw_block: {
        // Draw the line itself.
        match draw_path_with_shape_style(
            Some(StrokeStyle {
                bgra: lc.solid_colour_bgra(),
                width: ls.width,
                join: ls.join_type,
                cap: ls.cap_type,
            }),
            // No fill
            None,
            graphics_states,
            ops,
            |ops| {
                ops.extend([op_gen::move_to(start), op_gen::line_to(end)]);
                Ok(())
            },
        ) {
            Ok(it) => it,
            Err(err) => break 'draw_block Err(err),
        };

        for arrow_points in [start_arrow_points, end_arrow_points].into_iter().flatten() {
            match draw_path_with_shape_style(
                None,
                // The arrowheads are part of the line, but they are filled, so the line stroke
                // becomes the arrowhead fill.
                Some(lc.solid_colour_bgra()),
                graphics_states,
                ops,
                |ops| {
                    ops.extend([
                        op_gen::move_to(arrow_points[0]),
                        op_gen::line_to(arrow_points[1]),
                        op_gen::line_to(arrow_points[2]),
                    ]);

                    Ok(())
                },
            ) {
                Ok(it) => it,
                Err(err) => break 'draw_block Err(err),
            };
        }

        Ok(())
    };

    if let Err(err) = draw_res {
        // Remove any operations added.
        ops.truncate(op_count_pre);
        return Err(err);
    }

    ops.push(op_gen::restore_graphics_state());

    Ok(())
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

pub fn draw_path_segments(
    segments: &[PathSegment],
    lc: Option<&LineColourEffect>,
    ls: Option<&LineStyleEffect>,
    fill_effect: Option<&FillEffect>,
    graphics_states: &mut lopdf::Dictionary,
    ops: &mut Vec<Operation>,
) -> Result<(), PathDrawingError> {
    let stroke_style = match (lc, ls) {
        (None, None) => None,
        (lc, ls) => {
            let lc = match lc {
                Some(lc) => lc,
                None => &LineColourEffect::default(),
            };

            let ls = match ls {
                Some(ls) => {
                    check_line_style(ls);
                    ls
                }
                None => &LineStyleEffect::default(),
            };

            Some(StrokeStyle {
                bgra: lc.solid_colour_bgra(),
                width: ls.width,
                join: ls.join_type,
                cap: ls.cap_type,
            })
        }
    };

    draw_path_with_shape_style(
        stroke_style,
        fill_effect.and_then(fe_to_solid_bgra),
        graphics_states,
        ops,
        |ops| specify_path_by_segments(segments, ops),
    )
}
