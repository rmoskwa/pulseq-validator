//! Step 4 — derived imaging metrics (`docs/04-derived-metrics.md`).
//!
//! The product's spine: the headline "what is this sequence?" numbers, measured
//! from first principles off the interpreted IR.
//!
//! - **TR** — interval between successive excitations of the same slice.
//! - **TE (effective)** — k-space-centre ADC sample minus the excitation RF
//!   centre. The k-centre echo is the one whose net phase-encode area is
//!   smallest, so for an echo train this is the contrast-determining *effective*
//!   TE (the mid-train echo), not the first-echo time; a single-echo sequence
//!   has one echo, so it reduces to that echo.
//! - **Flip angle** — `360·|∫ RF envelope|` (small-tip), median over excitations.
//! - **n_slices** — count of distinct excitation RF frequency offsets.
//! - **Echo spacing** — median centre-to-centre echo interval (echo trains only).
//! - **Scan time** — total sequence duration.
//!
//! Geometry (FOV/matrix) is **not** here — it is the dual-witness Step 5.
//!
//! Units: times in SI **seconds** (matching the IR and Pulseq's own
//! `testReport`), flip in **degrees**, counts as integers. These are
//! *measurements*, not pass/fail assertions: in file-only mode each is a `pass`
//! carrying its `measured` value, or a `skip` when the sequence doesn't support
//! it (echo-spacing for a single-echo sequence; TE/echo-spacing when no readout
//! follows any excitation; everything but scan time when there is no excitation
//! RF). Step 7 reuses the same measurements for hard spec assertions.
//!
//! All six share one analysis pass — [`excitations`] then [`echo_trains`] — so
//! they live in a single [`DerivedMetrics`] check that emits one result per
//! metric, rather than recomputing the scaffold per metric.

use std::collections::BTreeMap;

use crate::checks::{Check, CheckCtx};
use crate::ir::{Adc, Rf, RfUse, Sequence, Shape};
use crate::result::{Category, CheckResult};

/// The derived-metrics check, wired into [`crate::checks::registry`].
pub(crate) fn checks() -> Vec<Box<dyn Check>> {
    vec![Box::new(DerivedMetrics)]
}

/// Minimum nominal flip [deg] for an RF event to count as a slice excitation.
/// Excludes a near-zero-flip stray/spoiling pulse the harness also ignores.
/// Shared with the trajectory gate (`crate::trajectory`) so both modules agree on
/// what an excitation is.
pub(crate) const MIN_EXCITATION_FLIP_DEG: f64 = 1.0;

/// Grouping precision for excitation frequency offsets: offsets are bucketed to
/// the nearest milli-hertz before counting distinct slices / pairing TR
/// intervals. Real slice offsets are kHz apart, repeats of one slice are
/// written bit-identically, so this only merges numerical dust.
const FREQ_BUCKET_HZ: f64 = 1e-3;

// ---------------------------------------------------------------------------
// Measurement primitives (the `seq_file.py` helpers, ported)
// ---------------------------------------------------------------------------

/// Effective-rotation time of an RF event relative to its block start [s]:
/// `delay + center` (Pulseq's `mr.calcRfCenter` convention; the file's `center`
/// column is measured from the waveform start, the delay shifts it onto the
/// block clock). Our parser targets 1.5.x, where `center` is always present.
/// Shared with the trajectory gate (`crate::trajectory`), which resets k-space at
/// each excitation's RF centre.
pub(crate) fn rf_center_s(rf: &Rf) -> f64 {
    rf.delay + rf.center
}

/// Time of the central ADC sample relative to its block start [s]: the
/// k-space-centre echo position for a symmetric full readout.
fn adc_center_s(adc: &Adc) -> f64 {
    adc.delay + adc.dwell * adc.num as f64 / 2.0
}

/// Whether an RF `use` tag marks the pulse as something *other* than the slice
/// excitation. An untagged (`Undefined`/`Other`) pulse is treated as a possible
/// excitation, gated by its flip in [`is_excitation`]. Shared with the trajectory
/// gate (`crate::trajectory`).
pub(crate) fn is_non_excitation_use(use_: RfUse) -> bool {
    matches!(
        use_,
        RfUse::Refocusing | RfUse::Inversion | RfUse::Saturation | RfUse::Preparation
    )
}

