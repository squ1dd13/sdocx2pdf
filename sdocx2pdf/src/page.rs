use std::collections::HashMap;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use log::{error, warn};
use thiserror::Error;

use crate::{
    pdf,
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
pub struct PageConversionCtx {
    /// The size of the page (in document units).
    page_size: (f32, f32),

    /// The PDF operations for the page.
    pub ops: Vec<pdf::Operation>,

    /// Maps names to the graphics state parameter dictionaries.
    graphics_dicts: pdf::Dictionary,

    /// Maps `t -> n` where `t` is a tool and `n` is the name of an existing graphics state
    /// parameter dictionary that can be used to draw strokes for `t`.
    ///
    /// Helps reduce the size of the PDF produced by allowing graphics state dictionaries to be
    /// reused.
    tool_graphics_dict_names: HashMap<Tool, pdf::GraphicsDictName>,

    /// Maps names to XObjects.
    xobjects: pdf::Dictionary,
}

impl PageConversionCtx {
    pub fn new(page_size: (f32, f32)) -> PageConversionCtx {
        PageConversionCtx {
            page_size,
            ops: Vec::new(),
            graphics_dicts: pdf::Dictionary::new(),
            tool_graphics_dict_names: HashMap::new(),
            xobjects: pdf::Dictionary::new(),
        }
    }

    pub fn add_xobject(&mut self, xobj: impl Into<pdf::Object>) -> pdf::XObjectName {
        let name = format!("s2p-xobj-{}", self.xobjects.len());
        self.xobjects.set(name.clone(), xobj);
        name.into()
    }

    pub fn add_graphics_dict(&mut self, gd: impl Into<pdf::Object>) -> pdf::GraphicsDictName {
        let name = format!("s2p-gd-{}", self.graphics_dicts.len());
        self.graphics_dicts.set(name.clone(), gd);
        name.into()
    }

    pub fn tool_graphics_dict_name(&mut self, tool: &Tool) -> pdf::GraphicsDictName {
        if let Some(name) = self.tool_graphics_dict_names.get(tool) {
            return name.clone();
        }

        let name = self.add_graphics_dict(tool.create_egs());

        self.tool_graphics_dict_names
            .insert(tool.clone(), name.clone());

        name
    }

    fn draw_stroke_chunk_events<'e>(
        &mut self,
        stroke_events: impl IntoIterator<Item = EventGroup<'e>>,
        tool: Tool,
    ) -> Result<(), ()> {
        tool.draw_events(
            self.tool_graphics_dict_name(&tool),
            self.page_size,
            stroke_events,
            &mut self.ops,
        )
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

        // Consecutive stokes very often use the same tool. To reduce the size of the output PDF, we
        // can process chains of such strokes in one go, loading the necessary graphics state only
        // once and then using it for all the strokes rather than loading the same graphics state for
        // every stroke.
        let chunked_objects = objects
            .iter()
            .inspect(|_| objects_bar.inc(1))
            .chunk_by(|&obj| match obj {
                sdocx::DocObject::Stroke(stroke) => Some(
                    Tool::for_stroke(stroke).with_scaled_width(pen_width_mul, marker_width_mul),
                ),
                _non_stroke => None,
            });

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

                if let Err(()) = self.draw_stroke_chunk_events(stroke_events, tool) {
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

    /// Returns (operations, name -> graphics dict map, name -> XObject ID map)
    pub fn into_parts(self) -> (Vec<pdf::Operation>, pdf::Dictionary, pdf::Dictionary) {
        let PageConversionCtx {
            page_size: _,
            ops,
            graphics_dicts,
            tool_graphics_dict_names: _,
            xobjects: xobject_ids,
        } = self;

        (ops, graphics_dicts, xobject_ids)
    }
}
