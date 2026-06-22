use std::{borrow::Cow, collections::VecDeque};

use euclid::default::Vector2D;
use itertools::{Either, Itertools};
use kahan::KahanSum;
use lerp::Lerp;
use ndarray::{Array1, ArrayView1, Axis};
use ndarray_ndimage::BorderMode;
use num::ToPrimitive;
use scirs2_interpolate::{
    CubicSpline, Interp1d, InterpolateResult, InterpolationMethod, PchipInterpolator,
    PiecewisePolynomial,
    interp1d::ExtrapolateMode,
    symbolic_derivative::{self, Differentiable},
    traits::{ExtremaType, SplineInterpolator},
    utils::{find_multiple_roots, find_roots_bisection},
};

/// A root of a function that takes `f64` values.
enum Root {
    Interval(f64, f64),
    Single(f64),
}

/// Piecewise linear function constructed by linearly interpolating between finitely many points of
/// a function `f64 -> f64`.
struct Linterpolator<'v> {
    independent: Cow<'v, [f64]>,
    dependent: Cow<'v, [f64]>,
}

impl<'v> Linterpolator<'v> {
    /// Returns the index of the interval containing `t` in the list `[t0, t1), [t1, t2), ...
    /// [tn-1, tn), [tn, tn]`, where the `ti` are the interpolated data points for the independent
    /// variable.
    fn interval_index(&self, t: f64) -> Option<usize> {
        if !t.is_finite() {
            return None;
        }

        // Find the index of the first independent data point strictly greater than `t`.
        // `t` lies in the interval immediately before that.
        let idx_first_gt = self.independent.partition_point(|&v| v <= t);

        // If `idx_first_gt == 0`, then `t` is below the range for which we have data.
        let idx_last_le = idx_first_gt.checked_sub(1)?;

        if idx_first_gt == self.independent.len() {
            // No independent data point is strictly greater than `t`. `t` may be equal to the
            // largest data point, in which case we use that as the "interval", since we do have
            // data for it.
            (self.independent[idx_last_le] == t).then_some(idx_last_le)
        } else {
            Some(idx_last_le)
        }
    }

    /// Returns `f(t)` where `f` is the piecewise linear function, or `None` if `t` is outside the
    /// range of the original interpolated data.
    fn evaluate(&self, t: f64) -> Option<f64> {
        let idx_before = self.interval_index(t)?;

        let t_before = self.independent[idx_before];
        let v_before = self.dependent[idx_before];

        if t_before == t {
            return Some(v_before);
        }

        // `idx_before + 1` is a valid index except when `interval_index` returned the index of
        // the last independent data point, but we've already handled that case above.
        let t_after = self.independent[idx_before + 1];
        let v_after = self.dependent[idx_before + 1];

        Some(v_before.lerp(v_after, (t - t_before) / (t_after - t_before)))
    }

    /// Integrates the piecewise linear function exactly between `t0` and `t1` using the trapezium
    /// rule. The result approximates the integral of the interpolated function between those
    /// points.
    ///
    /// Returns `None` if either bound is outside the range of the interpolated data.
    fn integrate(&self, t0: f64, t1: f64) -> Option<f64> {
        if t0 > t1 {
            return self.integrate(t1, t0).map(|x| -x);
        }

        let t0_idx = self.interval_index(t0)?;
        let t1_idx = self.interval_index(t1)?;

        // Only check for equality now we know that both bounds are valid. We don't want to return
        // an apparently meaningful result when the input is invalid.
        if t0 == t1 {
            return Some(0.0);
        }

        if t0_idx == t1_idx {
            return Some(
                0.5 * (t1 - t0) * (self.evaluate(t0).unwrap() + self.evaluate(t1).unwrap()),
            );
        }

        // Since `t0 < t1`, we know there is a point to the right of `t0`.
        let t0_after = self.independent[t0_idx + 1];
        let t0_ival_contrib =
            0.5 * (t0_after - t0) * (self.evaluate(t0).unwrap() + self.dependent[t0_idx + 1]);

        let t1_ival_contrib = if t1_idx + 1 == self.independent.len() {
            // `t1` is the final independent data point, so is exactly at the beginning of its
            // "interval".
            0.0
        } else {
            let t1_before = self.independent[t1_idx];
            0.5 * (t1 - t1_before) * (self.dependent[t1_idx] + self.evaluate(t1).unwrap())
        };

        Some(
            [t0_ival_contrib, t1_ival_contrib]
                .into_iter()
                .chain(((t0_idx + 1)..t1_idx).map(|i| {
                    let &[ta, tb] = &self.independent[i..=(i + 1)].try_into().unwrap();
                    let &[va, vb] = &self.dependent[i..=(i + 1)].try_into().unwrap();

                    // todo: We could precompute these
                    0.5 * (tb - ta) * (va + vb)
                }))
                .fold(KahanSum::<f64>::new(), |ks, contrib| ks + contrib)
                .sum(),
        )
    }

