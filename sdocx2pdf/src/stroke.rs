use std::collections::VecDeque;

use euclid::default::Vector2D;
use itertools::{Either, Itertools};
use ndarray::{Array1, ArrayView1, Axis};
use ndarray_ndimage::BorderMode;
use num::ToPrimitive;
use scirs2_interpolate::{
    CubicSpline, Interp1d, InterpolateResult, InterpolationMethod, PchipInterpolator,
    interp1d::ExtrapolateMode,
    symbolic_derivative,
    traits::{ExtremaType, SplineInterpolator},
    utils::{find_multiple_roots, find_roots_bisection},
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

/// Time, position and pressure data for a stroke, split into four vectors. It is guaranteed that
/// the four vectors have the same length, that this length is at least 2, and that the data is
/// stored in chronological order.
pub struct SplitStroke {
    time: Vec<f64>,
    x: Vec<f64>,
    y: Vec<f64>,
    pressure: Vec<f64>,
}

impl SplitStroke {
    pub fn event_count(&self) -> usize {
        self.time.len()
    }
}

pub enum StrokeOrDot {
    Stroke(SplitStroke),
    Dot { x: f64, y: f64, pressure: f64 },
}

impl StrokeOrDot {
    /// Extracts clean stroke data from the given events. There must be at least one event, and if
    /// there is more than one, the events must be in chronological order.
    ///
    /// If the events all have the same position, a `Dot` is returned with that position and the
    /// maximum pressure value across all events. If the events are not all at the same position,
    /// the stroke data is cleaned by merging consecutive events with the same timestamps, and is
    /// returned as a `SplitStroke` with the data in the same order as it was obtained from the
    /// events.
    pub fn from_events<'a>(
        events: impl IntoIterator<Item = &'a sdocx::page::object::stroke::Event> + 'a,
    ) -> StrokeOrDot {
        // Combine consecutive events that occur at the same time. Since the events are in
        // chronological order, the effect is that for every time value we have, there is exactly
        // one event that occurs at that time.
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

        let (time, x, y, pressure): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) =
            itertools::multiunzip(deduped);

        assert!(!time.is_empty(), "there must be at least one event");

        if let (Ok(&x), Ok(&y)) = (x.iter().all_equal_value(), y.iter().all_equal_value()) {
            return StrokeOrDot::Dot {
                x,
                y,
                // Unwrapping is safe here because there is at least one event.
                pressure: pressure.into_iter().max_by(f64::total_cmp).unwrap(),
            };
        }

        // Since the x and y values are not all the same, we now know there are at least two
        // events in the cleaned data.
        StrokeOrDot::Stroke(SplitStroke {
            time,
            x,
            y,
            pressure,
        })
    }
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

    /// Interpolates the given stroke data.
    pub fn from_split_stroke(stroke: &SplitStroke) -> InterpolatedStroke {
        let r = InterpolatedStroke::new(
            &ArrayView1::from(&stroke.time),
            &ArrayView1::from(&stroke.x),
            &ArrayView1::from(&stroke.y),
            &ArrayView1::from(&stroke.pressure),
        );

        // The guarantees made by `SplitStroke` ensure that unwrapping is safe here.
        r.unwrap()
    }
}

#[derive(Clone, Copy)]
pub enum KeyTime {
    Start(f64),
    CurvatureExtremum(f64),
    PressureExtremum(f64),
    InflectionPoint(f64),
    End(f64),
}

impl KeyTime {
    pub fn to_time(self) -> f64 {
        match self {
            KeyTime::Start(t)
            | KeyTime::CurvatureExtremum(t)
            | KeyTime::PressureExtremum(t)
            | KeyTime::InflectionPoint(t)
            | KeyTime::End(t) => t,
        }
    }
}

impl std::cmp::Eq for KeyTime {}

impl std::cmp::Ord for KeyTime {
    fn cmp(&self, other: &KeyTime) -> std::cmp::Ordering {
        self.to_time().total_cmp(&other.to_time())
    }
}

