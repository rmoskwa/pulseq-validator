//! Hardware/safety checks against a scanner [`Profile`].
//!
//! These are the first checks that need a scanner model: the `.seq` file does not
//! carry its target's amplifier, RF and PNS limits, so a [`Profile`] supplies
//! them (resolved in `crate::profile`). The interpreted gradients are physical —
//! already FOV-scaled and *rotated* — so peak amplitude / slew / PNS are measured
//! on what the coil actually plays, oblique blades and all.
//!
//! Units: the IR stores gradients in Hz/m and RF in Hz; the profile speaks the
//! scanner's units (mT/m, T/m/s, µT). We convert with the ¹H gyromagnetic ratio
//! [`DEFAULT_LARMOR_HZ`] (γ̄ = 42.577 MHz/T), which is field-strength-independent
//! — a gradient's Hz/m and a pulse's Hz are defined through ¹H γ regardless of B0.
//!
//! Checks emitted (all under `hardware.*`):
//! - `profile` — which scanner was assumed, or a `skip` when none was selected
//!   (the clear, non-silent "no profile" outcome).
//! - `gradient_amplitude` / `slew_rate` — the per-axis peak is the hard limit
//!   (`fail`); the combined vector magnitude is reported alongside it as an
//!   informational `measured` value. It is *not* checked against the per-axis
//!   limit: per-axis-limited amplifiers (the dominant architecture) may
//!   legitimately drive every axis to its own max at once, so
//!   "combined ≤ per-axis limit" would false-positive on normal
//!   sequences. A future profile with a distinct combined limit can add that check.
//! - `adc_dwell` — the ADC dwell must divide the scanner's ADC raster.
//! - `rf_b1` — peak B1 within `B1max`.
//! - `dead_time` — RF dead/ring-down and ADC dead-time (the dimension
//!   `integrity.dead_time` defers here once a profile exists).
//! - `pns` — a basic, **approximate** PNS estimate (`warn` only; see [`pns_stats`]).
//!
//! With no profile, only `hardware.profile` is emitted (a `skip`), keeping the
//! common file-only report uncluttered while still naming the missing input.

use serde_json::json;

use crate::checks::{Check, CheckCtx};
use crate::ir::{Block, DEFAULT_LARMOR_HZ, Gradient, Sequence};
use crate::profile::{Pns, Profile};
use crate::result::{Category, CheckResult};
use crate::waveform;

/// The hardware/safety check, wired into [`crate::checks::registry`].
pub(crate) fn checks() -> Vec<Box<dyn Check>> {
    vec![Box::new(Hardware)]
}

/// Relative tolerance for a limit comparison: a value counts as exceeding only
/// when it clears the limit by more than this, so a value sitting exactly on the
/// limit (floating-point noise aside) is not a false `fail`.
const REL_TOL: f64 = 1e-6;
/// Timing slack for dead-time comparisons [s].
const TIMING_TOL_S: f64 = 1e-7;
/// Quotient tolerance for the ADC-dwell-divides-raster test.
const RASTER_TOL: f64 = 1e-6;

const AXES: [&str; 3] = ["gx", "gy", "gz"];

/// `true` when `value` exceeds `limit` by more than [`REL_TOL`].
fn over(value: f64, limit: f64) -> bool {
    value > limit * (1.0 + REL_TOL)
}

// --- unit conversions (¹H γ̄, field-independent) ------------------------------

/// Gradient amplitude Hz/m → mT/m.
fn mt_per_m(hz_per_m: f64) -> f64 {
    hz_per_m / DEFAULT_LARMOR_HZ * 1e3
}
/// Slew rate Hz/m/s → T/m/s.
fn t_per_m_per_s(hz_per_m_per_s: f64) -> f64 {
    hz_per_m_per_s / DEFAULT_LARMOR_HZ
}
/// B1 amplitude Hz → µT.
fn ut(hz: f64) -> f64 {
    hz / DEFAULT_LARMOR_HZ * 1e6
}