    /// Returns an iterator yielding the roots of the piecewise linear function, up to `tolerance`,
    /// in ascending order.
    ///
    /// At most one root is returned per pair of interpolated data points.
    fn roots<'me>(&'me self, tolerance: f64) -> impl Iterator<Item = Root> + 'me {
        // todo: Document behaviour properly regarding intervals/single roots

        assert!(tolerance >= 0.0);

        self.independent
            .iter()
            .zip(self.dependent.iter())
            .tuple_windows()
            .flat_map(move |((&ta, &va), (&tb, &vb))| {
                let va_is = va.abs() <= tolerance;
                let vb_is = vb.abs() <= tolerance;

                if va_is && vb_is {
                    return Some(Root::Interval(ta, tb));
                }

                // `signum` gives different values for `0.0` and `-0.0`, but we needn't worry about
                // that because the check above rules out `va` and `vb` both being ±0.
                let same_sign = va.signum() == vb.signum();

                if same_sign {
                    return Some(match (va_is, vb_is) {
                        (true, false) => Root::Single(ta),
                        (false, true) => Root::Single(tb),
                        (false, false) => return None,
                        _ => unreachable!(),
                    });
                }

                // The endpoints have different signs, so there is a root in this interval.
                let calculated_root = {
                    // ta - (va * (tb - ta)) / (vb - va)
                    let root = ((tb - ta) / (vb - va)).mul_add(-va, ta);

                    // Mathematically, `root` is guaranteed to be in the interval. In
                    // floating-point arithmetic, however, we can't be sure. This is about the best
                    // we can reasonably do.
                    root.clamp(ta, tb)
                };

                // If one of the endpoints is below the tolerance, construct an interval between
                // that endpoint and the calculated root. (Because of the clamp, we have to check
                // for inequality before doing this.) Otherwise, return the calculated root only.
                Some(if va_is && calculated_root != ta {
                    Root::Interval(ta, calculated_root)
                } else if vb_is && calculated_root != tb {
                    Root::Interval(calculated_root, tb)
                } else {
                    Root::Single(calculated_root)
                })
            })
            // Remove duplicate roots, merge adjacent intervals, and absorb single roots into
            // intervals that contain them.
            .coalesce(|ra, rb| {
                Ok(match (ra, rb) {
                    // Merge
                    (Root::Interval(a, b), Root::Interval(c, d)) if b == c => Root::Interval(a, d),
                    (ra @ Root::Single(a), Root::Single(b)) if a == b => ra,

                    // Absorb
                    (ab @ Root::Interval(_, b), Root::Single(c)) if b == c => ab,
                    (Root::Single(a), bc @ Root::Interval(b, _)) if a == b => bc,

                    other => return Err(other),
                })
            })
    }

    /// Returns an iterator yielding the local extrema of the piecewise linear function in the form
    /// `(t, f(t))`, ordered by ascending `t`. The first and last points are not included.
    fn extrema(&self) -> impl Iterator<Item = (f64, f64)> {
        self.independent
            .iter()
            .zip(self.dependent.iter())
            .tuple_windows()
            .flat_map(|((_, va), (tb, vb), (_, vc))| {
                ((va < vb && vc < vb) || (vb < va && vb < vc)).then_some((*tb, *vb))
            })
    }
}