/// Nominal small-tip flip angle [deg] = `360·|∫ B1 dt|`, with `B1 = amp·shape`.
/// The integral is taken over the pulse's full active extent (see [`integrate`]),
/// so a sparse boundary-sampled shape (e.g. a two-point block pulse) and a dense
/// centred sinc both come out right. Shared with the trajectory gate
/// (`crate::trajectory`), which uses it to tell an excitation from a refocusing /
/// preparation pulse when deciding where to reset k-space.
pub(crate) fn flip_deg(rf: &Rf) -> f64 {
    rf.amp * integrate(&rf.shape).norm() * 360.0
}

/// Trapezoidal integral of a sparse [`Shape`] over its full active extent
/// `[0, duration]`. Samples are breakpoints of a piecewise-linear waveform held
/// constant from `0` to the first sample and from the last sample to `duration`
/// — exactly the IR's [`Shape::interpolate`] convention. For a uniform centred
/// shape this reproduces the midpoint rule (`Σ amp · raster`); for an explicit
/// boundary grid (`[0, dur]`) it reproduces the plain trapezoid — the two cases
/// the harness special-cased, unified.
#[allow(clippy::indexing_slicing)] // Shape invariants guarantee `time`/`amp` non-empty and equal length
fn integrate<T>(shape: &Shape<T>) -> T
where
    T: Copy + std::ops::Add<Output = T> + std::ops::Mul<f64, Output = T>,
{
    let (t, a) = (&shape.time, &shape.amp);
    let n = t.len();
    // Leading flat segment [0, t[0]] (also seeds the accumulator's zero value).
    let mut acc = a[0] * t[0];
    for i in 1..n {
        acc = acc + (a[i - 1] + a[i]) * (0.5 * (t[i] - t[i - 1]));
    }
    // Trailing flat segment [t[last], duration].
    acc + a[n - 1] * (shape.duration - t[n - 1])
}

/// numpy-style median: the mean of the two central values for an even count.
/// `None` for an empty input.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let mid = n / 2;
    Some(match v.get(mid) {
        Some(&hi) if n % 2 == 1 => hi,
        Some(&hi) => v.get(mid - 1).map_or(hi, |&lo| 0.5 * (lo + hi)),
        None => return None,
    })
}

// ---------------------------------------------------------------------------
// Analysis scaffold (the `find_excitations` / `find_echoes` passes, ported)
// ---------------------------------------------------------------------------

/// One excitation RF event on the whole-sequence clock.
struct Excitation<'a> {
    /// Index of the block carrying the pulse.
    block: usize,
    /// The interpreted RF event.
    rf: &'a Rf,
    /// Absolute effective-rotation time [s]: `block start + rf_center_s`.
    center_abs: f64,
    /// Nominal flip [deg], cached from the excitation test.
    flip: f64,
}

/// One acquired echo within an excitation's train.
struct Echo {
    /// Excitation-centre → ADC-centre time [s].
    te: f64,
    /// Net phase-encode (gy) area accumulated from the excitation up to this
    /// ADC; the minimum-`|area|` echo is the central ky line.
    pe_area: f64,
}

/// Every excitation, in file order: an RF whose `use` is not explicitly
/// non-excitation and whose nominal flip clears [`MIN_EXCITATION_FLIP_DEG`].
fn excitations(seq: &Sequence) -> Vec<Excitation<'_>> {
    let mut out = Vec::new();
    for (i, b) in seq.blocks.iter().enumerate() {
        let Some(rf) = b.rf.as_ref() else { continue };
        if is_non_excitation_use(rf.rf_use) {
            continue;
        }
        let flip = flip_deg(rf);
        if flip < MIN_EXCITATION_FLIP_DEG {
            continue;
        }
        let Some(&start) = seq.starts.get(i) else {
            continue;
        };
        out.push(Excitation {
            block: i,
            rf,
            center_abs: start + rf_center_s(rf),
            flip,
        });
    }
    out
}