// --- gradient waveform primitives --------------------------------------------

/// Peak gradient amplitude [Hz/m] over a gradient's active extent.
fn grad_peak_amp(g: &Gradient) -> f64 {
    g.amp.abs() * g.shape.amp.iter().fold(0.0_f64, |m, &x| m.max(x.abs()))
}

/// Block-relative breakpoint times where any axis changes slope: every shape
/// sample plus each gradient's start and end. Sorted; the combined-magnitude
/// sweep evaluates between consecutive entries.
fn block_breakpoints(b: &Block) -> Vec<f64> {
    let mut ts = vec![0.0_f64];
    for g in [&b.gx, &b.gy, &b.gz].into_iter().flatten() {
        ts.push(g.delay);
        ts.push(g.delay + g.shape.duration);
        for &t in &g.shape.time {
            ts.push(g.delay + t);
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
    ts
}

// --- whole-sequence gradient statistics --------------------------------------

/// Peak gradient amplitude and slew across the sequence: per axis (with the block
/// each peak is in, for the `fail` message) and the combined vector magnitude
/// (informational only, so no block tracked). Amplitudes in Hz/m, slews in Hz/m/s.
#[derive(Default)]
struct GradStats {
    grad: [f64; 3],
    grad_block: [usize; 3],
    slew: [f64; 3],
    slew_block: [usize; 3],
    comb_grad: f64,
    comb_slew: f64,
}

#[allow(clippy::indexing_slicing)] // axis arrays are length 3, indexed by a ∈ 0..3
fn analyze_gradients(seq: &Sequence) -> GradStats {
    let mut st = GradStats::default();
    for (bi, b) in seq.blocks.iter().enumerate() {
        let axes = [&b.gx, &b.gy, &b.gz];

        // Per-axis peaks: exact from each gradient's own shape.
        for (a, g) in axes.iter().enumerate() {
            let Some(g) = g else { continue };
            let pk = grad_peak_amp(g);
            if pk > st.grad[a] {
                st.grad[a] = pk;
                st.grad_block[a] = bi;
            }
            for r in waveform::ramps(g) {
                let s = r.slew.abs();
                if s > st.slew[a] {
                    st.slew[a] = s;
                    st.slew_block[a] = bi;
                }
            }
        }

        // Combined (vector) peaks: evaluate the three axes together on the block's
        // shared breakpoint grid (amplitude at each breakpoint, slew at each
        // sub-interval midpoint where every axis is within a linear segment).
        if axes.iter().all(|g| g.is_none()) {
            continue;
        }
        let ts = block_breakpoints(b);
        for &t in &ts {
            let mag = mag3(
                grad_value_opt(&b.gx, t),
                grad_value_opt(&b.gy, t),
                grad_value_opt(&b.gz, t),
            );
            if mag > st.comb_grad {
                st.comb_grad = mag;
            }
        }
        for w in ts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 <= 0.0 {
                continue;
            }
            let tm = 0.5 * (t0 + t1);
            let mag = mag3(
                slew_value_opt(&b.gx, tm),
                slew_value_opt(&b.gy, tm),
                slew_value_opt(&b.gz, tm),
            );
            if mag > st.comb_slew {
                st.comb_slew = mag;
            }
        }
    }
    st
}

fn grad_value_opt(g: &Option<Gradient>, t: f64) -> f64 {
    g.as_ref().map_or(0.0, |g| waveform::value_at(g, t))
}
fn slew_value_opt(g: &Option<Gradient>, t: f64) -> f64 {
    g.as_ref().map_or(0.0, |g| waveform::slew_at(g, t))
}
fn mag3(x: f64, y: f64, z: f64) -> f64 {
    (x * x + y * y + z * z).sqrt()
}

/// Index of the largest entry of a length-3 array. A NaN entry is never selected
/// (`x > best` is false for a NaN `x`), so a NaN on one axis cannot mask a real
/// larger peak on another and leave the "worst axis" pointing at the wrong place.
/// Peaks are finite in practice; this just keeps the choice honest regardless.
fn argmax3(v: [f64; 3]) -> usize {
    let mut best = 0;
    let mut best_val = f64::NEG_INFINITY;
    for (a, &x) in v.iter().enumerate() {
        if x > best_val {
            best_val = x;
            best = a;
        }
    }
    best
}

// --- PNS (approximate) -------------------------------------------------------

/// Per-axis peak PNS [% of stimulation threshold] and the block each peak is in,
/// from the IEC 60601-2-33:2022 nerve-impulse-response model (Eq. AA.21).
///
/// **This is the single-ramp closed form, not the full convolution.** For one
/// constant-slew ramp of slew `SR` and duration `τ`, the model's response peaks at
/// `P = 100·(SR/Smin)·τ/(c+τ)` with `Smin = rheobase/α`, `c = chronaxie`. We take
/// the worst ramp per axis. This is fast and bounded (the full convolution would
/// be O(samples·kernel) over a whole sequence), and is **deliberately
/// approximate**: it ignores temporal superposition of nearby ramps, so it is a
/// proxy, not a certifying calculation — hence the check only ever `warn`s.
fn pns_stats(seq: &Sequence, pns: &Pns) -> ([f64; 3], [usize; 3]) {
    let smin = pns.rheobase / pns.alpha;
    let c = pns.chronaxie_s;
    let mut peak = [0.0_f64; 3];
    let mut block = [0_usize; 3];
    for (bi, b) in seq.blocks.iter().enumerate() {
        for (a, g) in [&b.gx, &b.gy, &b.gz].iter().enumerate() {
            let Some(g) = g else { continue };
            for r in waveform::ramps(g) {
                let sr = t_per_m_per_s(r.slew).abs();
                let pct = 100.0 * sr / smin * r.dur / (c + r.dur);
                if let Some(p) = peak.get_mut(a)
                    && pct > *p
                {
                    *p = pct;
                    if let Some(blk) = block.get_mut(a) {
                        *blk = bi;
                    }
                }
            }
        }
    }
    (peak, block)
}

// --- the check ---------------------------------------------------------------

/// Runs every hardware/safety limit against the resolved profile, computing the
/// gradient waveform statistics once and emitting one `hardware.*` result per
/// limit. With no profile, emits only the `hardware.profile` skip.
struct Hardware;

impl Check for Hardware {
    fn category(&self) -> Category {
        Category::Hardware
    }
    fn name(&self) -> &'static str {
        // Unused: this check emits explicit per-limit ids, not `<category>.<name>`.
        "hardware"
    }

    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let Some(profile) = ctx.profile else {
            return vec![no_profile_result()];
        };
        let seq = ctx.seq;
        let st = analyze_gradients(seq);
        vec![
            profile_result(profile),
            gradient_amplitude_result(&st, profile),
            slew_rate_result(&st, profile),
            adc_dwell_result(seq, profile),
            rf_b1_result(seq, profile),
            dead_time_result(seq, profile),
            pns_result(seq, profile),
        ]
    }
}