/// An arc length that identifies a point of interest along a stroke.
#[derive(Clone, Copy)]
pub enum Feature {
    /// The beginning of the stroke.
    Start,

    /// A curvature extremum.
    Vertex(f64),

    /// A point of zero curvature.
    Inflection(f64),

    /// The end of the stroke.
    End(f64),
}

impl Feature {
    pub fn arc_length(self) -> f64 {
        match self {
            Feature::Start => 0.0,
            Feature::Vertex(s) | Feature::Inflection(s) | Feature::End(s) => s,
        }
    }
}

pub struct ContinuousStroke {
    /// Arc length at the end of the stroke.
    length: f64,

    /// x position as a function of arc length.
    x: PchipInterpolator<f64>,

    /// y position as a function of arc length.
    y: PchipInterpolator<f64>,

    /// Horizontal component of the unit tangent vector as a function of arc length.
    tx: PiecewisePolynomial<f64>,

    /// Vertical component of the unit tangent vector as a function of arc length.
    ty: PiecewisePolynomial<f64>,

    /// Pressure as a function of arc length.
    p: Linterpolator<'static>,

    /// Curvature as a function of arc length.
    curvature: Linterpolator<'static>,
}

impl ContinuousStroke {
    fn calc_curvature(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
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

        // The curvature will be NaN if the division by speed^2 fails. This means that x' and
        // y' are very small. In general this also means the numerator is very small, so we
        // deal with NaNs by replacing them with zeros.
        if curvature.is_finite() {
            curvature
        } else {
            0.0
        }
    }