impl std::cmp::PartialEq for KeyTime {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Start(l0), Self::Start(r0)) => l0 == r0,
            (Self::CurvatureExtremum(l0), Self::CurvatureExtremum(r0)) => l0 == r0,
            (Self::PressureExtremum(l0), Self::PressureExtremum(r0)) => l0 == r0,
            (Self::InflectionPoint(l0), Self::InflectionPoint(r0)) => l0 == r0,
            (Self::End(l0), Self::End(r0)) => l0 == r0,
            _ => false,
        }
    }
}

impl std::cmp::PartialOrd for KeyTime {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Smoothed stroke data and various time derivatives.
pub struct FilteredStroke {
    /// The times at which the samples of the original unsmoothed interpolated curve were taken.
    pub times: Array1<f64>,

    /// Filtered x data.
    pub x: Interp1d<f64>,

    /// Filtered dx/dt.
    x1: Interp1d<f64>,

    /// Filtered d^2x/dt^2.
    x2: Interp1d<f64>,

    /// Filtered y data.
    pub y: Interp1d<f64>,

    /// Filtered dy/dt.
    y1: Interp1d<f64>,

    /// Filtered d^2y/dt^2.
    y2: Interp1d<f64>,

    /// sqrt((dx/dt)^2 + (dy/dt)^2) for the filtered data.
    speed: Interp1d<f64>,

    /// Curvature of the filtered data. See https://en.wikipedia.org/wiki/Curvature#Plane_curves.
    ///
    /// We use a cubic spline for this so that we can easily find extrema via differentiation.
    curvature: CubicSpline<f64>,

    curvature_by_arc_length: CubicSpline<f64>,

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
        //   ((x'/s)y'' - (y'/s)x'')/s^2
        // with s = sqrt(x'^2 + y'^2), the speed of the stroke, calculated using `hypot` in
        // order to avoid over/underflow when we square the derivatives. We can use fused
        // multiply-add for the numerator. Note that x'/s and y'/s are the components of the
        // unit tangent vector, and are therefore in [-1,1].

        let speed = x1.hypot(y1);

        // Tangent vector components.
        let tx = x1 / speed;
        let ty = y1 / speed;