/// `hardware.profile` when none was resolved: a clear, non-silent `skip` naming
/// the missing input and how to supply it.
fn no_profile_result() -> CheckResult {
    CheckResult::skip(
        "hardware.profile",
        "no scanner profile selected and none embedded in [DEFINITIONS]; \
         hardware/safety checks skipped. Re-run with --profile <name> \
         (run `seq-validate --list-profiles` to list the available profiles).",
    )
}

/// `hardware.profile` when one was resolved: records which scanner — and its
/// cited source — the report assumed.
fn profile_result(p: &Profile) -> CheckResult {
    let b1 = if p.max_b1_ut.is_finite() {
        format!("{:.0}", p.max_b1_ut)
    } else {
        "—".to_string()
    };
    CheckResult::pass(
        "hardware.profile",
        format!(
            "scanner profile '{}' ({}) — maxGrad {:.0} mT/m, maxSlew {:.0} T/m/s, \
             B1max {b1} µT. Source: {}",
            p.name, p.vendor, p.max_grad_mt_m, p.max_slew_t_m_s, p.source
        ),
    )
    .with_measured(json!({
        "name": p.name,
        "vendor": p.vendor,
        "source": p.source,
        "max_grad_mt_m": p.max_grad_mt_m,
        "max_slew_t_m_s": p.max_slew_t_m_s,
        "max_b1_ut": p.max_b1_ut.is_finite().then_some(p.max_b1_ut),
        "b0_t": p.b0_t,
    }))
}