    pub fn new(split: &SplitStroke) -> ContinuousStroke {
        // The start and end times are respectively the first and last, and they will be distinct.
        // Both of these facts follow from the guarantees made by `SplitStroke`.
        let t_start = split.time[0];
        let t_end = *split.time.last().unwrap();

        let t_arr_v: ArrayView1<f64> = (&split.time).into();
        let xt_arr_v: ArrayView1<f64> = (&split.x).into();
        let yt_arr_v: ArrayView1<f64> = (&split.y).into();
        let pt_arr_v: ArrayView1<f64> = (&split.pressure).into();

        // Construct initial interpolators for x, y and pressure as functions of time.
        // We use PCHIP in order to best preserve the shape of the stroke and pressure curve.
        // Unwrapping is OK because of `SplitStroke`'s guarantees.
        let xt = PchipInterpolator::new(&t_arr_v, &xt_arr_v, false).unwrap();
        let yt = PchipInterpolator::new(&t_arr_v, &yt_arr_v, false).unwrap();
        let pt = PchipInterpolator::new(&t_arr_v, &pt_arr_v, false).unwrap();

        // We want an arc-length parameterisation rather than a time parameterisation. We need new
        // interpolators for this, and we'll construct them using data points from the current
        // interpolators, sampled at a greater rate than the original stroke data.

        // Generally, the slowest parts of the stroke are the most interesting. We resample using
        // uniform timesteps so that we still end up sampling more often (with respect to arc
        // length) in the areas of greatest interest. Using uniform timesteps is necessary for the
        // Gaussian filters we apply to smooth the data, because we need the array indices to be
        // proportional to the time in order for the index-based Gaussian filter to be meaningful.
        let t_resamples = {
            const TIME_RESAMPLING_RATIO: u8 = 15;

            let mut t_rs = scirs2_core::Array1::linspace(
                t_start,
                t_end,
                t_arr_v.len() * (TIME_RESAMPLING_RATIO as usize),
            );

            // `linspace` calculates the final value instead of setting it exactly, so sometimes it
            // is slightly greater than the end time we gave it. This causes an error when fed into
            // the interpolator because we do not have extrapolation enabled, so we set the last
            // time exactly here.
            *t_rs.last_mut().unwrap() = t_end;

            t_rs
        };

        // Resample.
        let xt_rs = xt.evaluate_array(&t_resamples.view()).unwrap();
        let yt_rs = yt.evaluate_array(&t_resamples.view()).unwrap();
        let pt_rs = pt.evaluate_array(&t_resamples.view()).unwrap();

        // Filter the resampled data. We also take derivatives here so we can calculate curvature.
        let (
            xt_rs_filt,
            yt_rs_filt,
            pt_rs_filt,
            x1t_rs_filt,
            y1t_rs_filt,
            x2t_rs_filt,
            y2t_rs_filt,
        ) = {
            use ndarray_ndimage::gaussian_filter1d as f;

            const XY_FILT_SIGMA: f64 = 155.5;
            const XY_FILT_BM: BorderMode<f64> = BorderMode::Nearest;
            const XY_FILT_TRUNC: usize = 10;

            const P_FILT_SIGMA: f64 = 7.9;
            const P_FILT_BM: BorderMode<f64> = BorderMode::Nearest;
            const P_FILT_TRUNC: usize = 6;

            // d(index)/dt. `f` gives us derivatives with respect to index, so we use the chain
            // rule to get derivatives with respect to time.
            let di_dt = t_resamples[1] - t_resamples[0];

            (
                f(&xt_rs, XY_FILT_SIGMA, Axis(0), 0, XY_FILT_BM, XY_FILT_TRUNC),
                f(&yt_rs, XY_FILT_SIGMA, Axis(0), 0, XY_FILT_BM, XY_FILT_TRUNC),
                f(&pt_rs, P_FILT_SIGMA, Axis(0), 0, P_FILT_BM, P_FILT_TRUNC),
                // For some reason, the sign is flipped for the first derivatives. The Python
                // implementation does not do this.
                -f(&xt_rs, XY_FILT_SIGMA, Axis(0), 1, XY_FILT_BM, XY_FILT_TRUNC) / di_dt,
                -f(&yt_rs, XY_FILT_SIGMA, Axis(0), 1, XY_FILT_BM, XY_FILT_TRUNC) / di_dt,
                // Second derivatives are fine.
                f(&xt_rs, XY_FILT_SIGMA, Axis(0), 2, XY_FILT_BM, XY_FILT_TRUNC) / (di_dt * di_dt),
                f(&yt_rs, XY_FILT_SIGMA, Axis(0), 2, XY_FILT_BM, XY_FILT_TRUNC) / (di_dt * di_dt),
            )
        };

        // The data we were given is guaranteed not to have had any adjacent events with the same
        // position. Though we have now interpolated, resampled and filtered the position data,
        // it should still be true that no two adjacent position samples will be the same.
        // Therefore, we can obtain a strictly increasing function s(t) giving the arc length
        // at time t by summing the distances between sample positions.
        let st_rs_filt: Array1<f64> = std::iter::once(0.0)
            .chain(
                xt_rs_filt
                    .iter()
                    .zip(&yt_rs_filt)
                    .tuple_windows()
                    .map(|((&xa, &ya), (&xb, &yb))| {
                        let dist = (xa - xb).hypot(ya - yb);

                        assert!(
                            dist > 0.0,
                            "arc length is not strictly increasing with time"
                        );

                        dist
                    })
                    .scan(0.0, |arc_length, dist| {
                        *arc_length += dist;
                        Some(*arc_length)
                    }),
            )
            .collect_vec()
            .into();

        let curvature = {
            // There is no vectorised `hypot` or FMA, so we need to work element-wise to some
            // degree. We might as well perform the whole calculation in that one loop.
            let mut curvature = x2t_rs_filt.to_vec();

            ndarray::azip!((
                k in &mut curvature,
                &x1 in &x1t_rs_filt,
                &y1 in &y1t_rs_filt,
                &y2 in &y2t_rs_filt,
            ) {
                let x2 = *k;
                *k = Self::calc_curvature(x1, y1, x2, y2);
            });

            curvature
        };

        // Since the arc length and x/y/p/curvature data all correspond to the time data, we can
        // treat the former as the independent variable for the latter four.
        let x = PchipInterpolator::new(&st_rs_filt.view(), &xt_rs_filt.view(), false).unwrap();
        let y = PchipInterpolator::new(&st_rs_filt.view(), &yt_rs_filt.view(), false).unwrap();

        ContinuousStroke {
            length: *st_rs_filt.last().unwrap(),
            tx: x.derivative(1).unwrap(),
            ty: y.derivative(1).unwrap(),
            x,
            y,
            p: Linterpolator {
                independent: st_rs_filt.to_vec().into(),
                // fixme: Could we avoid the allocation here and convert directly?
                dependent: pt_rs_filt.to_vec().into(),
            },
            curvature: Linterpolator {
                // fixme: Here, too
                independent: st_rs_filt.to_vec().into(),
                dependent: curvature.into(),
            },
        }
    }

