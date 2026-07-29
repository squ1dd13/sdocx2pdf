use euclid::{Angle, Point2D, Transform2D};
use sdocx::page::{
    Point as DocPoint,
    object::{
        ArrowShape, CapType, FillEffect, JoinType, LineColourEffect, LineStyleEffect, PathSegment,
    },
};
use thiserror::Error;

use crate::page::PageConversionCtx;
use crate::pdf;
use crate::pdf::dictionary;

// --▶
// Vertices are named for an arrowhead pointing to the right.
const NORMAL_ARROW: [pdf::Point; 3] = [
    // Top
    pdf::Point::new(-1.0, -0.5),
    // Apex
    pdf::Point::new(0.0, 0.0),
    // Bottom
    pdf::Point::new(-1.0, 0.5),
];

/// Returns `[top, apex, bottom]` if the angle to the horizontal is zero, and the transformed
/// equivalents otherwise.
fn normal_arrow_vertices_ordered(
    apex_at: Point2D<f64, pdf::Space>,
    angle_to_horizontal: Angle<f64>,
    size_unit: f64,
) -> [pdf::Point; 3] {
    let tx = Transform2D::<f64, pdf::Space, pdf::Space>::scale(size_unit, size_unit)
        .then_rotate(angle_to_horizontal)
        .then_translate(apex_at.to_vector());

    NORMAL_ARROW.map(|p| tx.transform_point(p))
}

fn doc_point_to_pdf(p: DocPoint) -> pdf::Point {
    <(f64, f64)>::from(p).into()
}

