//! Step 5 — trajectory gate + dual-witness geometry (`docs/05-trajectory-geometry.md`).
//!
//! A `.seq`'s real geometry lives in its gradients. Integrating them — `k(t) =
//! ∫ G·dt`, in 1/m (Pulseq gradients are already in Hz/m, so no γ is needed) — is
//! **dimension-general**: it follows permuted readouts (MPRAGE on Gz), oblique /
//! rotated blades and spokes (PROPELLER, radial), and non-Cartesian (radial /
//! spiral) readouts that the Cartesian area-algebra cannot. This is the second of
//! the two geometry witnesses.
//!
//! Two witnesses, reconciled:
//!
//! - **Param-algebra** (the Step-4 sibling, emitted as `metrics.fov` /
//!   `metrics.matrix`): measures FOV/matrix from the readout gradient + ADC and
//!   the phase/partition-encode area steps — but **only** when the single-readout-
//!   per-excitation Cartesian model holds (one ADC per excitation, a flat readout
//!   gradient spanning the ADC, no off-axis gradient in the readout block).
//!   Otherwise it `skip`s and defers to the trajectory.
//! - **Trajectory gate** (`trajectory.*`): integrates the interpreted gradients
//!   into the per-ADC-sample k-space path and measures, per axis, extent →
//!   presence (the kz-presence test *is* the 2D-vs-3D question), then coverage,
//!   tightening to an exact count + step only on a uniformly, fully-sampled grid.
//!   It owns geometry whenever the param-algebra `skip`s, and the two are
//!   reconciled (`trajectory.geometry_agreement`) wherever both apply.
//!
//! Unlike the Python harness (`kspace.py`), we integrate the **interpreted**
//! gradients, which the parser has *already rotated* per each block's `ROTATIONS`
//! extension — so a PROPELLER blade or a radial spoke fans out without us
//! re-applying any rotation matrix here.
//!
//! These are measurements, not pass/fail assertions: in file-only mode each is a
//! `pass` carrying its `measured` value or a `skip` when inapplicable; the lone
//! exception is the dual-witness reconciliation, which `warn`s if the two
//! witnesses disagree. Step 7 reuses the same measurements for spec assertions.

use serde_json::Value;

use crate::checks::{Check, CheckCtx};
use crate::ir::{Adc, Gradient, Rf, RfUse, Sequence, Shape};
use crate::metrics::{MIN_EXCITATION_FLIP_DEG, flip_deg, is_non_excitation_use, rf_center_s};
use crate::result::{Category, CheckResult};

/// The trajectory + dual-witness geometry check, wired into [`crate::checks::registry`].
pub(crate) fn checks() -> Vec<Box<dyn Check>> {
    vec![Box::new(TrajectoryGeometry)]
}

// --- thresholds (ported from the harness `sim_traj.py`) ----------------------

/// No-encoding floor [1/m]: a trajectory whose largest axis spans less than this
/// does not sweep k-space beyond numerical jitter (a non-imaging sequence).
const ABS_IMAGING_FLOOR_PER_M: f64 = 1.0;
/// Presence/merge floor as a fraction of the finest in-plane step: an axis
/// spanning less than this carries no real encoding (2D kz jitter, a slice
/// rephaser residual) and is declared absent.
const FLOOR_FRAC: f64 = 0.1;
/// Whole-volume Cartesian band: the product of present axes' distinct counts
/// equals the sample count for a Cartesian (incl. regularly-undersampled) grid; a
/// band absorbs minor merge slack. Radial/spiral volumes blow far past it.
const CART_PRODUCT_LO: f64 = 0.5;
const CART_PRODUCT_HI: f64 = 1.7;
/// Relative tolerance when reconciling the two witnesses' FOV (matrix is exact).
const FOV_REL_TOL: f64 = 0.05;

// --- gradient integration ----------------------------------------------------

/// Trapezoidal integral of a sparse gradient [`Shape`] from `0` to `upto` (s,
/// clamped to the shape's active extent). Mirrors `ir::integrate_shape_ticks` but
/// *partial* (up to an arbitrary time) and already in seconds, so it yields the
/// running k-space moment at any point inside a readout. The waveform is held
/// constant before the first sample and after the last (the IR's interpolation
/// convention) and is piecewise-linear between, so the result is exact at every
/// breakpoint — trapezoids and resampled rotated waveforms alike.
#[allow(clippy::indexing_slicing)] // Shape invariants: time/amp non-empty and equal length
fn shape_partial_integral(shape: &Shape<f64>, upto: f64) -> f64 {
    let upto = upto.clamp(0.0, shape.duration);
    let (t, a) = (&shape.time, &shape.amp);
    let n = t.len();
    // Leading flat segment [0, t[0]] held at a[0].
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
        acc += (a[i - 1] + a_end) * 0.5 * (end - t0);
        if upto <= t1 {
            return acc;
        }
    }
    // Trailing flat segment [t[last], duration] held at a[last].
    acc + a[n - 1] * (upto - t[n - 1])
}