        // todo: Kahan?
        let curvature = x2.mul_add(-ty, tx * y2) / (speed * speed);

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
    ///
    /// `n_samples` must be at least 3.
    pub fn new(
        stroke: &InterpolatedStroke,
        xy_sd: f64,
        p_sd: f64,
        n_samples: usize,
    ) -> InterpolateResult<FilteredStroke> {
        /// Number of standard deviations wide the Gaussian kernels are.
        const TRUNC: usize = 6;

        assert!(n_samples >= 3);

        let times = {
            let mut t = scirs2_core::Array1::linspace(stroke.t_first, stroke.t_last, n_samples);

            // `linspace` calculates the final value instead of setting it exactly, so sometimes
            // it is slightly greater than `last_time`. This causes an error when fed into
            // the interpolator because we do not have extrapolation enabled, so we set the
            // last time exactly here.
            *t.last_mut().unwrap() = stroke.t_last;

            t
        };

        let time_step = times[1] - times[0];

        let tv = times.view();

        // Sample the interpolated stroke data uniformly so the filtering makes sense. The raw
        // stroke data is not sampled uniformly (that is, the time delta is not always the same)
        // which is why we have to interpolate in the first place.
        let x_interp = stroke.x.evaluate_array(&tv)?;
        let y_interp = stroke.y.evaluate_array(&tv)?;
        let p_interp = stroke.pressure.evaluate_array(&tv)?;

        use ndarray_ndimage::gaussian_filter1d;

        // `gaussian_filter1d` with `order = k` gives us the kth derivative of the input with
        // respect to the index. Since n = t/Δt we have dn/dt = 1/Δt, so by the chain rule,
        // du/dt = (du/dn)/Δt and d^2u/du^2 = (d^2u/dn^2)/(Δt^2).
        // fixme: The denominators cancel in the curvature, so we should only include them after
        // calculating that.

        // hack: For some reason, when `order = 1`, the results are negated...? (Even vs. Python.)
        let x1 = -gaussian_filter1d(&x_interp, xy_sd, Axis(0), 1, BorderMode::Nearest, TRUNC)
            / time_step;

        let x2 = gaussian_filter1d(&x_interp, xy_sd, Axis(0), 2, BorderMode::Nearest, TRUNC)
            / (time_step * time_step);

        let y1 = -gaussian_filter1d(&y_interp, xy_sd, Axis(0), 1, BorderMode::Nearest, TRUNC)
            / time_step;

        let y2 = gaussian_filter1d(&y_interp, xy_sd, Axis(0), 2, BorderMode::Nearest, TRUNC)
            / (time_step * time_step);

        let pressure1 =
            -gaussian_filter1d(&p_interp, p_sd, Axis(0), 1, BorderMode::Nearest, TRUNC) / time_step;

        let (speed, curvature) = {
            let mut s = x1.clone();
            let mut sc = x2.clone();

            // There is no vectorised `hypot` or FMA, so we need to loop. We might as well perform
            // the whole calculation in that one loop. We can also use the loop to build the speed
            // array.
            ndarray::azip!((s in &mut s, sc in &mut sc, &y1 in &y1, &y2 in &y2) {
                (*s, *sc) = Self::calc_speed_curvature(*s, y1, *sc, y2);
            });

            (s, sc)
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

        let curvature_by_time = CubicSpline::new(&tv, &curvature.view()).unwrap();
        let curvature_by_arc_length = CubicSpline::with_boundary_condition(
            &arc_length_by_time.evaluate_array(&tv).unwrap().view(),
            &curvature.view(),
            scirs2_interpolate::SplineBoundaryCondition::NotAKnot,
        )
        .unwrap();

        Ok(FilteredStroke {
            x: interp(x),
            x1: interp(x1),
            x2: interp(x2),
            y: interp(y),
            y1: interp(y1),
            y2: interp(y2),
            speed: interp(speed),
            curvature: curvature_by_time,
            curvature_by_arc_length,
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
        // It can be shown that the curvature is equal to the rate of change
        // of the angle of the unit vector tangent to the stroke with respect to arc length.
        // Thus, |κ| = |dT/ds| = |dθ/ds|, where T is the unit tangent, and θ is the angle of the
        // unit tangent. Since |dθ/ds| = |dθ/dt||dt/ds| and |dt/ds| = 1/u, where u is the speed
        // of the stroke, it follows that |κ| = |dθ/dt|/u. For sufficiently small Δt, then,
        // we have |κ| ≈ Δθ/(uΔt), where Δθ > 0 is the absolute angle change between the samples at
        // t and t + Δt. Therefore, to (approximately) achieve a given Δθ between the samples at
        // t and t + Δt, we can use Δt = Δθ/(|κ|u).
        Ok(abs_angle_delta / (self.curvature.evaluate(now)?.abs() * self.speed.evaluate(now)?))
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
        let raw = self.compute_raw_timestep(now, abs_angle_delta)?;

        if !raw.is_finite() {
            return Ok(max_step);
        }

        if raw >= max_step {
            return Ok(max_step);
        }

        if raw <= min_step {
            return Ok(min_step);
        }

        // The calculated step is strictly within the bounds, so there is room to adjust it either
        // way to achieve a better approximation of the target angle.
        let mut current_step = raw;

        let vel_now = Vector2D::from(self.velocity(now)?);

        let angle_diff_with_step = |step: f64| {
            self.velocity(now + step).map(|vel_after| {
                (vel_now.angle_to(Vector2D::from(vel_after)).radians.abs() - abs_angle_delta).abs()
            })
        };

        let mut min_step_search = min_step;
        let mut max_step_search = max_step;

        // We should already be close to the target angle, so only perform a few iterations.
        for _ in 0..5 {
            let possible_steps = [
                current_step,
                0.5 * (current_step + max_step_search),
                0.5 * (current_step + min_step_search),
            ];

            // todo: Reuse angle calculated for the best step on the previous iteration.

            // Find the step which minimises the difference between the target angle and the
            // realised angle.
            let Some((best_step, _)) = possible_steps
                .iter()
                .flat_map(|&step| angle_diff_with_step(step).map(|a| (step, a)))
                .min_by(|(_, a1), (_, a2)| a1.total_cmp(a2))
            else {
                // Failed to calculate the angle for any of the steps. Give up.
                break;
            };

            if best_step == current_step {
                break;
            }

            if best_step > current_step {
                min_step_search = current_step;
            } else {
                max_step_search = current_step;
            }

            current_step = best_step;
        }

        Ok(current_step)
    }

    pub fn space_step_to_time_step(&self, t: f64, space_step: f64) -> f64 {
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
        max_time_step: f64,
    ) -> impl Iterator<Item = f64> {
        let t_first = *self.times.first().unwrap();
        let t_last = *self.times.last().unwrap();

        let mut t = t_first;

        std::iter::once(t).chain(std::iter::from_fn(move || {
            if t == t_last {
                return None;
            }

            let min_time_step = self.space_step_to_time_step(t, min_space_step);

            let clamped_time_step = if min_time_step >= max_time_step {
                min_time_step
            } else {
                self.compute_timestep(t, target_angle, min_time_step, max_time_step)
                    // Unwrapping is OK here because `t` is guaranteed to be within the
                    // interpolated range.
                    .unwrap()
            };

            t += clamped_time_step.min(t_last);

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

    pub fn key_times(&self) -> impl Iterator<Item = KeyTime> {
        let t_start = self.times[0];
        let t_end = *self.times.last().unwrap();

        let t_delta = self.times[1] - t_start;

        let extremum_tolerance = t_delta * 10.0;

        // To avoid duplicating `t_start` and `t_end`, we do not look for extrema within one time
        // step of either end of the stroke. The tolerance is the minimum interval width for the
        // bisection method.
        let curvature_extrema = self
            .curvature
            .find_extrema(&[(t_start + t_delta, t_end - t_delta)], extremum_tolerance)
            .unwrap();

        // fixme: 10% of the smallest _extremum_ is a stupid root-finding tolerance
        // But also, the algorithm uses the tolerance we give it both for the interval width (unit:
        // time) and for the distance of the curvature from zero (unit: absolute curvature). These
        // are not comparable. So what should it be?
        let root_tolerance = 0.1
            * curvature_extrema
                .iter()
                .map(|(_t, curvature, _)| curvature.abs())
                .min_by(f64::total_cmp)
                .map(|rt| rt.min(extremum_tolerance))
                .unwrap_or(extremum_tolerance);

        // Roots of the curvature are inflection points of the stroke.
        let inflection_points = self
            .curvature
            .find_roots(&[(t_start + t_delta, t_end - t_delta)], root_tolerance)
            .unwrap();

        // todo: Could we combine the root finding with the extremum finding?
        // We should re-implement them anyway, because the crate implementations aren't ideal

        std::iter::once(KeyTime::Start(t_start))
            .chain(
                curvature_extrema
                    .into_iter()
                    .map(|(t, _, _)| KeyTime::CurvatureExtremum(t))
                    .chain(inflection_points.into_iter().map(KeyTime::InflectionPoint))
                    .sorted_unstable(),
            )
            .chain(std::iter::once(KeyTime::End(t_end)))
    }

    pub fn compute_sample_times_strictly_between(
        &self,
        start_excl: f64,
        end_excl: f64,
        target_angle: f64,
        min_space_step: f64,
        max_time_step: f64,
    ) -> impl Iterator<Item = f64> {
        // Calculate the earliest time we can sample at according to the minimum step and the
        // exclusive start time.
        let start_min_time_step = self.space_step_to_time_step(start_excl, min_space_step);
        let earliest_allowed = start_excl + start_min_time_step;

        // Calculate the latest time we can sample at according to the minimum step and the
        // exclusive end time.
        let end_min_backwards_time_step = -self.space_step_to_time_step(end_excl, -min_space_step);
        let latest_allowed = end_excl - end_min_backwards_time_step;

        if earliest_allowed > latest_allowed {
            // The start and end times are too close together to fit a sample in between.
            return Either::Left(std::iter::empty());
        }

        // todo: Iterators
        let ltr_sample_times = {
            let mut ltr_s_t = Vec::new();

            let mut current_time = start_excl;
            let mut min_time_step = start_min_time_step;

            loop {
                let step = self
                    .compute_timestep(current_time, target_angle, min_time_step, max_time_step)
                    // Safe to unwrap as long as `start_excl` and `end_excl` are within
                    // interpolated bounds.
                    .unwrap();

                let updated_time = current_time + step;

                if updated_time > latest_allowed {
                    break;
                }

                ltr_s_t.push(updated_time);

                current_time = updated_time;
                min_time_step = self.space_step_to_time_step(current_time, min_space_step);
            }

            ltr_s_t
        };

        let rtl_sample_times = {
            let mut rtl_s_t = Vec::new();

            let mut current_time = end_excl;
            let mut min_backwards_time_step = end_min_backwards_time_step;

            loop {
                // todo: Time steps for a target angle change are symmetric...? Think this through.
                let backwards_step = self
                    .compute_timestep(
                        current_time,
                        target_angle,
                        min_backwards_time_step,
                        max_time_step,
                    )
                    .unwrap();

                let updated_time = current_time - backwards_step;

                if updated_time < earliest_allowed {
                    break;
                }

                rtl_s_t.push(updated_time);

                current_time = updated_time;
                min_backwards_time_step =
                    -self.space_step_to_time_step(current_time, -min_space_step);
            }

            rtl_s_t
        };

        let mut fwd_times = Vec::new();
        let mut bwd_times = Vec::new();

        for (forwards_time, backwards_time) in ltr_sample_times.into_iter().zip(rtl_sample_times) {
            // fixme: We've already calculated the minimum steps at both times, so we don't need to
            // find the arc lengths again

            // Note that the difference will be negative if the forwards time is ahead of the
            // backwards time, so in that case it is definitely below the minimum.
            if self.arc_length_by_time.evaluate(backwards_time).unwrap()
                - self.arc_length_by_time.evaluate(forwards_time).unwrap()
                <= min_space_step
            {
                let middle_time = 0.5 * (forwards_time + backwards_time);

                let last_fwd = fwd_times.last().copied().unwrap_or(start_excl);
                let last_bwd = bwd_times.last().copied().unwrap_or(end_excl);

                // todo: Do we need to check the max?
                // We can sample at the midpoint of the forwards and backwards times as long as
                // it is sufficiently far from the times either side.
                if self.time_step_to_space_step(last_fwd, middle_time - last_fwd) > min_space_step
                    && -self.time_step_to_space_step(last_bwd, middle_time - last_bwd)
                        > min_space_step
                {
                    fwd_times.push(middle_time);
                }

                break;
            }

            fwd_times.push(forwards_time);
            bwd_times.push(backwards_time);
        }

        Either::Right(fwd_times.into_iter().chain(bwd_times.into_iter().rev()))
    }

    pub fn compute_sample_times_from_key_times(
        &self,
        target_angle: f64,
        min_space_step: f64,
        max_time_step: f64,
    ) -> impl Iterator<Item = f64> {
        self.key_times()
            .map(Some)
            .chain(std::iter::once(None))
            .tuple_windows()
            .map(move |(a, b)| {
                let (Some(a), Some(b)) = (a, b) else {
                    // Dummy pair to let us yield the final key time as a sample time.
                    return Either::Right([a.unwrap().to_time()]);
                };

                let a = a.to_time();
                let b = b.to_time();

                Either::Left(std::iter::once(a).chain({
                    let lcs = LinearCurvatureSegment::new(
                        self.arc_length_by_time.evaluate(a).unwrap(),
                        self.arc_length_by_time.evaluate(b).unwrap(),
                        &self.curvature_by_arc_length,
                    )
                    .unwrap();

                    lcs.sample_points_strictly_inside(target_angle)
                        .map(|s| self.time_by_arc_length.evaluate(s).unwrap())
                }))
                // Either::Left(
                //     std::iter::once(a).chain(self.compute_sample_times_strictly_between(
                //         a,
                //         b,
                //         target_angle,
                //         min_space_step,
                //         max_time_step,
                //     )),
                // )
            })
            .flat_map(|x| x.into_iter())
    }
}

struct LinearCurvatureSegment {
    /// The arc length between the start of the whole stroke and the start of this segment. This is
    /// only used to offset local arc lengths to get global ones. It is not used in calculations.
    s_start: f64,

    /// The length L of the segment.
    length: f64,

    /// κ0, the absolute curvature at the beginning of the segment. Locally, we consider this to be
    /// the value of the curvature when s = 0.
    abs_curv_start: f64,

    /// κ1, the absolute curvature at the end of the segment. Locally, we consider this to be the
    /// value of the curvature at s = L.
    abs_curv_end: f64,

    /// The accumulated absolute angle change across the segment.
    accumulated_abs_angular_change: f64,
}

impl LinearCurvatureSegment {
    /// Creates a new linear curvature segment from the given curvature data in the region where
    /// the arc length is in `[s_start, s_end]`. There must be no zeros or local extrema of
    /// curvature in this region.
    fn new(
        s_start: f64,
        s_end: f64,
        curvature_by_arc_length: &CubicSpline<f64>,
    ) -> InterpolateResult<LinearCurvatureSegment> {
        let length = s_end - s_start;

        Ok(LinearCurvatureSegment {
            s_start,

            length,

            abs_curv_start: curvature_by_arc_length.evaluate(s_start)?.abs(),
            abs_curv_end: curvature_by_arc_length.evaluate(s_end)?.abs(),

            // Since the sign of the curvature does not change between `s_start` and `s_end`,
            // we can integrate and take the absolute value instead of integrating the absolute
            // curvature.
            accumulated_abs_angular_change: curvature_by_arc_length
                .integrate(s_start + length * 0.1, s_end - length * 0.1)?
                .abs(),
        })
    }

    /// Returns the gradient of the curvature when we approximate it as linear.
    fn true_curvature_gradient(&self) -> f64 {
        (self.abs_curv_end - self.abs_curv_start) / self.length
    }

    /// Returns the curvature gradient we can use in our linear model to get a perfect segmentation
    /// when we use a segment angle that perfectly divides the accumulated angular change.
    fn adjusted_curvature_gradient(&self) -> f64 {
        // todo: Explain
        (2.0 * (self.accumulated_abs_angular_change - self.abs_curv_start * self.length))
            / (self.length * self.length)
    }

    fn adjusted_model_is_valid(&self) -> bool {
        // Since we are working with absolute curvature, the adjusted linear model is invalid if
        // it predicts a negative curvature. The adjusted model has the form κ(s) = cs + κ0,
        // where c is the adjusted curvature gradient. Since κ is assumed to be linear and
        // κ(0) = κ0 >= 0, we can only have κ(s) < 0 for some s if we also have κ(L) < 0.
        // Therefore, it suffices to check that cL + κ0 >= 0. Using the definition of the adjusted
        // gradient c, this reduces to the following condition:
        self.accumulated_abs_angular_change >= 0.5 * self.abs_curv_start * self.length
    }

    // hack: `&self`
    fn spsi_adjusted(self, tgt_ss_angle: f64) -> impl Iterator<Item = f64> {
        let tgt_ss_count = self.accumulated_abs_angular_change / tgt_ss_angle;

        // In general the target angle delta does not divide the accumulated angle change,
        // so the target subsegment count will not be an integer. We round it to get an integer
        // count and an adjusted target subsegment angle. This gives us a perfect subsegmentation
        // when we use the adjusted curvature gradient and angle.
        let adj_ss_count = tgt_ss_count.round();
        let adj_ss_angle = self.accumulated_abs_angular_change / adj_ss_count;
        let adj_ss_count = adj_ss_count.to_u32().unwrap();

        if adj_ss_count <= 1 {
            // We already have one subsegment: the whole thing.
            return Either::Left(std::iter::empty());
        }

        // todo: Remove
        assert!(adj_ss_angle.is_finite());

        let acs = self.abs_curv_start;
        let c = self.adjusted_curvature_gradient();

        // Solution to the quadratic equation we get when we integrate the adjusted linear model
        // between 0 and sn to get the accumulated angle change over the subsegments up to and
        // including the nth and set it equal to n times the adjusted target angle per subsegment.
        // todo: Explain better
        let sn = move |n: u32| -> f64 {
            // As in the fallback, we have to consider the case c = 0, where curvature is const
            // fixme: Epsilon
            if c.abs() < f64::EPSILON {
                (f64::from(n) * adj_ss_angle) / acs
            } else {
                (-acs + (acs * acs + 2.0 * c * f64::from(n) * adj_ss_angle).sqrt()) / c
            }
        };

        // s0 = 0 and sN = L (where N is the adjusted count), so we only care about
        // s1, s2, ..., s_{N-1}.

        // hack: take 100 stops us spinning when the accumulated angle is stupidly big from
        // numerical instability
        Either::Right((1..adj_ss_count).map(sn).take(100))
    }

    // hack: `&self`
    fn spsi_fallback(self, tgt_ss_angle: f64) -> impl Iterator<Item = f64> {
        if tgt_ss_angle >= self.accumulated_abs_angular_change {
            // No need to subsegment.
            return Either::Left(std::iter::empty());
        }

        let acs = self.abs_curv_start;
        let c = self.true_curvature_gradient();

        let sn = move |n: u32| -> f64 {
            // fixme: Is epsilon right here? In any case, explain better:
            // When c = 0 we have constant curvature, so we have to do this instead
            if c.abs() < f64::EPSILON {
                (f64::from(n) * tgt_ss_angle) / acs
            } else {
                (-acs + (acs * acs + 2.0 * c * f64::from(n) * tgt_ss_angle).sqrt()) / c
            }
        };

        let subseg_count_limit = (self.accumulated_abs_angular_change / tgt_ss_angle)
            .ceil()
            .to_usize()
            .unwrap()
            .min(100);

        // s0 = 0, so start from 1. Keep going until we reach the end of the segment.
        Either::Right(
            (1..)
                .map(sn)
                .take_while({
                    let length = self.length;
                    move |&sn| sn < length
                })
                .take(subseg_count_limit),
        )
    }

    // hack: This takes ownership of `self` to work around lifetime capture rules
    fn sample_points_strictly_inside(
        self,
        tgt_subseg_angle_delta: f64,
    ) -> impl Iterator<Item = f64> {
        let s_start = self.s_start;

        if self.adjusted_model_is_valid() {
            // eprintln!("Adjusted");
            Either::Left(self.spsi_adjusted(tgt_subseg_angle_delta))
        } else {
            // eprintln!("Fallback");
            Either::Right(self.spsi_fallback(tgt_subseg_angle_delta))
        }
        .into_iter()
        .map(move |s| s + s_start)
    }
}