/// `hardware.gradient_amplitude` — the per-axis peak is the hard limit; the
/// combined vector magnitude rides along as informational `measured` data, not a
/// pass/fail input (see the module doc on per-axis-limited amplifiers).
#[allow(clippy::indexing_slicing)] // length-3 axis arrays
fn gradient_amplitude_result(st: &GradStats, p: &Profile) -> CheckResult {
    let per = [
        mt_per_m(st.grad[0]),
        mt_per_m(st.grad[1]),
        mt_per_m(st.grad[2]),
    ];
    let comb = mt_per_m(st.comb_grad);
    let lim = p.max_grad_mt_m;
    let w = argmax3(per);
    let measured = json!({ "per_axis_mt_m": per, "combined_mt_m": comb, "limit_mt_m": lim });

    let result = if over(per[w], lim) {
        CheckResult::fail(
            "hardware.gradient_amplitude",
            format!(
                "{} peak gradient {:.1} mT/m exceeds maxGrad {lim:.1} mT/m (block {})",
                AXES[w], per[w], st.grad_block[w]
            ),
        )
    } else {
        CheckResult::pass(
            "hardware.gradient_amplitude",
            format!(
                "peak gradient within maxGrad {lim:.1} mT/m (per-axis max {:.1}, combined {comb:.1})",
                per[w]
            ),
        )
    };
    result.with_measured(measured)
}

/// `hardware.slew_rate` — the per-axis peak is the hard limit; combined is
/// informational `measured` data (see the module doc).
#[allow(clippy::indexing_slicing)] // length-3 axis arrays
fn slew_rate_result(st: &GradStats, p: &Profile) -> CheckResult {
    let per = [
        t_per_m_per_s(st.slew[0]),
        t_per_m_per_s(st.slew[1]),
        t_per_m_per_s(st.slew[2]),
    ];
    let comb = t_per_m_per_s(st.comb_slew);
    let lim = p.max_slew_t_m_s;
    let w = argmax3(per);
    let measured = json!({ "per_axis_t_m_s": per, "combined_t_m_s": comb, "limit_t_m_s": lim });

    let result = if over(per[w], lim) {
        CheckResult::fail(
            "hardware.slew_rate",
            format!(
                "{} peak slew {:.1} T/m/s exceeds maxSlew {lim:.1} T/m/s (block {})",
                AXES[w], per[w], st.slew_block[w]
            ),
        )
    } else {
        CheckResult::pass(
            "hardware.slew_rate",
            format!(
                "peak slew within maxSlew {lim:.1} T/m/s (per-axis max {:.1}, combined {comb:.1})",
                per[w]
            ),
        )
    };
    result.with_measured(measured)
}

