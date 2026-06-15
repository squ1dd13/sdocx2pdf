use std::ops::Add;

use itertools::Itertools;
use ndarray::{Array1, ArrayView1, Axis};
use ndarray_ndimage::BorderMode;
use scirs2_interpolate::{InterpolateResult, PchipInterpolator};

/// Interpolates between samples of stroke data to create a continuous mapping from time to
/// position and pressure.
pub struct InterpolatedStroke {
    x: PchipInterpolator<f64>,
    y: PchipInterpolator<f64>,
    pressure: PchipInterpolator<f64>,
}

impl InterpolatedStroke {
    /// Returns an interpolated stroke created from the given `x`, `y` and `p`ressure data sampled
    /// at the times `t`.
    ///
    /// Returns an error if the given arrays are not all of the same length; if there are
    /// fewer than two samples; or if `t` is not in order.
    fn new(
        t: &ArrayView1<f64>,
        x: &ArrayView1<f64>,
        y: &ArrayView1<f64>,
        p: &ArrayView1<f64>,
    ) -> InterpolateResult<InterpolatedStroke> {
        Ok(InterpolatedStroke {
            x: PchipInterpolator::new(t, x, false)?,
            y: PchipInterpolator::new(t, y, false)?,
            pressure: PchipInterpolator::new(t, p, false)?,
        })
    }

    /// Constructs an interpolated stroke from the given events. There must be at least two events,
    /// and the events must be in chronological order.
    pub fn from_events<'a>(
        events: impl IntoIterator<Item = &'a sdocx::page::object::stroke::Event> + 'a,
    ) -> InterpolatedStroke {
        // Combine events that occur at the same time so that the x, y and pressure functions are
        // well defined. It is sufficient to combine consecutive events with the same timestamps,
        // because the events are given in chronological order.
        let deduped = events
            .into_iter()
            .map(|ev| {
                (
                    f64::from(ev.timestamp),
                    ev.point.x,
                    ev.point.y,
                    f64::from(ev.pressure),
                )
            })
            .coalesce(|a @ (t0, x0, y0, p0), b @ (t1, x1, y1, p1)| {
                if t0 == t1 {
                    // Sometimes the two events are identical, but sometimes they are not.
                    // We just take the mean of the two.
                    Ok((t0, 0.5 * (x0 + x1), 0.5 * (y0 + y1), 0.5 * (p0 + p1)))
                } else {
                    Err((a, b))
                }
            });

        let (t, x, y, p): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) = itertools::multiunzip(deduped);

        let r = InterpolatedStroke::new(
            &Array1::from(t).view(),
            &Array1::from(x).view(),
            &Array1::from(y).view(),
            &Array1::from(p).view(),
        );

        // The data is in order, the arrays are all the same length, and the caller guarantees
        // that there is more than one event, so `unwrap` is safe here.
        r.unwrap()
    }
}

/// Smoothed stroke data and various time derivatives.
pub struct DerivativesX {
    /// Times corresponding to the other data.
    pub t: Array1<f64>,

    /// Smoothed x data.
    pub x: Array1<f64>,

    /// Smoothed y data.
    pub y: Array1<f64>,

    /// Smoothed pressure data.
    pub pressure: Array1<f64>,

    /// dx/dt
    x1: Array1<f64>,

    /// d^2x/dt^2
    x2: Array1<f64>,

    /// dy/dt
    y1: Array1<f64>,

    /// d^2y/dt^2
    y2: Array1<f64>,

    /// Curvature of the stroke. See https://en.wikipedia.org/wiki/Curvature#Plane_curves.
    pub curvature: Array1<f64>,

    /// d(pressure)/dt
    pressure1: Array1<f64>,
}

