use euclid::{Angle, Transform2D};
use krilla::{
    color::rgb,
    geom::PathBuilder,
    num::NormalizedF32,
    paint::{Fill, FillRule, LineCap, LineJoin, Stroke},
};
use sdocx::page::{
    Point as DocPoint,
    object::{
        ArrowShape, CapType, FillEffect, JoinType, LineColourEffect, LineStyleEffect, PathSegment,
    },
};
use thiserror::Error;

use crate::page::PageConversionCtx;
use crate::pdf;

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
    apex_at: pdf::Point,
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
        path_fn: impl FnOnce(&mut PathBuilder) -> Result<(), PathDrawingError>,
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

impl InternalPathDrawingCtx for PageConversionCtx<'_> {
    fn draw_path_with_stroke_fill_style(
        &mut self,
        stroke_style: Option<StrokeStyle>,
        fill_bgra: Option<[u8; 4]>,
        path_fn: impl FnOnce(&mut PathBuilder) -> Result<(), PathDrawingError>,
    ) -> Result<(), PathDrawingError> {
        let (filling, stroking) = (fill_bgra.is_some(), stroke_style.is_some());

        if !filling && !stroking {
            return Err(PathDrawingError::NoStyleError(NoStyleError));
        }

        if let Some([b, g, r, a]) = fill_bgra {
            self.surface.set_fill(Some(Fill {
                paint: rgb::Color::new(r, g, b).into(),
                opacity: NormalizedF32::new(a as f32 / 255.0).unwrap(),
                rule: FillRule::NonZero,
            }));
        } else {
            self.surface.set_fill(None);
        }

        if let Some(StrokeStyle {
            bgra: [b, g, r, a],
            width,
            join,
            cap,
        }) = stroke_style
        {
            self.surface.set_stroke(Some({
                let line_cap = match cap {
                    CapType::Butt => LineCap::Butt,
                    CapType::Round => LineCap::Round,
                    CapType::Square => LineCap::Square,
                };

                let line_join = match join {
                    JoinType::Miter => LineJoin::Miter,
                    JoinType::Round => LineJoin::Round,
                    JoinType::Bevel => LineJoin::Bevel,
                };

                Stroke {
                    paint: rgb::Color::new(r, g, b).into(),
                    width,
                    opacity: NormalizedF32::new(a as f32 / 255.0).unwrap(),
                    line_cap,
                    line_join,
                    ..Stroke::default()
                }
            }));
        } else {
            self.surface.set_stroke(None);
        }

        let mut pb = PathBuilder::new();
        path_fn(&mut pb)?;

        if let Some(path) = pb.finish() {
            self.surface.draw_path(&path);
        }

        Ok(())
    }
}

fn specify_path_by_segments(
    segments: &[PathSegment],
    pb: &mut PathBuilder,
) -> Result<(), PathDrawingError> {
    let mut last_point: Option<pdf::Point> = None;
    let mut found_close = false;

    for s in segments {
        if found_close {
            return Err(PathDrawingError::SegmentAfterClose);
        }

        match s {
            &PathSegment::MoveTo(p) => {
                let p = doc_point_to_pdf(p);
                last_point = Some(p);
                pb.move_to(p.x as f32, p.y as f32);
            }

            &PathSegment::LineTo(p) => {
                let p = doc_point_to_pdf(p);
                last_point = Some(p);
                pb.line_to(p.x as f32, p.y as f32);
            }

            &PathSegment::CubicTo { cp1, cp2, p3 } => {
                let cp1 = doc_point_to_pdf(cp1);
                let cp2 = doc_point_to_pdf(cp2);
                let p3 = doc_point_to_pdf(p3);
                last_point = Some(p3);
                pb.cubic_to(
                    cp1.x as f32,
                    cp1.y as f32,
                    cp2.x as f32,
                    cp2.y as f32,
                    p3.x as f32,
                    p3.y as f32,
                );
            }

            &PathSegment::QuadTo {
                cp1: quad_control,
                p2: end,
            } => {
                let Some(start) = last_point else {
                    return Err(PathDrawingError::BadQuad);
                };

                let quad_control = doc_point_to_pdf(quad_control);
                let end = doc_point_to_pdf(end);

                // Convert the quadratic Bézier to a cubic one.
                // https://fontforge.org/docs/techref/bezier.html#converting-truetype-to-postscript
                let cp1 = (((quad_control - start) * 2.0) / 3.0 + start.to_vector()).to_point();
                let cp2 = (((quad_control - end) * 2.0) / 3.0 + end.to_vector()).to_point();

                last_point = Some(end);

                pb.cubic_to(
                    cp1.x as f32,
                    cp1.y as f32,
                    cp2.x as f32,
                    cp2.y as f32,
                    end.x as f32,
                    end.y as f32,
                );
            }

            // todo: Implement these
            PathSegment::ArcTo { .. } => return Err(PathDrawingError::ArcUnsupported),
            PathSegment::AddOval(..) => return Err(PathDrawingError::OvalUnsupported),

            PathSegment::Close => {
                found_close = true;
                pb.close();
            }
        }
    }

    Ok(())
}

