use euclid::{Point2D, Vector2D};
use itertools::Either;
use lopdf::{Object, content::Operation};

pub struct PdfSpace;
pub type PdfPoint = Point2D<f64, PdfSpace>;
pub type PdfVector = Vector2D<f64, PdfSpace>;

fn point_vec<const N: usize>(points: [PdfPoint; N]) -> Vec<Object> {
    let mut v = Vec::with_capacity(N * 2);

    for PdfPoint { x, y, .. } in points {
        v.push(x.into());
        v.push(y.into());
    }

    v
}

pub fn set_transformation_matrix(mx: [f32; 6]) -> Operation {
    Operation::new("cm", mx.map(Object::Real).to_vec())
}

pub fn save_graphics_state() -> Operation {
    Operation::new("q", vec![])
}

pub fn load_graphics_state(egs_name: &str) -> Operation {
    Operation::new("gs", vec![Object::Name(egs_name.into())])
}

pub fn set_fill_colour(r: u8, g: u8, b: u8) -> Operation {
    Operation::new(
        "rg",
        vec![
            Object::Real((r as f32) / 255.0),
            Object::Real((g as f32) / 255.0),
            Object::Real((b as f32) / 255.0),
        ],
    )
}

pub fn set_stroke_colour(r: u8, g: u8, b: u8) -> Operation {
    Operation::new(
        "RG",
        vec![
            Object::Real((r as f32) / 255.0),
            Object::Real((g as f32) / 255.0),
            Object::Real((b as f32) / 255.0),
        ],
    )
}

pub fn set_stroke_width(pt: f32) -> Operation {
    Operation::new("w", vec![Object::Real(pt)])
}

pub fn set_line_cap_round() -> Operation {
    Operation::new("J", vec![Object::Integer(1)])
}

pub fn set_line_cap_butt() -> Operation {
    Operation::new("J", vec![Object::Integer(0)])
}

pub fn draw_line(a: PdfPoint, b: PdfPoint) -> [Operation; 3] {
    [
        // Move to `a`
        Operation::new("m", point_vec([a])),
        // Line to `b`
        Operation::new("l", point_vec([b])),
        // Stroke
        Operation::new("S", vec![]),
    ]
}

#[derive(Debug, Clone, Copy)]
pub enum PolygonPoint {
    Normal(PdfPoint),
    Control(PdfPoint),
}

#[derive(Debug, Clone, Copy)]
pub enum WindingRule {
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy)]
pub enum PolygonDrawMode {
    Fill(WindingRule),
    Stroke,
    CloseFillAndStroke(WindingRule),
}

impl PolygonDrawMode {
    fn to_operation(self) -> Operation {
        Operation::new(
            match self {
                PolygonDrawMode::Fill(WindingRule::NonZero) => "f",
                PolygonDrawMode::Fill(WindingRule::EvenOdd) => "f*",
                PolygonDrawMode::Stroke => "S",
                PolygonDrawMode::CloseFillAndStroke(WindingRule::NonZero) => "b",
                PolygonDrawMode::CloseFillAndStroke(WindingRule::EvenOdd) => "b*",
            },
            vec![],
        )
    }
}

/// Constructs a closed path from `points` without closing the path object. This function may be
/// used to create a path consisting of several polygon subpaths.
pub fn specify_polygon<'p, P: IntoIterator<Item = PolygonPoint>>(
    points: P,
) -> impl Iterator<Item = Operation>
where
    P::IntoIter: 'p,
{
    let mut points = points.into_iter();

    let first_move = match points.next() {
        // Move to first point.
        Some(PolygonPoint::Normal(p)) => Operation::new("m", point_vec([p])),
        Some(PolygonPoint::Control(_)) => panic!("first polygon point cannot be a control point"),
        None => return Either::Left(std::iter::empty()),
    };

    Either::Right(
        std::iter::once(first_move)
            .chain(std::iter::from_fn(move || {
                Some(match points.next()? {
                    // Use "line to" for normal points.
                    PolygonPoint::Normal(p) => Operation::new("l", point_vec([p])),
                    // If this is a Bézier control point, we need another control point followed by
                    // a normal point to specify the entire curve.
                    PolygonPoint::Control(cp1) => {
                        let Some(PolygonPoint::Control(cp2)) = points.next() else {
                            panic!("missing second control point");
                        };

                        let Some(PolygonPoint::Normal(p3)) = points.next() else {
                            panic!("missing end point for cubic Bézier");
                        };

                        Operation::new("c", point_vec([cp1, cp2, p3]))
                    }
                })
            }))
            // Close the path.
            .chain(std::iter::once(Operation::new("h", vec![]))),
    )
}

pub fn draw_polygon<'p, P: IntoIterator<Item = PolygonPoint>>(
    points: P,
    mode: PolygonDrawMode,
) -> impl Iterator<Item = Operation>
where
    P::IntoIter: 'p,
{
    specify_polygon(points).chain(std::iter::once(mode.to_operation()))
}

pub fn clip(rule: WindingRule) -> [Operation; 2] {
    [
        // Clip.
        Operation::new(
            match rule {
                WindingRule::NonZero => "W",
                WindingRule::EvenOdd => "W*",
            },
            vec![],
        ),
        // End the path without drawing it.
        Operation::new("n", vec![]),
    ]
}

pub fn restore_graphics_state() -> Operation {
    Operation::new("Q", vec![])
}