impl DerivativesX {
    /// Calculates various time derivatives of the components of `stroke` after filtering. Gaussian
    /// filtering is applied independently to the x, y and pressure data after sampling from
    /// `stroke` at `n_samples` times, with the first of these equal to `first_time` and the last
    /// equal to `last_time`. The standard deviation for the x and y filters is `xy_sd`, and the
    /// standard deviation for the pressure filter is `p_sd`.
    ///
    /// The unfiltered samples of the interpolated function are not consumed during the calculation
    /// of the derivatives, so are returned in case they are useful to the caller.
    pub fn new(
        stroke: &InterpolatedStroke,
        xy_sd: f64,
        p_sd: f64,
        first_time: f64,
        last_time: f64,
        n_samples: usize,
    ) -> InterpolateResult<DerivativesX> {
        /// Number of standard deviations wide the Gaussian kernels are.
        const TRUNC: usize = 4;

        let times = {
            let mut t = scirs2_core::Array1::linspace(first_time, last_time, n_samples);

            // `linspace` calculates the final value instead of setting it exactly, so sometimes
            // it is slightly greater than `last_time`. This causes an error when fed into
            // the interpolator because we do not have extrapolation enabled, so we set the
            // last time exactly here.
            *t.last_mut().unwrap() = last_time;

            t
        };

        let tv = times.view();

        // Sample the interpolated stroke data uniformly so the filtering makes sense. The raw
        // stroke data is not sampled uniformly (that is, the time delta is not always the same)
        // which is why we have to interpolate in the first place.
        let x_interp = stroke.x.evaluate_array(&tv)?;
        let y_interp = stroke.y.evaluate_array(&tv)?;
        let p_interp = stroke.pressure.evaluate_array(&tv)?;

        use ndarray_ndimage::gaussian_filter1d;

        let x1 = gaussian_filter1d(&x_interp, xy_sd, Axis(0), 1, BorderMode::Nearest, TRUNC);
        let x2 = gaussian_filter1d(&x_interp, xy_sd, Axis(0), 2, BorderMode::Nearest, TRUNC);
        let y1 = gaussian_filter1d(&y_interp, xy_sd, Axis(0), 1, BorderMode::Nearest, TRUNC);
        let y2 = gaussian_filter1d(&y_interp, xy_sd, Axis(0), 2, BorderMode::Nearest, TRUNC);
        let pressure1 = gaussian_filter1d(&p_interp, p_sd, Axis(0), 1, BorderMode::Nearest, TRUNC);

        let curvature = {
            // https://en.wikipedia.org/wiki/Curvature#Plane_curves
            // For more accurate calculation using floats, we rearrange the formula to
            //   |(x'/r)y'' - (y'/r)x''|/r^2
            // with r = sqrt(x'^2 + y'^2) calculated using `hypot` in order to avoid over/underflow
            // when we square the derivatives. We can use fused multiply-add for the numerator.
            // Note that x'/r and y'/r are the components of the unit tangent vector, and are
            // therefore in [-1,1].

            let mut c = x2.clone();

            // There is no vectorised `hypot` or FMA, so we need to loop anyway. We might as well
            // perform the whole calculation in that one loop.
            ndarray::azip!((c in &mut c, &y2 in &y2, &x1 in &x1, &y1 in &y1) {
                let x2 = *c;

                let r = x1.hypot(y1);

                // Tangent vector components.
                let tx = x1 / r;
                let ty = y1 / r;

                // todo: Kahan?
                *c = x2.mul_add(-ty, tx * y2).abs() / (r * r);

                // The curvature will be NaN if the division by r^2 fails. This means that x' and
                // y' are very small. In general this also means the numerator is very small, so we
                // deal with NaNs by replacing them with zeros.
                if !c.is_finite() {
                    *c = 0.0;
                }
            });

            c
        };

        let x = gaussian_filter1d(&x_interp, xy_sd, Axis(0), 0, BorderMode::Nearest, TRUNC);
        let y = gaussian_filter1d(&y_interp, xy_sd, Axis(0), 0, BorderMode::Nearest, TRUNC);
        let pressure = gaussian_filter1d(&p_interp, p_sd, Axis(0), 0, BorderMode::Nearest, TRUNC);

        Ok(DerivativesX {
            t: times,
            x,
            y,
            pressure,
            x1,
            x2,
            y1,
            y2,
            curvature,
            pressure1,
        })
    }
}