/// Running gradient moment `∫₀^t G·dt` [1/m] up to block-relative time `t` [s].
fn grad_partial_area(g: &Gradient, t: f64) -> f64 {
    let local = t - g.delay;
    if local <= 0.0 {
        0.0
    } else {
        g.amp * shape_partial_integral(&g.shape, local)
    }
}

/// Full gradient moment `∫ G·dt` [1/m] over the gradient's active extent.
fn grad_area(g: &Gradient) -> f64 {
    g.amp * shape_partial_integral(&g.shape, g.shape.duration)
}

/// Instantaneous gradient amplitude [Hz/m] at block-relative time `t` [s]; `0`
/// outside the gradient's active window.
fn grad_value(g: &Gradient, t: f64) -> f64 {
    let local = t - g.delay;
    if local < 0.0 || local > g.shape.duration {
        0.0
    } else {
        g.amp * g.shape.interpolate(local)
    }
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

// --- analytic k-space --------------------------------------------------------

/// Whether an RF event resets k-space: the slice excitation (its `use` tag is not
/// a non-excitation role and it carries a real flip). Shared definition with the
/// derived-metrics pass (see [`crate::metrics`]).
fn is_excitation(rf: &Rf) -> bool {
    !is_non_excitation_use(rf.rf_use) && flip_deg(rf) >= MIN_EXCITATION_FLIP_DEG
}

/// The per-ADC-sample k-space trajectory, as `[kx, ky, kz]` rows in 1/m, in
/// acquisition order. A single forward pass over the interpreted blocks tracks the
/// running k vector with the standard convention — k resets to zero at each
/// **excitation** and is conjugated (negated) at each **refocusing** pulse — and
/// emits one row per ADC sample, integrating the in-readout gradient so a
/// swept/blipped readout (EPI/spiral) is followed exactly. The interpreted
/// gradients are already FOV-scaled and rotated, so this is the *physical* path.
fn analytic_kspace(seq: &Sequence) -> Vec<[f64; 3]> {
    let mut k = [0.0_f64; 3];
    let mut samples: Vec<[f64; 3]> = Vec::new();

    for b in &seq.blocks {
        let area = [
            b.gx.as_ref().map_or(0.0, grad_area),
            b.gy.as_ref().map_or(0.0, grad_area),
            b.gz.as_ref().map_or(0.0, grad_area),
        ];
        let partial = |t: f64| -> [f64; 3] {
            [
                b.gx.as_ref().map_or(0.0, |g| grad_partial_area(g, t)),
                b.gy.as_ref().map_or(0.0, |g| grad_partial_area(g, t)),
                b.gz.as_ref().map_or(0.0, |g| grad_partial_area(g, t)),
            ]
        };

        match (b.rf.as_ref(), b.adc.as_ref()) {
            (Some(rf), adc_opt) => {
                // Integrate up to the RF centre, then apply the reset/conjugation.
                let pre = partial(rf_center_s(rf));
                k = add3(k, pre);
                if is_excitation(rf) {
                    k = [0.0; 3];
                } else if matches!(rf.rf_use, RfUse::Refocusing) {
                    k = [-k[0], -k[1], -k[2]];
                }
                let k_ref = k;
                // Rare: an ADC in the same block as the RF (UTE/ZTE).
                if let Some(adc) = adc_opt {
                    for s in 0..adc.num {
                        let ts = adc.delay + (f64::from(s) + 0.5) * adc.dwell;
                        samples.push(add3(k_ref, sub3(partial(ts), pre)));
                    }
                }
                k = add3(k, sub3(area, pre));
            }
            (None, Some(adc)) => {
                for s in 0..adc.num {
                    let ts = adc.delay + (f64::from(s) + 0.5) * adc.dwell;
                    samples.push(add3(k, partial(ts)));
                }
                k = add3(k, area);
            }
            (None, None) => {
                k = add3(k, area);
            }
        }
    }
    samples
}

// --- per-axis measurement (ported from `sim_traj.py::measure_trajectory`) ----

/// One axis's measured k-space geometry.
struct AxisMeas {
    /// The axis carries real encoding (extent above the jitter floor).
    present: bool,
    /// Distinct sample positions (merged within the jitter gap).
    count: usize,
    /// Modal step between distinct positions [1/m], if any.
    step: Option<f64>,
    /// Full extent (max − min) [1/m].
    extent: f64,
    /// A uniformly, fully-sampled grid axis — exact count + step are trustworthy.
    exact_ok: bool,
}

impl AxisMeas {
    /// Exact matrix count, when this axis is a clean grid.
    fn matrix(&self) -> Option<u64> {
        self.exact_ok.then_some(self.count as u64)
    }
    /// Grid FOV [mm] from the modal step (`1/Δk`), when this axis is a clean grid.
    fn fov_mm(&self) -> Option<f64> {
        if self.exact_ok {
            self.step.filter(|&s| s > 0.0).map(|s| 1.0 / s * 1e3)
        } else {
            None
        }
    }
}

/// The measured trajectory geometry across all three axes.
struct TrajMeas {
    /// At least one axis encodes k-space.
    imaging: bool,
    /// kz is present — the 2D-vs-3D headline.
    is_3d: bool,
    /// Per-axis measurement, `[x, y, z]`.
    axes: [AxisMeas; 3],
}

/// Sorted distinct values, merging entries closer than `gap` (repeats of the same
/// line/partition across slices/averages are written identically and collapse;
/// real steps are orders of magnitude larger).
fn distinct_sorted(vals: &[f64], gap: f64) -> Vec<f64> {
    let mut v = vals.to_vec();
    v.sort_by(f64::total_cmp);
    let mut keep: Vec<f64> = Vec::new();
    for x in v {
        if keep.last().is_none_or(|&l| (x - l).abs() > gap) {
            keep.push(x);
        }
    }
    keep
}

/// Adjacent differences of a sorted slice.
fn diffs(v: &[f64]) -> Vec<f64> {
    v.iter().zip(v.iter().skip(1)).map(|(a, b)| b - a).collect()
}

/// numpy-style median of a slice (mean of the two central values for an even
/// count); `0.0` for an empty input.
fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let mid = n / 2;
    if n % 2 == 1 {
        s.get(mid).copied().unwrap_or(0.0)
    } else {
        let lo = s.get(mid - 1).copied().unwrap_or(0.0);
        let hi = s.get(mid).copied().unwrap_or(0.0);
        0.5 * (lo + hi)
    }
}