/// Per excitation, every acquired echo in train order. An excitation's train
/// spans the blocks after it up to the next excitation; the phase-encode area is
/// accumulated across the whole train (each ky line is applied then rewound, so
/// the running area at an ADC is that echo's own ky line). Excitations with no
/// trailing ADC (dummy / steady-state shots) yield no train.
fn echo_trains(seq: &Sequence, exc: &[Excitation<'_>]) -> Vec<Vec<Echo>> {
    let mut trains = Vec::new();
    for (k, e) in exc.iter().enumerate() {
        let next = exc.get(k + 1).map_or(seq.blocks.len(), |n| n.block);
        let mut gy_acc = 0.0;
        let mut echoes = Vec::new();
        for j in (e.block + 1)..next {
            // Logical-frame gy area: the phase-encode line index within the
            // (possibly rotated) blade. Absent gy contributes 0.
            if let Some([_gx, gy, _gz]) = seq.logical_grad_areas.get(j) {
                gy_acc += gy;
            }
            let Some(b) = seq.blocks.get(j) else { continue };
            if let Some(adc) = b.adc.as_ref()
                && let Some(&start) = seq.starts.get(j)
            {
                echoes.push(Echo {
                    te: start + adc_center_s(adc) - e.center_abs,
                    pe_area: gy_acc,
                });
            }
        }
        if !echoes.is_empty() {
            trains.push(echoes);
        }
    }
    trains
}

/// Bucket key for a frequency offset [Hz], for grouping excitations by slice.
fn freq_bucket(freq_hz: f64) -> i64 {
    (freq_hz / FREQ_BUCKET_HZ).round() as i64
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

/// Measures the six non-geometry imaging metrics in one shared analysis pass and
/// emits one `metrics.*` result per metric.
struct DerivedMetrics;

impl DerivedMetrics {
    /// Stable ids of the metrics that need an excitation (everything but scan
    /// time), reported as `skip`s when no excitation RF exists.
    const EXCITATION_DEPENDENT: &'static [&'static str] = &[
        "metrics.tr",
        "metrics.te",
        "metrics.flip_angle",
        "metrics.n_slices",
        "metrics.echo_spacing",
    ];
}

impl Check for DerivedMetrics {
    fn category(&self) -> Category {
        Category::Metrics
    }
    fn name(&self) -> &'static str {
        // Unused: this check emits explicit per-metric ids below rather than the
        // default `<category>.<name>`. Kept distinct for the trait contract.
        "derived"
    }

    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let seq = ctx.seq;
        let mut out = vec![scan_time(seq)];

        let exc = excitations(seq);
        if exc.is_empty() {
            for id in Self::EXCITATION_DEPENDENT {
                out.push(CheckResult::skip(
                    *id,
                    "no excitation RF found; metric not measurable",
                ));
            }
            return out;
        }

        let (n_slices_result, groups) = n_slices_and_groups(&exc);
        out.push(n_slices_result);
        out.push(tr(seq, &groups));
        out.push(flip_angle(&exc));

        let trains = echo_trains(seq, &exc);
        let (te_result, esp_result) = te_and_echo_spacing(&trains);
        out.push(te_result);
        out.push(esp_result);
        out
    }
}

/// `metrics.scan_time` — always measurable from the total duration.
fn scan_time(seq: &Sequence) -> CheckResult {
    let d = seq.total_duration;
    CheckResult::pass(
        "metrics.scan_time",
        format!("scan time = {d:.6} s ({:.3} min)", d / 60.0),
    )
    .with_measured(d)
}