    pub fn position(&self, s: f64) -> (f64, f64) {
        let x = self.x.evaluate(s).expect("arc length out of range");
        (x, self.y.evaluate(s).unwrap())
    }

    pub fn unit_tangent(&self, s: f64) -> (f64, f64) {
        let tx = self.tx.evaluate(s).expect("arc length out of range");
        (tx, self.ty.evaluate(s).unwrap())
    }

    pub fn pressure(&self, s: f64) -> f64 {
        self.p.evaluate(s).expect("arc length out of range")
    }

    /// Returns an iterator yielding the arc lengths at the vertices (curvature extrema) of the
    /// stroke, in ascending order. The start and end of the stroke do not count as vertices.
    fn vertices(&self) -> impl Iterator<Item = f64> {
        self.curvature.extrema().map(|(s, _)| s)
    }

    /// Returns an iterator yielding the arc lengths at the inflection points of the stroke, in
    /// ascending order.
    fn inflection_points(&self) -> impl Iterator<Item = f64> {
        const TOLERANCE: f64 = f64::EPSILON;

        let s_end = self.length;

        self.curvature
            .roots(TOLERANCE)
            .filter_map(move |root| match root {
                Root::Single(s) => Some(s),

                // For an interval of zero curvature that is not the entire stroke, we take the
                // midpoint.
                Root::Interval(s0, s1) if 0.0 < s0 || s1 < s_end => Some(0.5 * (s0 + s1)),

                // We don't care if there is zero curvature along the entire stroke, because that
                // just means the stroke is perfectly straight. Logically, there are no inflection
                // points.
                Root::Interval(_, _) => None,
            })
    }

    /// Returns the features of the stroke, in ascending order of arc length.
    pub fn features(&self) -> Vec<Feature> {
        // There are usually more vertices than inflection points, so collect those first (along
        // with the start and end, which are always present as features).
        let mut features: Vec<Feature> = std::iter::once(Feature::Start)
            .chain(self.vertices().map(Feature::Vertex))
            .chain(std::iter::once(Feature::End(self.length)))
            .collect();

        // Insert the inflection points into `features` while keeping it in ascending order.
        for ip_s in self.inflection_points() {
            let i = features.partition_point(|s| s.arc_length() <= ip_s);

            if features.get(i - 1).map(|f| f.arc_length()) == Some(ip_s) {
                // Already counted, presumably as a different type of feature.
                continue;
            }

            // Don't allow inflection points to be placed before the `Start` feature or after
            // the `End`.
            assert!(
                (1..features.len()).contains(&i),
                "inflection point (s = {ip_s}) too big (end: s = {}) or small (start: s = 0)",
                features.last().unwrap().arc_length()
            );

            features.insert(i, Feature::Inflection(ip_s));
        }

        features
    }