/// True when the distinct positions sit on a single regular lattice (one modal
/// step, no gaps) — the fullness test that gates exact counting. A GRAPPA axis
/// (ACS lines spaced 1, outer lines spaced 2) is non-uniform and routes to
/// coverage; a fully, regularly sampled axis is uniform.
fn is_uniform(distinct: &[f64]) -> bool {
    if distinct.len() < 3 {
        return true; // 0/1/2 points are trivially "uniform"
    }
    let d = diffs(distinct);
    let modal = median(&d);
    if modal <= 0.0 {
        return false;
    }
    d.iter().all(|&x| (x - modal).abs() <= 0.1 * modal)
}

fn col_extent(c: &[f64]) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &x in c {
        lo = lo.min(x);
        hi = hi.max(x);
    }
    if hi >= lo { hi - lo } else { 0.0 }
}

fn col_scale(c: &[f64]) -> f64 {
    c.iter().fold(0.0_f64, |m, &x| m.max(x.abs()))
}

/// Measure the k-space geometry of the trajectory with one symmetric per-axis
/// routine: extent → presence, distinct count + modal step, uniformity, then the
/// grid tests (whole-volume Cartesian *or* a low-cardinality uniform ladder).
#[allow(clippy::indexing_slicing)] // every array below is length 3, indexed by a ∈ 0..3
fn measure(kspace: &[[f64; 3]]) -> TrajMeas {
    let n = kspace.len();
    let cols: [Vec<f64>; 3] = [
        kspace.iter().map(|s| s[0]).collect(),
        kspace.iter().map(|s| s[1]).collect(),
        kspace.iter().map(|s| s[2]).collect(),
    ];
    let extent: [f64; 3] = std::array::from_fn(|a| col_extent(&cols[a]));
    let scale: [f64; 3] = std::array::from_fn(|a| col_scale(&cols[a]));
    let max_extent = extent.iter().copied().fold(0.0_f64, f64::max);

    let blank = |a: usize| AxisMeas {
        present: false,
        count: 1,
        step: None,
        extent: extent[a],
        exact_ok: false,
    };
    // No encoding anywhere: a non-imaging sequence.
    if n < 2 || max_extent <= ABS_IMAGING_FLOOR_PER_M {
        return TrajMeas {
            imaging: false,
            is_3d: false,
            axes: std::array::from_fn(blank),
        };
    }

    // Reference in-plane step: the finest characteristic step among the axes that
    // are unambiguously present (extent a meaningful fraction of the largest).
    let mut fine_steps: Vec<f64> = Vec::new();
    for a in 0..3 {
        if extent[a] <= 0.05 * max_extent {
            continue;
        }
        let fine = distinct_sorted(&cols[a], (1e-3 * scale[a]).max(0.0));
        if fine.len() >= 2 {
            let step = median(&diffs(&fine));
            if step > 0.0 {
                fine_steps.push(step);
            }
        }
    }
    let dk_ref = fine_steps.iter().copied().fold(f64::INFINITY, f64::min);
    let dk_ref = if dk_ref.is_finite() {
        dk_ref
    } else {
        max_extent
    };
    let floor = FLOOR_FRAC * dk_ref;

    let mut count = [0usize; 3];
    let mut present = [false; 3];
    let mut step: [Option<f64>; 3] = [None; 3];
    let mut uniform = [true; 3];
    for a in 0..3 {
        let gap = (1e-3 * scale[a]).max(floor);
        let d = distinct_sorted(&cols[a], gap);
        count[a] = d.len();
        present[a] = d.len() >= 2 && extent[a] > floor;
        step[a] = (d.len() >= 2).then(|| median(&diffs(&d)));
        uniform[a] = is_uniform(&d);
    }

    // Whole-volume Cartesian? product of present axes' distinct counts vs n.
    let product: usize = (0..3).filter(|&a| present[a]).map(|a| count[a]).product();
    let nf = n as f64;
    let cartesian_volume =
        CART_PRODUCT_LO * nf <= product as f64 && product as f64 <= CART_PRODUCT_HI * nf;
    let sqrt_n = nf.sqrt();

    // exact_ok: a uniform grid axis. Any uniform axis qualifies in a Cartesian
    // volume; otherwise only a low-cardinality uniform ladder (`count < √n`) does
    // — which keeps a stack-of-spirals' z exact while a 3D radial's polar axis
    // (≈ one value per spoke) falls to coverage.
    let axes: [AxisMeas; 3] = std::array::from_fn(|a| {
        let low_card = (count[a] as f64) < sqrt_n;
        let grid = cartesian_volume || low_card;
        AxisMeas {
            present: present[a],
            count: count[a],
            step: step[a],
            extent: extent[a],
            exact_ok: present[a] && uniform[a] && grid,
        }
    });

    TrajMeas {
        imaging: axes.iter().any(|a| a.present),
        is_3d: axes.get(2).is_some_and(|a| a.present),
        axes,
    }
}