impl PathDrawingCtx for PageConversionCtx<'_> {
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

        // If there are arrowheads, we need to clip the line so that it does not poke out from
        // behind them. At an end where there is an arrowhead, the clipping path conforms to the
        // shape of the arrow. At an end where there is not, we need the line to fit inside the
        // clipping path. To do that, we assume the line has butt caps and think of it as a
        // rectangle. The corners of the clipping path at the non-arrow end are found by padding
        // this rectangle with some multiple of the stroke width such that regardless of the line
        // caps, the padded rectangle contains the line.
        let pushed_arrow_clip = match (start_arrow_points, end_arrow_points) {
            (None, None) => false,
            _ => {
                let width = ls.width as f64;
                let no_arrow_clip_pad = width * 3.0;

                let line_dir = line_vec.normalize();
                let up = pdf::Vector::new(line_dir.y, -line_dir.x) * width;
                let down = pdf::Vector::new(-line_dir.y, line_dir.x) * width;

                let mut clip = PathBuilder::new();

                match start_arrow_points {
                    // Start is rotated, so order is bottom, apex, top
                    Some([bottom, apex, top]) => {
                        clip.move_to(bottom.x as f32, bottom.y as f32);
                        clip.line_to(apex.x as f32, apex.y as f32);
                        clip.line_to(top.x as f32, top.y as f32);
                    }
                    None => {
                        let start_plus_pad = start - line_dir * no_arrow_clip_pad;

                        // As promised: No arrowhead at this end, so just add a butt cap with
                        // padding so that it won't clip the line itself.
                        let padded_down = start_plus_pad + down * no_arrow_clip_pad;
                        let padded_up = start_plus_pad + up * no_arrow_clip_pad;

                        clip.move_to(padded_down.x as f32, padded_down.y as f32);
                        clip.line_to(padded_up.x as f32, padded_up.y as f32);
                    }
                };

                match end_arrow_points {
                    Some([top, apex, bottom]) => {
                        clip.line_to(top.x as f32, top.y as f32);
                        clip.line_to(apex.x as f32, apex.y as f32);
                        clip.line_to(bottom.x as f32, bottom.y as f32);
                    }
                    None => {
                        let end_plus_pad = end + line_dir * no_arrow_clip_pad;

                        let padded_up = end_plus_pad + up * no_arrow_clip_pad;
                        let padded_down = end_plus_pad + down * no_arrow_clip_pad;

                        clip.line_to(padded_up.x as f32, padded_up.y as f32);
                        clip.line_to(padded_down.x as f32, padded_down.y as f32);
                    }
                };

                clip.close();
                let clip_path = clip.finish().unwrap();

                self.surface.push_clip_path(&clip_path, &FillRule::NonZero);

                // "Yes, we pushed a clip path"
                true
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
            |pb| {
                pb.move_to(start.x as f32, start.y as f32);
                pb.line_to(end.x as f32, end.y as f32);
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
                |pb| {
                    pb.move_to(arrow_points[0].x as f32, arrow_points[0].y as f32);
                    pb.line_to(arrow_points[1].x as f32, arrow_points[1].y as f32);
                    pb.line_to(arrow_points[2].x as f32, arrow_points[2].y as f32);
                    Ok(())
                },
            );

            // Similarly:
            arrow_res.expect("arrowhead path drawing should not fail");
        }

        if pushed_arrow_clip {
            self.surface.pop();
        }

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
            |pb| specify_path_by_segments(segments, pb),
        )
    }
}