/// `hardware.adc_dwell` — the ADC dwell must be an integer multiple of the
/// scanner's ADC raster (the hardware sampling clock). A file may declare a finer
/// `AdcRasterTime` of its own and still pass `integrity.raster_alignment`, yet be
/// unsamplable on the target; this is that scanner-specific gate.
fn adc_dwell_result(seq: &Sequence, p: &Profile) -> CheckResult {
    let raster = p.adc_raster_s;
    let mut n_adc = 0u64;
    let mut offending = 0u64;
    let mut first: Option<(usize, f64)> = None;
    for (bi, b) in seq.blocks.iter().enumerate() {
        let Some(adc) = &b.adc else { continue };
        n_adc += 1;
        if raster > 0.0 {
            let q = adc.dwell / raster;
            if (q - q.round()).abs() > RASTER_TOL {
                offending += 1;
                first.get_or_insert((bi, adc.dwell));
            }
        }
    }
    if n_adc == 0 {
        return CheckResult::skip("hardware.adc_dwell", "no ADC events to check");
    }
    if raster <= 0.0 {
        // No positive ADC raster to divide into — skip rather than pass vacuously
        // (a non-positive raster would otherwise report every dwell as "legal").
        return CheckResult::skip(
            "hardware.adc_dwell",
            "profile defines no positive ADC raster; dwell divisibility not checked",
        );
    }
    let measured = json!({ "adc_raster_s": raster, "adc_events": n_adc, "offending": offending });
    match first {
        Some((bi, dwell)) => CheckResult::fail(
            "hardware.adc_dwell",
            format!(
                "ADC dwell {dwell:.4e} s (block {bi}) is not a multiple of the scanner ADC \
                 raster {raster:.4e} s; {offending} of {n_adc} ADC event(s) affected"
            ),
        ),
        None => CheckResult::pass(
            "hardware.adc_dwell",
            format!("all {n_adc} ADC dwell time(s) divide the scanner ADC raster {raster:.4e} s"),
        ),
    }
    .with_measured(measured)
}

/// `hardware.rf_b1` — peak B1 within `B1max`. `skip`s when the profile defines no
/// B1 limit or the sequence has no RF.
fn rf_b1_result(seq: &Sequence, p: &Profile) -> CheckResult {
    if !p.max_b1_ut.is_finite() {
        return CheckResult::skip("hardware.rf_b1", "profile defines no peak-B1 limit");
    }
    let mut n_rf = 0u64;
    let mut peak_hz = 0.0_f64;
    let mut peak_block = 0usize;
    let mut max_dur = 0.0_f64;
    for (bi, b) in seq.blocks.iter().enumerate() {
        let Some(rf) = &b.rf else { continue };
        n_rf += 1;
        let pk = rf.amp.abs() * rf.shape.amp.iter().fold(0.0_f64, |m, c| m.max(c.norm()));
        if pk > peak_hz {
            peak_hz = pk;
            peak_block = bi;
        }
        max_dur = max_dur.max(rf.shape.duration);
    }
    if n_rf == 0 {
        return CheckResult::skip("hardware.rf_b1", "no RF events to check");
    }
    let peak_ut = ut(peak_hz);
    let lim = p.max_b1_ut;
    let measured = json!({ "peak_b1_ut": peak_ut, "limit_ut": lim, "longest_rf_s": max_dur });
    let result = if over(peak_ut, lim) {
        CheckResult::fail(
            "hardware.rf_b1",
            format!("peak B1 {peak_ut:.1} µT (block {peak_block}) exceeds B1max {lim:.1} µT"),
        )
    } else {
        CheckResult::pass(
            "hardware.rf_b1",
            format!(
                "peak B1 {peak_ut:.1} µT within B1max {lim:.1} µT (longest RF {:.3} ms)",
                max_dur * 1e3
            ),
        )
    };
    result.with_measured(measured)
}