// --- param-algebra geometry (ported from `param_check.py::measure`) ----------

/// FOV/matrix measured by the Cartesian area-algebra. Only produced when the
/// single-readout-per-excitation Cartesian model holds.
struct ParamGeom {
    /// `[matrix_x, matrix_y, matrix_z]` (z is `1` for a single-partition family).
    matrix: [i64; 3],
    /// `[fov_x, fov_y, fov_z]` [mm]; `fov_z` is `None` for a single partition
    /// (slice thickness is not witnessed by the area-algebra here).
    fov_mm: [Option<f64>; 3],
}

/// Block indices of every excitation, in file order.
fn excitation_blocks(seq: &Sequence) -> Vec<usize> {
    seq.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let rf = b.rf.as_ref()?;
            is_excitation(rf).then_some(i)
        })
        .collect()
}

/// One acquired readout: its block index and the net phase- / partition-encode
/// moments set between the excitation and the readout (logical frame).
struct Shot {
    adc_bi: usize,
    pe_area: f64,
    pz_area: f64,
}

/// The first ADC block after each excitation (dummy shots with no ADC skipped),
/// with the gy/gz moments accumulated from just after the excitation block (so a
/// slice-select gz in the excitation block is excluded). Uses the *logical*
/// (pre-rotation) areas; this witness applies only to non-rotated readouts, where
/// logical and physical areas coincide.
fn find_shots(seq: &Sequence, exc: &[usize]) -> Vec<Shot> {
    let n_blocks = seq.blocks.len();
    let mut shots = Vec::new();
    for (k, &ei) in exc.iter().enumerate() {
        let next = exc.get(k + 1).copied().unwrap_or(n_blocks);
        let Some(adc_bi) =
            (ei + 1..next).find(|&j| seq.blocks.get(j).is_some_and(|b| b.adc.is_some()))
        else {
            continue;
        };
        let mut pe = 0.0;
        let mut pz = 0.0;
        for j in (ei + 1)..adc_bi {
            if let Some(area) = seq.logical_grad_areas.get(j) {
                pe += area[1];
                pz += area[2];
            }
        }
        shots.push(Shot {
            adc_bi,
            pe_area: pe,
            pz_area: pz,
        });
    }
    shots
}

