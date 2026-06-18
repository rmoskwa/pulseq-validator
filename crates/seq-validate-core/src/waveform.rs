//! Gradient/RF waveform math over the IR's sparse [`Shape`]: one home for the
//! interpolation convention (held constant before the first sample and after the
//! last, piecewise-linear between) and the trapezoidal integral that follows from
//! it. Every check that integrates a shape or samples a gradient calls in here, so
//! a fix to the convention reaches all of them.
//!
//! The core ([`partial_integral`]) takes the shape's `(time, amp, duration)`
//! directly, not a layer-specific struct, so the *interp* gradients (already
//! FOV-scaled and rotated) and the *model* gradients (logical frame, raster ticks)
//! share it through thin adapters: the interp helpers below and [`model_area`].

use std::ops::{Add, Mul, Sub};

use pulseq_parse::model;

use crate::ir::{Gradient, Shape};

/// Trapezoidal integral of a sparse shape — samples `amp` at breakpoints `time`,
/// held constant from `0` to the first sample and from the last to `duration` —
/// accumulated from `0` to `upto` (clamped to the active extent). Exact at every
/// breakpoint. For a uniform centred shape this reproduces the midpoint rule; for
/// an explicit boundary grid (`[0, dur]`) the plain trapezoid — one rule for both.
/// Generic over the sample type so it serves f64 gradients and `Complex64` RF.
#[allow(clippy::indexing_slicing)] // Shape invariants: time/amp non-empty and equal length
fn partial_integral<T>(time: &[f64], amp: &[T], duration: f64, upto: f64) -> T
where
    T: Copy + Add<Output = T> + Sub<Output = T> + Mul<f64, Output = T>,
{
    let upto = upto.clamp(0.0, duration);
    let (t, a) = (time, amp);
    let n = t.len();
    // Leading flat segment [0, t[0]] held at a[0] (also seeds the zero value).
    let mut acc = a[0] * t[0].min(upto);
    if upto <= t[0] {
        return acc;
    }
    // Piecewise-linear segments.
    for i in 1..n {
        let (t0, t1) = (t[i - 1], t[i]);
        let seg = t1 - t0;
        if seg <= 0.0 {
            continue;
        }
        let end = t1.min(upto);
        let frac = (end - t0) / seg;
        let a_end = a[i - 1] + (a[i] - a[i - 1]) * frac;
        acc = acc + (a[i - 1] + a_end) * (0.5 * (end - t0));
        if upto <= t1 {
            return acc;
        }
    }
    // Trailing flat segment [t[last], duration] held at a[last].
    acc + a[n - 1] * (upto - t[n - 1])
}

/// Full integral of a shape over its active extent `[0, duration]`. The RF flip
/// integral and any whole-shape moment go through here.
pub fn integrate<T>(shape: &Shape<T>) -> T
where
    T: Copy + Add<Output = T> + Sub<Output = T> + Mul<f64, Output = T>,
{
    partial_integral(&shape.time, &shape.amp, shape.duration, shape.duration)
}

/// Full gradient moment `∫ G·dt` [1/m] over the gradient's active extent.
pub fn area(g: &Gradient) -> f64 {
    g.amp
        * partial_integral(
            &g.shape.time,
            &g.shape.amp,
            g.shape.duration,
            g.shape.duration,
        )
}

/// Running gradient moment `∫₀^t G·dt` [1/m] up to block-relative time `t` [s].
pub fn partial_area(g: &Gradient, t: f64) -> f64 {
    let local = t - g.delay;
    if local <= 0.0 {
        0.0
    } else {
        g.amp * partial_integral(&g.shape.time, &g.shape.amp, g.shape.duration, local)
    }
}

/// Instantaneous gradient amplitude [Hz/m] at block-relative time `t` [s]; `0`
/// outside the gradient's active window.
pub fn value_at(g: &Gradient, t: f64) -> f64 {
    let local = t - g.delay;
    if local < 0.0 || local > g.shape.duration {
        0.0
    } else {
        g.amp * g.shape.interpolate(local)
    }
}

/// Instantaneous slew [Hz/m/s] of `g` at block-relative time `t`: the slope of the
/// shape segment containing `t`, or `0` outside the active window / in a held-flat
/// region. Sampling at a segment's interior (a sub-interval midpoint) avoids the
/// gradient's start/end edge, where the held-flat convention is not a real ramp.
#[allow(clippy::indexing_slicing)] // Shape invariant: time/amp non-empty and equal length
pub fn slew_at(g: &Gradient, t: f64) -> f64 {
    let local = t - g.delay;
    let (time, amp) = (&g.shape.time, &g.shape.amp);
    if local < time[0] || local > g.shape.duration {
        return 0.0;
    }
    for i in 1..time.len() {
        if local <= time[i] {
            let seg = time[i] - time[i - 1];
            return if seg > 0.0 {
                g.amp * (amp[i] - amp[i - 1]) / seg
            } else {
                0.0
            };
        }
    }
    0.0 // trailing held-flat region
}

/// One linear segment of a gradient shape: its constant slew [Hz/m/s] and
/// duration [s]. Flat segments have zero slew.
pub struct Ramp {
    pub slew: f64,
    pub dur: f64,
}

/// The linear segments of a gradient's (piecewise-linear) shape, in physical
/// Hz/m/s. A trapezoid yields rise / flat(0) / fall; a sampled free gradient one
/// per inter-sample interval. The leading/trailing held-flat regions carry no
/// slew and are omitted.
#[allow(clippy::indexing_slicing)] // Shape invariant: time/amp non-empty and equal length
pub fn ramps(g: &Gradient) -> Vec<Ramp> {
    let (t, a) = (&g.shape.time, &g.shape.amp);
    let mut out = Vec::with_capacity(t.len());
    for i in 1..t.len() {
        let dur = t[i] - t[i - 1];
        if dur > 0.0 {
            out.push(Ramp {
                slew: g.amp * (a[i] - a[i - 1]) / dur,
                dur,
            });
        }
    }
    out
}

/// Zeroth moment (`∫G·dt`, `[Hz/m·s]`) of one canonical `model` gradient — the
/// logical-frame area, before any block rotation. A trapezoid integrates in
/// closed form; a free gradient integrates its sampled shape (in raster ticks,
/// scaled to seconds) through the shared [`partial_integral`].
pub fn model_area(g: &model::Gradient, grad_raster: f64) -> f64 {
    match g {
        model::Gradient::Trap {
            amp,
            rise,
            flat,
            fall,
            ..
        } => amp * (flat + 0.5 * (rise + fall)),
        model::Gradient::Free { amp, shape, .. } => {
            let ticks = partial_integral(
                &shape.time,
                &shape.amp,
                f64::from(shape.duration),
                f64::from(shape.duration),
            );
            amp * ticks * grad_raster
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_integral_of_trapezoid() {
        // Unit trapezoid: rise 1, flat 2, fall 1 (area = 3).
        let (time, amp, dur) = ([0.0_f64, 1.0, 3.0, 4.0], [0.0_f64, 1.0, 1.0, 0.0], 4.0_f64);
        assert!((partial_integral(&time, &amp, dur, 0.0)).abs() < 1e-12);
        assert!((partial_integral(&time, &amp, dur, 1.0) - 0.5).abs() < 1e-12); // rise only
        assert!((partial_integral(&time, &amp, dur, 2.0) - 1.5).abs() < 1e-12); // rise + 1 flat
        assert!((partial_integral(&time, &amp, dur, 4.0) - 3.0).abs() < 1e-12); // full
        assert!((partial_integral(&time, &amp, dur, 9.9) - 3.0).abs() < 1e-12); // clamps past end
    }
}