/// `hardware.dead_time` — RF dead time / ring-down and ADC dead-time against the
/// profile. This is the check `integrity.dead_time` defers here once a profile
/// exists.
fn dead_time_result(seq: &Sequence, p: &Profile) -> CheckResult {
    let mut issues: Vec<String> = Vec::new();
    let mut violations = 0u64;
    let note = |cond: bool, msg: String, issues: &mut Vec<String>, n: &mut u64| {
        if cond {
            *n += 1;
            if issues.len() < 3 {
                issues.push(msg);
            }
        }
    };
    for (bi, b) in seq.blocks.iter().enumerate() {
        if let Some(rf) = &b.rf {
            note(
                rf.delay + TIMING_TOL_S < p.rf_dead_s,
                format!(
                    "block {bi}: RF starts at {:.3e} s < rfDeadTime {:.3e} s",
                    rf.delay, p.rf_dead_s
                ),
                &mut issues,
                &mut violations,
            );
            let tail = b.duration - (rf.delay + rf.shape.duration);
            note(
                tail + TIMING_TOL_S < p.rf_ringdown_s,
                format!(
                    "block {bi}: {tail:.3e} s after RF < rfRingdownTime {:.3e} s",
                    p.rf_ringdown_s
                ),
                &mut issues,
                &mut violations,
            );
        }
        if let Some(adc) = &b.adc {
            note(
                adc.delay + TIMING_TOL_S < p.adc_dead_s,
                format!(
                    "block {bi}: ADC starts at {:.3e} s < adcDeadTime {:.3e} s",
                    adc.delay, p.adc_dead_s
                ),
                &mut issues,
                &mut violations,
            );
        }
    }
    let measured = json!({ "violations": violations });
    if violations == 0 {
        CheckResult::pass(
            "hardware.dead_time",
            format!(
                "RF dead/ring-down and ADC dead-time satisfied (rfDead {:.0} µs, \
                 rfRingdown {:.0} µs, adcDead {:.0} µs)",
                p.rf_dead_s * 1e6,
                p.rf_ringdown_s * 1e6,
                p.adc_dead_s * 1e6
            ),
        )
    } else {
        CheckResult::fail(
            "hardware.dead_time",
            format!(
                "{violations} dead-time violation(s); e.g. {}",
                issues.join("; ")
            ),
        )
    }
    .with_measured(measured)
}

/// `hardware.pns` — a basic, **approximate** PNS estimate (see [`pns_stats`]).
/// Reported as `warn` past the IEC normal (80 %) / first-controlled (100 %) modes,
/// never `fail`: it is a conservative proxy, not a certifying calculation
/// ("reported as warn unless a profile defines a hard limit"). `skip`s
/// when the profile carries no PNS model.
#[allow(clippy::indexing_slicing)] // length-3 axis arrays
fn pns_result(seq: &Sequence, p: &Profile) -> CheckResult {
    let Some(pns) = &p.pns else {
        return CheckResult::skip(
            "hardware.pns",
            "profile defines no PNS model (chronaxie/rheobase/alpha)",
        );
    };
    let (per, block) = pns_stats(seq, pns);
    let comb = mag3(per[0], per[1], per[2]);
    let w = argmax3(per);
    let measured = json!({ "combined_pct": comb, "per_axis_pct": per });
    let head = format!(
        "PNS ≈ {comb:.0}% of threshold (per-ramp IEC 60601-2-33 model, approximate) — \
         worst axis {} {:.0}% at block {}",
        AXES[w], per[w], block[w]
    );
    let result = if comb >= 100.0 {
        CheckResult::warn(
            "hardware.pns",
            format!("{head}; exceeds first-controlled-mode (100%)"),
        )
    } else if comb >= 80.0 {
        CheckResult::warn("hardware.pns", format!("{head}; exceeds normal-mode (80%)"))
    } else {
        CheckResult::pass("hardware.pns", format!("{head}; within normal mode (<80%)"))
    };
    result.with_measured(measured)
}

#[cfg(test)]
mod tests {
    use super::argmax3;

    #[test]
    fn argmax3_picks_largest_and_nan_never_masks_a_real_peak() {
        assert_eq!(argmax3([1.0, 3.0, 2.0]), 1);
        assert_eq!(argmax3([0.0, 0.0, 0.0]), 0, "ties keep the first axis");
        // A NaN on one axis must not hide a genuine larger peak on another.
        assert_eq!(argmax3([f64::NAN, 5.0, 2.0]), 1);
        assert_eq!(argmax3([5.0, f64::NAN, 2.0]), 0);
    }
}