/// True when `gx` is a flat readout gradient (constant, non-zero) across the ADC
/// sampling window — the signature of an axis-aligned Cartesian readout. A
/// phase/partition blip, a ramp, or a swept (spiral) readout is not flat and is
/// rejected.
fn readout_covers_adc(gx: &Gradient, adc: &Adc) -> bool {
    if adc.num == 0 {
        return false;
    }
    let center = |i: u32| grad_value(gx, adc.delay + (f64::from(i) + 0.5) * adc.dwell);
    let mid = center(adc.num / 2);
    let amp = mid.abs();
    if amp == 0.0 {
        return false;
    }
    let probes = [
        0,
        adc.num / 4,
        adc.num / 2,
        (adc.num / 4) * 3,
        adc.num.saturating_sub(1),
    ];
    probes
        .iter()
        .all(|&i| (center(i) - mid).abs() <= 1e-6 * amp)
}

/// Measure FOV/matrix by the Cartesian area-algebra, or `Err(reason)` when the
/// single-readout-per-excitation Cartesian model does not apply (the caller turns
/// that into a `skip`, deferring to the trajectory gate).
fn param_geometry(seq: &Sequence) -> Result<ParamGeom, String> {
    let exc = excitation_blocks(seq);
    if exc.is_empty() {
        return Err("no excitation RF in the sequence".into());
    }
    let n_adc = seq.blocks.iter().filter(|b| b.adc.is_some()).count();
    let shots = find_shots(seq, &exc);
    let Some(first) = shots.first() else {
        return Err("no readout (ADC) after any excitation".into());
    };
    if n_adc > exc.len() {
        return Err("echo train / multiple readouts per excitation (e.g. EPI/TSE)".into());
    }

    // An off-axis gradient in ANY readout block means the readout is rotated,
    // oblique, or permuted: the parser bakes a block's `ROTATIONS` extension into
    // the interpreted gradients, so a rotated spoke shows up as a gy/gz running
    // alongside the readout gx. Tested across all shots — a golden-angle fan's
    // first spoke is at φ=0 (identity), so the rotation only shows from the next.
    let off_axis = shots.iter().any(|s| {
        seq.blocks
            .get(s.adc_bi)
            .is_some_and(|b| b.gy.is_some() || b.gz.is_some())
    });
    if off_axis {
        return Err(
            "off-axis gradient in the readout block (non-Cartesian / oblique / permuted readout)"
                .into(),
        );
    }

    let block = seq.blocks.get(first.adc_bi);
    let (Some(gx), Some(adc)) = (
        block.and_then(|b| b.gx.as_ref()),
        block.and_then(|b| b.adc.as_ref()),
    ) else {
        return Err("readout block has no gx readout gradient (permuted readout)".into());
    };
    if !readout_covers_adc(gx, adc) {
        return Err("readout gradient is not flat across the ADC (non-Cartesian readout)".into());
    }

    // matrix_x / fov_x from the readout gradient amplitude (read on its flat top)
    // and the ADC dwell.
    let g_ro = grad_value(gx, adc.delay + (f64::from(adc.num) * 0.5 + 0.5) * adc.dwell).abs();
    let matrix_x = i64::from(adc.num);
    let fov_x = (g_ro > 0.0).then(|| 1.0 / (g_ro * adc.dwell) * 1e3);

    // matrix_y / fov_y from the distinct phase-encode area steps.
    let (matrix_y, fov_y) = count_and_fov(shots.iter().map(|s| s.pe_area));
    // matrix_z / fov_z from the distinct partition-encode area steps (z = 1 for a
    // single-partition family, where fov_z is left to a future slice-select witness).
    let (matrix_z, fov_z) = count_and_fov(shots.iter().map(|s| s.pz_area));

    Ok(ParamGeom {
        matrix: [matrix_x, matrix_y, matrix_z],
        fov_mm: [fov_x, fov_y, fov_z],
    })
}

/// Distinct-count and FOV (`1/Δarea·1e3` mm) from a set of encode areas: the
/// number of distinct values and, when ≥ 2, the modal step's reciprocal.
fn count_and_fov(areas: impl Iterator<Item = f64>) -> (i64, Option<f64>) {
    let areas: Vec<f64> = areas.collect();
    let scale = areas.iter().fold(0.0_f64, |m, &a| m.max(a.abs()));
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let distinct = distinct_sorted(&areas, 1e-3 * scale);
    let count = distinct.len() as i64;
    let fov = if distinct.len() >= 2 {
        let d = median(&diffs(&distinct));
        (d > 0.0).then(|| 1.0 / d * 1e3)
    } else {
        None
    };
    (count, fov)
}

// --- the check ---------------------------------------------------------------

/// Computes the analytic trajectory once, measures its geometry, runs the
/// param-algebra witness, and emits the dual-witness result set.
struct TrajectoryGeometry;

