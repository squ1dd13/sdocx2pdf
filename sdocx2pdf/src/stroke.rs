use itertools::Itertools;
use ndarray::{Array1, ArrayView1, Axis};
use ndarray_ndimage::BorderMode;
use scirs2_interpolate::{
    Interp1d, InterpolateResult, InterpolationMethod, PchipInterpolator, interp1d::ExtrapolateMode,
};

/// Interpolates between samples of stroke data to create a continuous mapping from time to
/// position and pressure.
pub struct InterpolatedStroke {
    /// Time of the first sample used for interpolation. The interpolated stroke is undefined at
    /// times strictly less than this.
    t_first: f64,

    /// Time of the last sample used for interpolation. The interpolated stroke is undefined at
    /// times strictly greater than this.
    t_last: f64,

    /// Interpolator for x values.
    x: PchipInterpolator<f64>,

    /// Interpolator for y values.
    y: PchipInterpolator<f64>,

    /// Interpolator for pressure values.
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
            t_first: *t.first().unwrap(),
            t_last: *t.last().unwrap(),
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
pub struct FilteredStroke {
    /// The times at which the samples of the original unsmoothed interpolated curve were taken.
    pub times: Array1<f64>,

    /// Filtered x data.
    pub x: Interp1d<f64>,

    /// Filtered dx/dt.
    x1: PchipInterpolator<f64>,

    /// Filtered d^2x/dt^2.
    x2: Interp1d<f64>,

    /// Filtered y data.
    pub y: Interp1d<f64>,

    /// Filtered dy/dt.
    y1: PchipInterpolator<f64>,

    /// Filtered d^2y/dt^2.
    y2: Interp1d<f64>,

    /// sqrt((dx/dt)^2 + (dy/dt)^2) for the filtered data.
    speed: PchipInterpolator<f64>,

    /// Curvature of the filtered data. See https://en.wikipedia.org/wiki/Curvature#Plane_curves.
    pub curvature: PchipInterpolator<f64>,

    /// Filtered pressure data.
    pub pressure: Interp1d<f64>,

    /// Filtered d(pressure)/dt.
    pressure1: Interp1d<f64>,

    /// Arc length as a function of time.
    arc_length_by_time: Interp1d<f64>,

    /// Time as a function of arc length.
    time_by_arc_length: Interp1d<f64>,
}

impl FilteredStroke {
    fn calc_speed_curvature(x1: f64, y1: f64, x2: f64, y2: f64) -> (f64, f64) {
        // https://en.wikipedia.org/wiki/Curvature#Plane_curves
        // For more accurate calculation using floats, we rearrange the formula to
        //   |(x'/s)y'' - (y'/s)x''|/s^2
        // with s = sqrt(x'^2 + y'^2), the speed of the stroke, calculated using `hypot` in
        // order to avoid over/underflow when we square the derivatives. We can use fused
        // multiply-add for the numerator. Note that x'/s and y'/s are the components of the
        // unit tangent vector, and are therefore in [-1,1].

        let speed = x1.hypot(y1);

        // Tangent vector components.
        let tx = x1 / speed;
        let ty = y1 / speed;

        // todo: Kahan?
        let curvature = x2.mul_add(-ty, tx * y2).abs() / (speed * speed);

        // The curvature will be NaN if the division by speed^2 fails. This means that x'
        // and y' are very small. In general this also means the numerator is very small,
        // so we deal with NaNs by replacing them with zeros.
        (
            speed,
            if curvature.is_finite() {
                curvature
            } else {
                0.0
            },
        )
    }

