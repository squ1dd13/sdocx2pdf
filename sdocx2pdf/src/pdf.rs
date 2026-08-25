use krilla::geom::PathBuilder;
use sdocx::euclid;

pub enum PdfSpace {}
pub type Point2d = euclid::Point2D<f64, PdfSpace>;
pub type Vector2d = euclid::Vector2D<f64, PdfSpace>;
pub type Box2d = euclid::Box2D<f64, PdfSpace>;
pub type Size2d = euclid::Size2D<f64, PdfSpace>;
pub type Length = euclid::Length<f64, PdfSpace>;

#[derive(Debug, Clone, Copy)]
pub enum PolygonPoint {
    Normal(Point2d),
    Control(Point2d),
}

/// Appends the path specified by the given polygon points to `pb`.
pub fn specify_polygon(points: impl IntoIterator<Item = PolygonPoint>, pb: &mut PathBuilder) {
    let mut points = points.into_iter();

    let first = match points.next() {
        Some(first) => first,
        None => return,
    };

    let Point2d { x: fx, y: fy, .. } = match first {
        PolygonPoint::Normal(p) => p,
        PolygonPoint::Control(_) => panic!("first polygon point cannot be a control point"),
    };

    pb.move_to(fx as f32, fy as f32);

    loop {
        match points.next() {
            None => break,

            Some(PolygonPoint::Normal(p)) => {
                pb.line_to(p.x as f32, p.y as f32);
            }

            Some(PolygonPoint::Control(cp1)) => {
                let Some(PolygonPoint::Control(cp2)) = points.next() else {
                    panic!("missing second control point after first");
                };

                let Some(PolygonPoint::Normal(end)) = points.next() else {
                    panic!("missing end point for cubic Bézier");
                };

                pb.cubic_to(
                    cp1.x as f32,
                    cp1.y as f32,
                    cp2.x as f32,
                    cp2.y as f32,
                    end.x as f32,
                    end.y as f32,
                );
            }
        }
    }

    pb.close();
}