impl Check for TrajectoryGeometry {
    fn category(&self) -> Category {
        Category::Trajectory
    }
    fn name(&self) -> &'static str {
        // Unused: this check emits explicit per-result ids (some under `metrics.`,
        // the param-algebra sibling) rather than the default `<category>.<name>`.
        "geometry"
    }

    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let kspace = analytic_kspace(ctx.seq);
        let meas = measure(&kspace);
        let param = param_geometry(ctx.seq);

        vec![
            param_matrix_result(&param),
            param_fov_result(&param),
            dimensionality_result(&meas),
            extent_result(&meas),
            traj_matrix_result(&meas),
            traj_fov_result(&meas),
            agreement_result(&meas, &param),
        ]
    }
}

/// Format an `Option<f64>` mm value (or `—` when not measured).
fn fmt_mm(x: Option<f64>) -> String {
    x.map_or_else(|| "—".to_string(), |v| format!("{v:.1}"))
}

/// `metrics.matrix` — the Cartesian area-algebra matrix, or a skip deferring to
/// the trajectory gate.
fn param_matrix_result(param: &Result<ParamGeom, String>) -> CheckResult {
    match param {
        Ok(p) => CheckResult::pass(
            "metrics.matrix",
            format!(
                "matrix = [{}, {}, {}] (Cartesian area-algebra: readout samples × \
                 phase-encode lines × partitions)",
                p.matrix[0], p.matrix[1], p.matrix[2]
            ),
        )
        .with_measured(Value::Array(
            p.matrix.iter().map(|&m| Value::from(m)).collect(),
        )),
        Err(reason) => CheckResult::skip(
            "metrics.matrix",
            format!("{reason}; geometry deferred to the trajectory gate"),
        ),
    }
}

/// `metrics.fov` — the Cartesian area-algebra FOV [mm], or a skip.
fn param_fov_result(param: &Result<ParamGeom, String>) -> CheckResult {
    match param {
        Ok(p) => CheckResult::pass(
            "metrics.fov",
            format!(
                "FOV = [{}, {}, {}] mm (Cartesian area-algebra)",
                fmt_mm(p.fov_mm[0]),
                fmt_mm(p.fov_mm[1]),
                fmt_mm(p.fov_mm[2])
            ),
        )
        .with_measured(Value::Array(
            p.fov_mm
                .iter()
                .map(|f| f.map_or(Value::Null, Value::from))
                .collect(),
        )),
        Err(reason) => CheckResult::skip(
            "metrics.fov",
            format!("{reason}; geometry deferred to the trajectory gate"),
        ),
    }
}

/// `trajectory.dimensionality` — the 2D-vs-3D headline (the kz-presence test).
fn dimensionality_result(meas: &TrajMeas) -> CheckResult {
    if !meas.imaging {
        return CheckResult::skip(
            "trajectory.dimensionality",
            "no k-space encoding detected (gradients integrate to ~0); not an imaging sequence",
        );
    }
    let names = ["kx", "ky", "kz"];
    let encoded: Vec<&str> = meas
        .axes
        .iter()
        .zip(names)
        .filter(|(a, _)| a.present)
        .map(|(_, n)| n)
        .collect();
    // The 2D-vs-3D headline is the kz-presence test (`docs/05`): 3D iff kz encodes.
    let dims = if meas.is_3d { 3u64 } else { 2 };
    CheckResult::pass(
        "trajectory.dimensionality",
        format!(
            "{dims}D acquisition (kz {}; encoded axes: {})",
            if meas.is_3d { "present" } else { "absent" },
            encoded.join(", ")
        ),
    )
    .with_measured(dims)
}

/// `trajectory.extent` — per-axis k-space coverage [1/m], the general witness.
fn extent_result(meas: &TrajMeas) -> CheckResult {
    if !meas.imaging {
        return CheckResult::skip("trajectory.extent", "no k-space encoding to measure");
    }
    let e = [
        meas.axes[0].extent,
        meas.axes[1].extent,
        meas.axes[2].extent,
    ];
    CheckResult::pass(
        "trajectory.extent",
        format!(
            "k-space extent = [{:.1}, {:.1}, {:.1}] 1/m (per-axis coverage)",
            e[0], e[1], e[2]
        ),
    )
    .with_measured(Value::Array(e.iter().map(|&x| Value::from(x)).collect()))
}

/// `trajectory.matrix` — the gradient-integrated matrix on clean-grid axes
/// (`null` where an axis is not a uniformly fully-sampled grid).
fn traj_matrix_result(meas: &TrajMeas) -> CheckResult {
    if !meas.imaging {
        return CheckResult::skip("trajectory.matrix", "no k-space encoding to measure");
    }
    let m = [
        meas.axes[0].matrix(),
        meas.axes[1].matrix(),
        meas.axes[2].matrix(),
    ];
    if m.iter().all(Option::is_none) {
        return CheckResult::skip(
            "trajectory.matrix",
            "no axis is a uniformly, fully-sampled grid (non-Cartesian or accelerated); \
             see trajectory.extent for coverage",
        );
    }
    CheckResult::pass(
        "trajectory.matrix",
        format!(
            "matrix = [{}, {}, {}] (gradient-integrated; — = axis not a clean grid)",
            m[0].map_or_else(|| "—".to_string(), |v| v.to_string()),
            m[1].map_or_else(|| "—".to_string(), |v| v.to_string()),
            m[2].map_or_else(|| "—".to_string(), |v| v.to_string()),
        ),
    )
    .with_measured(Value::Array(
        m.iter()
            .map(|o| o.map_or(Value::Null, Value::from))
            .collect(),
    ))
}