    pub fn sample_points<'me>(
        &'me self,
        target_segment_angle: f64,
    ) -> impl Iterator<Item = f64> + 'me {
        self.features()
            .into_iter()
            // Add a dummy element so that we can yield the final feature from `tuple_windows`.
            .map(Some)
            .chain([None])
            .tuple_windows()
            .flat_map(move |(f0, f1)| {
                let s0 = f0.unwrap().arc_length();

                let Some(s1) = f1.map(|f1| f1.arc_length()) else {
                    return Either::Left(std::iter::once(s0));
                };

                Either::Right(std::iter::once(s0).chain({
                    let lcs = LinearCurvatureSegment::new(
                        s0,
                        s1,
                        self.curvature.evaluate(s0).unwrap().abs(),
                        self.curvature.evaluate(s1).unwrap().abs(),
                        self.curvature.integrate(s0, s1).unwrap().abs(),
                    );

                    lcs.sample_points_strictly_inside(target_segment_angle)
                }))
            })
    }

    /// Returns the change in the angle of the tangent to the stroke from arc length `s0` to arc
    /// length `s1`, or `None` if either arc length is not in range. The returned angle may have
    /// magnitude greater than 2π.
    fn tangent_angle_change(&self, s0: f64, s1: f64) -> Option<f64> {
        self.curvature.integrate(s0, s1)
    }
}

/// Time, position and pressure data for a stroke, split into four vectors. It is guaranteed that
/// the four vectors have the same length, that this length is at least 2, that the data is
/// stored in chronological order, and that no two consecutive events have the same position.
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
    /// the stroke data is cleaned by merging consecutive events with the same timestamp or
    /// position and is returned as a `SplitStroke` with the data in the same order as it was
    /// obtained from the events.
    pub fn from_events<'a>(
        events: impl IntoIterator<Item = &'a sdocx::page::object::stroke::Event> + 'a,
    ) -> StrokeOrDot {
        const COORD_DUPE_TOLERANCE: f64 = f64::EPSILON;

        // Combine consecutive events that occur at the same time or position. Since the events are
        // in chronological order, the effect is that for every time value we have, there is
        // exactly one event that occurs at that time.
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
            // Combine consecutive elements with the same position by taking the mean of their
            // timestamps and the maximum pressure. Taking the maximum pressure ensures that marks
            // that are likely to be wider (higher pressure) effectively obscure ones that are
            // likely to be narrower (lower pressure).
            .coalesce(|a @ (t0, x0, y0, p0), b @ (t1, x1, y1, p1)| {
                if (x0 - x1).abs() < COORD_DUPE_TOLERANCE && (y0 - y1).abs() < COORD_DUPE_TOLERANCE
                {
                    Ok((0.5 * (t0 + t1), x0, y0, p0.max(p1)))
                } else {
                    Err((a, b))
                }
            })
            // Combine consecutive events with the same timestamp. Since the events are in
            // chronological order, the effect of this is to ensure that there is at most one event
            // for any given timestamp.
            .coalesce(|a @ (t0, x0, y0, p0), b @ (t1, x1, y1, p1)| {
                if t0 == t1 {
                    // There are lots of exactly duplicate events in the data for some reason, so
                    // the two events are likely to be identical. There are sometimes events with
                    // the same timestamp which are not identical otherwise, however, so we take
                    // the mean of the other fields.
                    Ok((t0, 0.5 * (x0 + x1), 0.5 * (y0 + y1), 0.5 * (p0 + p1)))
                } else {
                    Err((a, b))
                }
            });

        match deduped.at_most_one() {
            // Single event => dot
            Ok(Some((_time, x, y, pressure))) => StrokeOrDot::Dot { x, y, pressure },

            // Many events => stroke
            Err(deduped) => {
                let (time, x, y, pressure) = itertools::multiunzip(deduped);

                StrokeOrDot::Stroke(SplitStroke {
                    time,
                    x,
                    y,
                    pressure,
                })
            }

            // No events => :(
            Ok(None) => panic!("there must be at least one event"),
        }
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
        abs_curv_start: f64,
        abs_curv_end: f64,
        accumulated_abs_angular_change: f64,
    ) -> LinearCurvatureSegment {
        let length = s_end - s_start;

        LinearCurvatureSegment {
            s_start,

            length,

            abs_curv_start,
            abs_curv_end,

            accumulated_abs_angular_change,
        }
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
