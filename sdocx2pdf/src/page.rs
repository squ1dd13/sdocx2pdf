use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use krilla::{color::rgb, geom::Transform, num::NormalizedF32, paint::Fill, surface::Surface};
use log::{error, warn};
use thiserror::Error;

use crate::{
    shape::{NoStyleError, PathDrawingCtx, PathDrawingError},
    tool::{EventGroup, Tool},
};

#[derive(Debug, Error)]
pub enum DrawObjectError {
    #[error("failed to draw stroke object")]
    Stroke,

    #[error("shape is missing path")]
    ShapeMissingPath,

    #[error("failed to draw path for shape")]
    ShapePath(PathDrawingError),

    #[error("no style information for line")]
    NoLineStyle(NoStyleError),

    #[error("objects of type '{0}' are not yet supported")]
    Unsupported(&'static str),
}

impl DrawObjectError {
    pub fn log(&self) {
        match self {
            DrawObjectError::Stroke => error!("Failed to draw stroke object"),

            DrawObjectError::ShapeMissingPath => {
                error!("Shape object has no path")
            }

            DrawObjectError::ShapePath(err) => {
                error!("Failed to draw shape: {err}")
            }

            DrawObjectError::NoLineStyle(_) => {
                error!("Line has no style information")
            }

            DrawObjectError::Unsupported(obj_type_name) => {
                warn!("Ignoring '{obj_type_name}' object (not yet supported)")
            }
        }
    }
}

/// SDOCX page -> PDF page conversion context.
pub struct PageConversionCtx<'s> {
    /// Page size in document units: `(width, height)`.
    page_size: (f32, f32),

    /// PDF drawing surface.
    ///
    /// When the page conversion context is constructed, a transformation matrix is pushed to this
    /// drawing surface that scales the content such that drawing operations can be done in
    /// document space rather than PDF space.
    pub surface: Surface<'s>,
}

impl<'s> PageConversionCtx<'s> {
    pub fn new(
        page_size: (f32, f32),
        pt_per_unit: f32,
        mut surface: Surface<'s>,
    ) -> PageConversionCtx<'s> {
        // Apply a transformation here so that drawing operations to the surface can use document
        // units rather than locally scaling things whenever we need to draw them.
        surface.push_transform(&Transform::from_row(
            pt_per_unit,
            0.0,
            0.0,
            pt_per_unit,
            0.0,
            0.0,
        ));

        PageConversionCtx { page_size, surface }
    }

    pub fn fill_background(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.surface.set_stroke(None);
        self.surface.set_fill(Some(Fill {
            paint: rgb::Color::new(r, g, b).into(),
            opacity: NormalizedF32::new(a as f32 / 255.0).unwrap(),
            ..Fill::default()
        }));

        self.surface.draw_path(&self.boundary_path());
    }

    fn boundary_path(&self) -> krilla::geom::Path {
        let mut rect_pb = krilla::geom::PathBuilder::new();
        rect_pb.push_rect(
            krilla::geom::Rect::from_xywh(0.0, 0.0, self.page_size.0, self.page_size.1).unwrap(),
        );

        rect_pb.finish().unwrap()
    }

    fn draw_stroke_chunk_events<'e>(
        &mut self,
        stroke_events: impl IntoIterator<Item = EventGroup<'e>>,
        fill_boundary: &krilla::geom::Path,
        tool: Tool,
    ) -> Result<(), ()> {
        tool.draw_events(stroke_events, fill_boundary, &mut self.surface)
    }

    pub fn draw_single_object(
        &mut self,
        object: &sdocx::DocObject,
        pen_width_mul: f32,
        marker_width_mul: f32,
    ) -> Result<(), DrawObjectError> {
        match object {
            sdocx::DocObject::Stroke(stroke) => self
                .draw_stroke_chunk_events(
                    [EventGroup::from_stroke(stroke)],
                    &self.boundary_path(),
                    Tool::for_stroke(stroke).with_scaled_width(pen_width_mul, marker_width_mul),
                )
                .map_err(|()| DrawObjectError::Stroke),

            sdocx::DocObject::Line(line) => {
                if line.has_control_points() {
                    warn!("Ignoring line control points");
                }

                self.draw_line(line.start(), line.end(), line.colour_effect(), line.style())
                    .map_err(DrawObjectError::NoLineStyle)
            }

            sdocx::DocObject::Shape(shape) => {
                if let Some(path) = shape.path() {
                    self.draw_path_segments(
                        path.segments(),
                        shape.line_colour_effect(),
                        shape.line_style(),
                        shape.fill_effect(),
                    )
                    .map_err(DrawObjectError::ShapePath)
                } else {
                    Err(DrawObjectError::ShapeMissingPath)
                }
            }

            other => Err(DrawObjectError::Unsupported(<&str>::from(other))),
        }
    }

    pub fn draw_layer(
        &mut self,
        layer: &sdocx::page::Layer,
        pen_width_mul: f32,
        marker_width_mul: f32,
        multi_progress: &MultiProgress,
    ) {
        let objects = layer.objects();

        let objects_bar = multi_progress
            .add(ProgressBar::new(objects.len() as _))
            .with_style(
                ProgressStyle::with_template(
                    "Processing objects [{bar:40}] {percent}% [{pos}/{len}]",
                )
                .unwrap()
                .progress_chars("# "),
            );

        // Consecutive strokes very often use the same tool. To reduce the size of the output PDF,
        // we can process chains of such strokes in one go, loading the necessary graphics state
        // only once and then using it for all the strokes rather than loading the same graphics
        // state for every stroke.
        let chunked_objects = objects
            .iter()
            .inspect(|_| objects_bar.inc(1))
            .chunk_by(|&obj| match obj {
                sdocx::DocObject::Stroke(stroke) => Some(
                    Tool::for_stroke(stroke).with_scaled_width(pen_width_mul, marker_width_mul),
                ),
                _non_stroke => None,
            });

        // Compute the filling boundary once for this layer so we don't have to recompute it for
        // every chunk of events we draw.
        let fill_boundary = self.boundary_path();

        for (opt_stroke_tool, objects) in &chunked_objects {
            if let Some(tool) = opt_stroke_tool {
                // This is a chunk of strokes that all use `tool`.

                // Get the event slice for each stroke.
                let stroke_events = objects.map(|o| {
                    let sdocx::DocObject::Stroke(s) = o else {
                        unreachable!()
                    };
                    EventGroup::from_stroke(s)
                });

                if let Err(()) = self.draw_stroke_chunk_events(stroke_events, &fill_boundary, tool)
                {
                    DrawObjectError::Stroke.log();
                }

                continue;
            }

            // This is a chunk of non-strokes.
            for obj in objects {
                if let Err(err) = self.draw_single_object(obj, pen_width_mul, marker_width_mul) {
                    err.log();
                }
            }
        }
    }
}

impl Drop for PageConversionCtx<'_> {
    fn drop(&mut self) {
        // Pop the coordinate transformation.
        self.surface.pop();
    }
}