fn specify_path_by_segments(
    segments: &[PathSegment],
    ops: &mut Vec<pdf::Operation>,
) -> Result<(), PathDrawingError> {
    let mut last_point: Option<pdf::Point> = None;

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
                    pdf::move_to(p)
                }

                &PathSegment::LineTo(p) => {
                    let p = doc_point_to_pdf(p);
                    last_point = Some(p);
                    pdf::line_to(p)
                }

                &PathSegment::CubicTo { cp1, cp2, p3 } => {
                    let p3 = doc_point_to_pdf(p3);
                    last_point = Some(p3);
                    pdf::cubic_to(doc_point_to_pdf(cp1), doc_point_to_pdf(cp2), p3)
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

                    pdf::cubic_to(cp1, cp2, end)
                }

                // todo: Implement these
                PathSegment::ArcTo { .. } => break 'op_block Err(PathDrawingError::ArcUnsupported),
                PathSegment::AddOval(..) => break 'op_block Err(PathDrawingError::OvalUnsupported),

                PathSegment::Close => {
                    found_close = true;
                    pdf::close_subpath()
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

#[derive(Debug, Error)]
#[error("there is no style information with which to draw the path")]
pub struct NoStyleError;

#[derive(Debug, Error)]
pub enum PathDrawingError {
    #[error(transparent)]
    NoStyleError(NoStyleError),

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

trait InternalPathDrawingCtx {
    fn draw_path_with_stroke_fill_style(
        &mut self,
        stroke_style: Option<StrokeStyle>,
        fill_bgra: Option<[u8; 4]>,
        path_fn: impl FnOnce(&mut Vec<pdf::Operation>) -> Result<(), PathDrawingError>,
    ) -> Result<(), PathDrawingError>;
}

pub trait PathDrawingCtx {
    fn draw_line(
        &mut self,
        start: DocPoint,
        end: DocPoint,
        lc: Option<&LineColourEffect>,
        ls: Option<&LineStyleEffect>,
    ) -> Result<(), NoStyleError>;

    fn draw_path_segments(
        &mut self,
        segments: &[PathSegment],
        lc: Option<&LineColourEffect>,
        ls: Option<&LineStyleEffect>,
        fill_effect: Option<&FillEffect>,
    ) -> Result<(), PathDrawingError>;
}

impl InternalPathDrawingCtx for PageConversionCtx {
    fn draw_path_with_stroke_fill_style(
        &mut self,
        stroke_style: Option<StrokeStyle>,
        fill_bgra: Option<[u8; 4]>,
        path_fn: impl FnOnce(&mut Vec<pdf::Operation>) -> Result<(), PathDrawingError>,
    ) -> Result<(), PathDrawingError> {
        let (filling, stroking) = (fill_bgra.is_some(), stroke_style.is_some());

        if !filling && !stroking {
            return Err(PathDrawingError::NoStyleError(NoStyleError));
        }

        let op_count_pre = self.ops.len();
        self.ops.push(pdf::save_graphics_state());

        let mut graphics_state = None;

        if let Some(StrokeStyle {
            bgra: [b, g, r, a],
            width,
            join,
            cap,
        }) = stroke_style
        {
            self.ops.push(pdf::set_stroke_colour(r, g, b));

            graphics_state = Some(pdf::dictionary! {
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
            self.ops.push(pdf::set_fill_colour(r, g, b));

            // If the fill is not opaque, we need an extended graphics state for the alpha.
            if a < 255 {
                let ca = (a as f32) / 255.0;

                if let Some(graphics_state) = graphics_state.as_mut() {
                    graphics_state.set("ca", ca);
                } else {
                    graphics_state = Some(pdf::dictionary! {
                        "Type" => "ExtGState",
                        "ca" => ca,
                    });
                }
            }
        }

        if let Some(graphics_dict) = graphics_state {
            let name = self.add_graphics_dict(graphics_dict);
            self.ops.push(pdf::load_graphics_dict(name));
        }

        if let Err(err) = path_fn(&mut self.ops) {
            // Remove the operations added.
            self.ops.truncate(op_count_pre);
            return Err(err);
        };

        match (filling, stroking) {
            (true, true) => self.ops.push(pdf::fill_and_stroke()),
            (true, false) => self.ops.push(pdf::fill()),
            (false, true) => self.ops.push(pdf::stroke()),
            _ => (),
        }

        self.ops.push(pdf::restore_graphics_state());

        Ok(())
    }
}

impl PathDrawingCtx for PageConversionCtx {
    fn draw_line(
        &mut self,
        start: DocPoint,
        end: DocPoint,
        lc: Option<&LineColourEffect>,
        ls: Option<&LineStyleEffect>,
    ) -> Result<(), NoStyleError> {
        if lc.is_none() && ls.is_none() {
            return Err(NoStyleError);
        }

        let start: pdf::Point = (start.x, start.y).into();
        let end: pdf::Point = (end.x, end.y).into();

        self.ops.push(pdf::save_graphics_state());

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

                let up = pdf::Vector::new(line_dir.y, -line_dir.x) * width;
                let down = pdf::Vector::new(-line_dir.y, line_dir.x) * width;

                match sap {
                    // Start is rotated, so order is bottom, apex, top
                    Some([bottom, apex, top]) => {
                        self.ops.extend([
                            pdf::move_to(bottom),
                            pdf::line_to(apex),
                            pdf::line_to(top),
                        ]);
                    }
                    None => {
                        let start_plus_pad = start - line_dir * no_arrow_clip_pad;

                        self.ops.extend([
                            pdf::move_to(start_plus_pad + down * no_arrow_clip_pad),
                            pdf::line_to(start_plus_pad + up * no_arrow_clip_pad),
                        ]);
                    }
                };

                match eap {
                    Some([top, apex, bottom]) => {
                        self.ops.extend([
                            pdf::line_to(top),
                            pdf::line_to(apex),
                            pdf::line_to(bottom),
                        ]);
                    }
                    None => {
                        let end_plus_pad = end + line_dir * no_arrow_clip_pad;

                        self.ops.extend([
                            pdf::line_to(end_plus_pad + up * no_arrow_clip_pad),
                            pdf::line_to(end_plus_pad + down * no_arrow_clip_pad),
                        ]);
                    }
                };

                // Clip the current graphics state with the path just specified.
                self.ops.extend(pdf::clip(pdf::WindingRule::NonZero));
            }
        };

        // Draw the line itself.
        let line_res = self.draw_path_with_stroke_fill_style(
            Some(StrokeStyle {
                bgra: lc.solid_colour_bgra(),
                width: ls.width,
                join: ls.join_type,
                cap: ls.cap_type,
            }),
            // No fill
            None,
            |ops| {
                ops.extend([pdf::move_to(start), pdf::line_to(end)]);
                Ok(())
            },
        );

        // There's style information and a valid path, so...
        line_res.expect("straight line path drawing should not fail");

        for arrow_points in [start_arrow_points, end_arrow_points].into_iter().flatten() {
            let arrow_res = self.draw_path_with_stroke_fill_style(
                None,
                // The arrowheads are part of the line, but they are filled, so the line stroke
                // becomes the arrowhead fill.
                Some(lc.solid_colour_bgra()),
                |ops| {
                    ops.extend([
                        pdf::move_to(arrow_points[0]),
                        pdf::line_to(arrow_points[1]),
                        pdf::line_to(arrow_points[2]),
                    ]);

                    Ok(())
                },
            );

            // Similarly:
            arrow_res.expect("arrowhead path drawing should not fail");
        }

        self.ops.push(pdf::restore_graphics_state());

        Ok(())
    }

    fn draw_path_segments(
        &mut self,
        segments: &[PathSegment],
        lc: Option<&LineColourEffect>,
        ls: Option<&LineStyleEffect>,
        fill_effect: Option<&FillEffect>,
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

        self.draw_path_with_stroke_fill_style(
            stroke_style,
            fill_effect.and_then(fe_to_solid_bgra),
            |ops| specify_path_by_segments(segments, ops),
        )
    }
}