/// `trajectory.fov` — the gradient-integrated FOV [mm] on clean-grid axes.
fn traj_fov_result(meas: &TrajMeas) -> CheckResult {
    if !meas.imaging {
        return CheckResult::skip("trajectory.fov", "no k-space encoding to measure");
    }
    let f = [
        meas.axes[0].fov_mm(),
        meas.axes[1].fov_mm(),
        meas.axes[2].fov_mm(),
    ];
    if f.iter().all(Option::is_none) {
        return CheckResult::skip(
            "trajectory.fov",
            "no axis is a uniformly, fully-sampled grid (non-Cartesian or accelerated); \
             see trajectory.extent for coverage",
        );
    }
    CheckResult::pass(
        "trajectory.fov",
        format!(
            "FOV = [{}, {}, {}] mm (gradient-integrated; — = axis not a clean grid)",
            fmt_mm(f[0]),
            fmt_mm(f[1]),
            fmt_mm(f[2])
        ),
    )
    .with_measured(Value::Array(
        f.iter()
            .map(|o| o.map_or(Value::Null, Value::from))
            .collect(),
    ))
}

/// `trajectory.geometry_agreement` — reconcile the two witnesses on every axis
/// both measure. `pass` when they agree, `warn` on a disagreement, `skip` when no
/// axis is covered by both.
fn agreement_result(meas: &TrajMeas, param: &Result<ParamGeom, String>) -> CheckResult {
    let Ok(p) = param else {
        return CheckResult::skip(
            "trajectory.geometry_agreement",
            "param-algebra geometry not applicable; only the trajectory witness measured geometry",
        );
    };

    let mut compared = 0usize;
    let mut issues: Vec<String> = Vec::new();
    for (name, (axis, (pm, pf))) in ["x", "y", "z"]
        .iter()
        .zip(meas.axes.iter().zip(p.matrix.iter().zip(p.fov_mm.iter())))
    {
        if !axis.exact_ok {
            continue; // the trajectory does not claim an exact value on this axis
        }
        compared += 1;
        let traj_m = axis.count as i64;
        if traj_m != *pm {
            issues.push(format!(
                "matrix_{name}: param-algebra {pm} vs trajectory {traj_m}"
            ));
        }
        if let (Some(&pf), Some(tf)) = (pf.as_ref(), axis.fov_mm()) {
            let rel = (pf - tf).abs() / pf.abs().max(1e-9);
            if rel > FOV_REL_TOL {
                issues.push(format!(
                    "fov_{name}: param-algebra {pf:.1} vs trajectory {tf:.1} mm ({:.1}%)",
                    rel * 100.0
                ));
            }
        }
    }

    if compared == 0 {
        CheckResult::skip(
            "trajectory.geometry_agreement",
            "no axis measured by both witnesses (the trajectory found no clean-grid axis the \
             area-algebra also covers)",
        )
    } else if issues.is_empty() {
        CheckResult::pass(
            "trajectory.geometry_agreement",
            format!(
                "dual-witness geometry agrees on {compared} axis/axes (area-algebra ↔ trajectory)"
            ),
        )
    } else {
        CheckResult::warn(
            "trajectory.geometry_agreement",
            format!("dual-witness geometry disagreement — {}", issues.join("; ")),
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]
    use super::*;

    /// A perfectly Cartesian k-space lattice: `ny` lines × `nx` readout samples,
    /// each line a full sweep on a uniform kx grid, with optional partitions.
    fn grid(nx: usize, ny: usize, nz: usize, dkx: f64, dky: f64, dkz: f64) -> Vec<[f64; 3]> {
        let mut v = Vec::new();
        for iz in 0..nz {
            for iy in 0..ny {
                for ix in 0..nx {
                    v.push([ix as f64 * dkx, iy as f64 * dky, iz as f64 * dkz]);
                }
            }
        }
        v
    }

    #[test]
    fn cartesian_2d_grid_is_exact() {
        // dk = 5 1/m ⟺ FOV 200 mm; 32×32.
        let m = measure(&grid(32, 32, 1, 5.0, 5.0, 0.0));
        assert!(m.imaging && !m.is_3d);
        assert_eq!(m.axes[0].matrix(), Some(32));
        assert_eq!(m.axes[1].matrix(), Some(32));
        assert_eq!(m.axes[2].matrix(), None, "no partition encode ⇒ 2D");
        assert!((m.axes[0].fov_mm().unwrap() - 200.0).abs() < 1.0);
        assert!((m.axes[1].fov_mm().unwrap() - 200.0).abs() < 1.0);
    }

    #[test]
    fn cartesian_3d_grid_detects_partitions() {
        // 16×16×8, dk = 10/10/20 1/m ⟺ FOV 100/100/50 mm.
        let m = measure(&grid(16, 16, 8, 10.0, 10.0, 20.0));
        assert!(m.imaging && m.is_3d, "kz present ⇒ 3D");
        assert_eq!(m.axes[2].matrix(), Some(8));
        assert!((m.axes[2].fov_mm().unwrap() - 50.0).abs() < 1.0);
    }

    #[test]
    fn accelerated_axis_falls_back_to_coverage() {
        // ky reaches full extent but is non-uniform (R=2 outer + dense ACS centre,
        // a GRAPPA pattern): it must NOT be treated as an exact grid, yet must stay
        // present with full extent (coverage), while kx remains a clean grid.
        let dk = 5.0;
        let mut ky: Vec<f64> = Vec::new();
        let mut y = -16.0;
        while y <= -6.0 {
            ky.push(y);
            y += 2.0;
        } // R=2 outer
        let mut y = -5.0;
        while y <= 5.0 {
            ky.push(y);
            y += 1.0;
        } // ACS centre
        let mut y = 6.0;
        while y <= 16.0 {
            ky.push(y);
            y += 2.0;
        } // R=2 outer
        let mut k = Vec::new();
        for &kyi in &ky {
            for ix in 0..32 {
                k.push([ix as f64 * dk, kyi * dk, 0.0]);
            }
        }
        let m = measure(&k);
        assert!(m.axes[1].present, "an accelerated axis is still present");
        assert!(
            !m.axes[1].exact_ok,
            "non-uniform ⇒ coverage, not an exact count"
        );
        assert!(
            m.axes[1].extent > 0.9 * 32.0 * dk,
            "full extent still reached"
        );
        assert_eq!(
            m.axes[0].matrix(),
            Some(32),
            "readout is still a clean grid"
        );
    }

    #[test]
    fn coverage_is_oversampling_invariant() {
        // Oversampling = the SAME k-space extent (kmax) sampled more densely. The
        // extent — the resolution invariant the coverage witness reports — is
        // unchanged; only the exact count/step scale with the oversampling factor.
        let line = |nx: usize, kmax: f64| -> Vec<[f64; 3]> {
            (0..nx)
                .map(|i| [i as f64 * kmax / (nx as f64 - 1.0), 0.0, 0.0])
                .collect()
        };
        let plain = measure(&line(32, 155.0));
        let over = measure(&line(64, 155.0)); // 2× oversampled, same extent
        assert!((plain.axes[0].extent - over.axes[0].extent).abs() < 1e-6);
        assert_eq!(plain.axes[0].matrix(), Some(32));
        assert_eq!(over.axes[0].matrix(), Some(64));
    }

    #[test]
    fn no_encoding_is_non_imaging() {
        // Sub-floor jitter only (< 1 1/m): not an imaging trajectory.
        let m = measure(&[[0.0, 0.0, 0.0], [0.3, 0.1, 0.0], [0.2, 0.4, 0.0]]);
        assert!(!m.imaging);
        assert!(m.axes.iter().all(|a| !a.present));
    }

    #[test]
    fn partial_integral_of_trapezoid() {
        // Unit trapezoid: rise 1, flat 2, fall 1 (area = 3).
        let s = Shape {
            time: vec![0.0, 1.0, 3.0, 4.0],
            amp: vec![0.0, 1.0, 1.0, 0.0],
            duration: 4.0,
        };
        assert!((shape_partial_integral(&s, 0.0)).abs() < 1e-12);
        assert!((shape_partial_integral(&s, 1.0) - 0.5).abs() < 1e-12); // rise only
        assert!((shape_partial_integral(&s, 2.0) - 1.5).abs() < 1e-12); // rise + 1 flat
        assert!((shape_partial_integral(&s, 4.0) - 3.0).abs() < 1e-12); // full
        assert!((shape_partial_integral(&s, 9.9) - 3.0).abs() < 1e-12); // clamps past end
    }

    #[test]
    fn uniformity_detects_gaps() {
        assert!(is_uniform(&[0.0, 1.0, 2.0, 3.0]));
        assert!(!is_uniform(&[0.0, 1.0, 2.0, 4.0])); // a doubled gap
        assert!(is_uniform(&[0.0, 1.0])); // <3 points trivially uniform
    }
}