    /// Calculates various time derivatives of the components of `stroke` after filtering. Gaussian
    /// filtering is applied independently to the x, y and pressure data after sampling from
    /// `stroke` at `n_samples` times, with the first of these precisely at the start of the
    /// interpolated stroke, and the last precisely at the end of the interpolated stroke. The
    /// standard deviation for the x and y filters is `xy_sd`, and the standard deviation for the
    /// pressure filter is `p_sd`.
    pub fn new(
        stroke: &InterpolatedStroke,
        xy_sd: f64,
        p_sd: f64,
        n_samples: usize,
    ) -> InterpolateResult<FilteredStroke> {
        /// Number of standard deviations wide the Gaussian kernels are.
        const TRUNC: usize = 4;

        let times = {
            let mut t = scirs2_core::Array1::linspace(stroke.t_first, stroke.t_last, n_samples);

            // `linspace` calculates the final value instead of setting it exactly, so sometimes
            // it is slightly greater than `last_time`. This causes an error when fed into
            // the interpolator because we do not have extrapolation enabled, so we set the
            // last time exactly here.
            *t.last_mut().unwrap() = stroke.t_last;

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

        // hack: For some reason, when `order = 1`, the results are negated...? (Even vs. Python.)
        let x1 = -gaussian_filter1d(&x_interp, xy_sd, Axis(0), 1, BorderMode::Nearest, TRUNC);
        let x2 = gaussian_filter1d(&x_interp, xy_sd, Axis(0), 2, BorderMode::Nearest, TRUNC);
        let y1 = -gaussian_filter1d(&y_interp, xy_sd, Axis(0), 1, BorderMode::Nearest, TRUNC);
        let y2 = gaussian_filter1d(&y_interp, xy_sd, Axis(0), 2, BorderMode::Nearest, TRUNC);
        let pressure1 = -gaussian_filter1d(&p_interp, p_sd, Axis(0), 1, BorderMode::Nearest, TRUNC);

        let (speed, curvature) = {
            let mut s = x1.clone();
            let mut c = x2.clone();

            // There is no vectorised `hypot` or FMA, so we need to loop. We might as well perform
            // the whole calculation in that one loop. We can also use the loop to build the speed
            // array.
            ndarray::azip!((s in &mut s, c in &mut c, &y1 in &y1, &y2 in &y2) {
                (*s, *c) = Self::calc_speed_curvature(*s, y1, *c, y2);
            });

            (s, c)
        };

        let x = gaussian_filter1d(&x_interp, xy_sd, Axis(0), 0, BorderMode::Nearest, TRUNC);
        let y = gaussian_filter1d(&y_interp, xy_sd, Axis(0), 0, BorderMode::Nearest, TRUNC);
        let pressure = gaussian_filter1d(&p_interp, p_sd, Axis(0), 0, BorderMode::Nearest, TRUNC);

        // todo: Use ndarray methods
        let (arc_length, times_with_distinct_arc_lengths): (Vec<f64>, Vec<f64>) =
            std::iter::once(0.0)
                .chain(
                    x.iter()
                        .zip(&y)
                        .tuple_windows()
                        .map(|((&xa, &ya), (&xb, &yb))| (xa - xb).hypot(ya - yb))
                        .scan(0.0, |arc_length, dist| {
                            *arc_length += dist;
                            Some(*arc_length)
                        }),
                )
                .zip(&tv)
                .dedup_by(|(al0, _t0), (al1, _t1)| al0 == al1)
                .unzip();

        let arc_length = Array1::from_vec(arc_length);
        let times_with_distinct_arc_lengths = Array1::from_vec(times_with_distinct_arc_lengths);

        let time_by_arc_length = Interp1d::new(
            &arc_length.view(),
            &times_with_distinct_arc_lengths.view(),
            InterpolationMethod::Linear,
            ExtrapolateMode::Extrapolate,
        )
        .unwrap();

        let arc_length_by_time = Interp1d::new(
            &times_with_distinct_arc_lengths.view(),
            &arc_length.view(),
            InterpolationMethod::Linear,
            ExtrapolateMode::Extrapolate,
        )
        .unwrap();

        let interp = |v: Array1<f64>| {
            Interp1d::new(
                &tv,
                &v.view(),
                InterpolationMethod::Linear,
                ExtrapolateMode::Error,
            )
            // The data should all be valid for interpolation.
            .unwrap()
        };

        Ok(FilteredStroke {
            x: interp(x),
            x1: PchipInterpolator::new(&tv, &x1.view(), false)?,
            x2: interp(x2),
            y: interp(y),
            y1: PchipInterpolator::new(&tv, &y1.view(), false)?,
            y2: interp(y2),
            speed: PchipInterpolator::new(&tv, &speed.view(), false)?,
            curvature: PchipInterpolator::new(&tv, &curvature.view(), false)?,
            pressure: interp(pressure),
            pressure1: interp(pressure1),
            arc_length_by_time,
            time_by_arc_length,
            times,
        })
    }

    /// Returns the delta from `now` to the time at which the next sample of the filtered data
    /// should be taken in order to achieve an approximate absolute angle change of
    /// `abs_angle_delta`. The returned timestep is not clamped and may not be finite.
    fn compute_raw_timestep(&self, now: f64, abs_angle_delta: f64) -> InterpolateResult<f64> {
        // It can be shown that the curvature is equal to the magnitude of the rate of change
        // of the angle of the unit vector tangent to the stroke with respect to arc length.
        // That is, κ = |dT/ds| = |dθ/ds|, where T is the unit tangent, and θ is the angle of the
        // unit tangent. Since |dθ/ds| = |dθ/dt||dt/ds| and |dt/ds| = 1/u, where u is the speed
        // of the stroke, it follows that κ = |dθ/dt|/u. For sufficiently small Δt, then,
        // we have κ ≈ Δθ/(uΔt), where Δθ > 0 is the absolute angle change between the samples at t
        // and t + Δt. Therefore, to (approximately) achieve a given Δθ between the samples at
        // t and t + Δt, we can use Δt = Δθ/(κu).
        Ok(abs_angle_delta / (self.curvature.evaluate(now)? * self.speed.evaluate(now)?))
    }

    /// Returns a timestep in `[min_step, max_step]` that will approximately achieve an absolute
    /// angle change of `abs_angle_data` between the samples at `now` and `now + timestep`.
    /// If `now + max_step` is beyond the end of the interpolated stroke data, `now + timestep`
    /// may be as well.
    fn compute_timestep(
        &self,
        now: f64,
        abs_angle_delta: f64,
        min_step: f64,
        max_step: f64,
    ) -> InterpolateResult<f64> {
        self.compute_raw_timestep(now, abs_angle_delta).map(|dt| {
            if !dt.is_finite() {
                max_step
            } else {
                dt.clamp(min_step, max_step)
            }
        })
    }

    fn space_step_to_time_step(&self, t: f64, space_step: f64) -> f64 {
        // hack: This is nasty
        self.time_by_arc_length
            .evaluate(self.arc_length_by_time.evaluate(t).unwrap() + space_step)
            .unwrap()
            - t
    }

    fn time_step_to_space_step(&self, t: f64, time_step: f64) -> f64 {
        self.arc_length_by_time.evaluate(t + time_step).unwrap()
            - self.arc_length_by_time.evaluate(t).unwrap()
    }

    /// Returns an iterator yielding sample times such that the approximate stroke tangent angle
    /// change between consecutive samples is approximately `target_angle`. The first time is
    /// guaranteed to be the beginning of the interpolated stroke data, and the last time is
    /// guaranteed to be the end of it.
    pub fn compute_sample_times(
        &self,
        target_angle: f64,
        min_space_step: f64,
        max_space_step: f64,
    ) -> impl Iterator<Item = f64> {
        let t_first = *self.times.first().unwrap();
        let t_last = *self.times.last().unwrap();

        let mut t = t_first;

        std::iter::once(t).chain(std::iter::from_fn(move || {
            if t == t_last {
                return None;
            }

            let min_time_step = self.space_step_to_time_step(t, min_space_step);
            let max_time_step = self.space_step_to_time_step(t, max_space_step);

            t += self
                .compute_timestep(t, target_angle, min_time_step, max_time_step)
                // Unwrapping is OK here because `t` is guaranteed to be within the interpolated
                // range.
                .unwrap()
                .min(t_last);

            if t == t_last {
                return Some(t);
            }

            if t + min_time_step > t_last {
                // We are not at the end of the stroke, but we are so close to it that taking the
                // minimum timestep to the next sample would take us past the end. The usual `min`
                // strategy on the next step would then result in a timestep less than `min_step`,
                // and possibly so much so that the segment length would be numerically
                // problematic. It is better to risk slightly exceeding `max_step` here by stepping
                // directly to the final point.
                t = t_last;
            }

            Some(t)
        }))
    }

    pub fn arc_length_third_times(&self, t_start: f64, t_end: f64) -> (f64, f64) {
        let space_step = self.time_step_to_space_step(t_start, t_end - t_start);

        (
            t_start + self.space_step_to_time_step(t_start, space_step / 3.0),
            t_end + self.space_step_to_time_step(t_end, -space_step / 3.0),
        )
    }

    /// Returns the stroke velocity at time `t`.
    pub fn velocity(&self, t: f64) -> InterpolateResult<(f64, f64)> {
        Ok((self.x1.evaluate(t)?, self.y1.evaluate(t)?))
    }

    /// Returns the stroke position at time `t`.
    pub fn position(&self, t: f64) -> InterpolateResult<(f64, f64)> {
        Ok((self.x.evaluate(t)?, self.y.evaluate(t)?))
    }
}