/// `metrics.n_slices` — distinct excitation frequency offsets. Returns the
/// result and the freq-bucketed groups (reused by [`tr`]).
fn n_slices_and_groups(exc: &[Excitation<'_>]) -> (CheckResult, BTreeMap<i64, Vec<f64>>) {
    let mut groups: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    for e in exc {
        groups
            .entry(freq_bucket(e.rf.freq))
            .or_default()
            .push(e.center_abs);
    }
    let n = groups.len();
    let result = CheckResult::pass(
        "metrics.n_slices",
        format!("{n} distinct excitation frequency offset(s) (slice/slab count)"),
    )
    .with_measured(n as u64);
    (result, groups)
}

/// `metrics.tr` — median interval between successive excitations of the same
/// slice (grouped by frequency offset). Falls back to the whole-sequence
/// duration when each slice is excited only once.
fn tr(seq: &Sequence, groups: &BTreeMap<i64, Vec<f64>>) -> CheckResult {
    let mut intervals = Vec::new();
    for centers in groups.values() {
        let mut c = centers.clone();
        c.sort_by(f64::total_cmp);
        for pair in c.windows(2) {
            if let [a, b] = pair {
                intervals.push(b - a);
            }
        }
    }

    match median(&intervals) {
        Some(t) => CheckResult::pass(
            "metrics.tr",
            format!(
                "TR = {t:.6} s ({:.2} ms), median over {} interval(s)",
                t * 1e3,
                intervals.len()
            ),
        )
        .with_measured(t),
        None => {
            let d = seq.total_duration;
            CheckResult::pass(
                "metrics.tr",
                format!("single excitation per slice; TR = total duration {d:.6} s"),
            )
            .with_measured(d)
        }
    }
}

/// `metrics.flip_angle` — median nominal flip across excitations (with the
/// spread noted when excitations disagree, e.g. a variable-flip train).
fn flip_angle(exc: &[Excitation<'_>]) -> CheckResult {
    let flips: Vec<f64> = exc.iter().map(|e| e.flip).collect();
    let Some(med) = median(&flips) else {
        return CheckResult::skip("metrics.flip_angle", "no excitation flip to measure");
    };
    let (lo, hi) = flips
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &f| {
            (lo.min(f), hi.max(f))
        });
    let spread = if hi - lo > 1.0 {
        format!(" (range {lo:.2}°–{hi:.2}°)")
    } else {
        String::new()
    };
    CheckResult::pass(
        "metrics.flip_angle",
        format!(
            "flip angle = {med:.2}° (median over {} excitation(s)){spread}",
            flips.len()
        ),
    )
    .with_measured(med)
}

/// `metrics.te` and `metrics.echo_spacing` from the per-excitation echo trains.
/// TE is the median k-space-centre echo time; echo spacing the median
/// centre-to-centre interval (only when a train has more than one echo).
fn te_and_echo_spacing(trains: &[Vec<Echo>]) -> (CheckResult, CheckResult) {
    if trains.is_empty() {
        let te = CheckResult::skip(
            "metrics.te",
            "no readout (ADC) after any excitation; TE not measurable",
        );
        let esp = CheckResult::skip("metrics.echo_spacing", "no echo train to measure");
        return (te, esp);
    }

    // TE: per train, the echo nearest the k-space centre (smallest |pe_area|).
    let te_centres: Vec<f64> = trains
        .iter()
        .filter_map(|t| {
            t.iter()
                .min_by(|a, b| a.pe_area.abs().total_cmp(&b.pe_area.abs()))
                .map(|e| e.te)
        })
        .collect();
    let te = match median(&te_centres) {
        Some(te) => CheckResult::pass(
            "metrics.te",
            format!(
                "effective TE = {te:.6} s ({:.3} ms), k-space-centre echo of {} train(s)",
                te * 1e3,
                trains.len()
            ),
        )
        .with_measured(te),
        None => CheckResult::skip(
            "metrics.te",
            "no readout after any excitation; TE not measurable",
        ),
    };

    // Echo spacing: median diff of sorted echo times, over multi-echo trains.
    let mut spacings = Vec::new();
    for t in trains {
        if t.len() < 2 {
            continue;
        }
        let mut times: Vec<f64> = t.iter().map(|e| e.te).collect();
        times.sort_by(f64::total_cmp);
        for pair in times.windows(2) {
            if let [a, b] = pair {
                spacings.push(b - a);
            }
        }
    }
    let esp = match median(&spacings) {
        Some(s) => CheckResult::pass(
            "metrics.echo_spacing",
            format!("echo spacing = {s:.6} s ({:.3} ms)", s * 1e3),
        )
        .with_measured(s),
        None => CheckResult::skip(
            "metrics.echo_spacing",
            "single echo per excitation: no echo spacing to measure",
        ),
    };
    (te, esp)
}
