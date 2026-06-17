//! Acceptance tests for the derived imaging metrics.
//!
//! Two strategies, mirroring `integrity.rs`:
//!
//!   * **Synthetic, fully-controlled v1.5.1 sequences** with hand-computed
//!     answers, one per behaviour: a single-shot GRE (clean known flip/TE/TR), a
//!     two-echo train whose central ky line is the *second* echo (the
//!     effective-TE-is-k-centre-not-first proof), a two-slice multi-rep sequence
//!     (n_slices and a real TR interval), and an RF-free sequence (every metric
//!     but scan time is a `skip`).
//!   * **The bundled fixtures** for the headline acceptance: the example file's
//!     metrics are sane and pinned (criterion 1), and the echo-train fixtures
//!     (HASTE, PROPELLER) confirm the effective TE lands on the mid-train
//!     k-space-centre echo, not the first echo (criterion 3). Every fixture value
//!     here is pinned as a regression baseline.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use seq_validate_core::checks::run_all;
use seq_validate_core::serde_json::Value;
use seq_validate_core::{CheckCtx, CheckResult, DEFAULT_LARMOR_HZ, Sequence, Status};

/// Single-shot GRE: one 90° excitation (a 2-sample block pulse, `amp·∫ =
/// 125000·2e-6 = 0.25` turns → 90°) at RF-centre 1 µs, then a 64-sample, 1 µs
/// dwell readout starting at 10 µs. Hand-computed: flip 90°, scan time 80 µs,
/// n_slices 1, single excitation ⇒ TR = total duration, single echo ⇒ TE =
/// 10 µs + 32 µs − 1 µs = 41 µs and no echo spacing.
const SINGLE_SHOT_GRE: &str = "\
[VERSION]
major 1
minor 5
revision 1

[DEFINITIONS]
AdcRasterTime 1e-07
BlockDurationRaster 1e-05
FOV 0.25 0.25 0.005
GradientRasterTime 1e-05
RadiofrequencyRasterTime 1e-06
TotalDuration 8e-05

[BLOCKS]
1 1 1 0 0 0 0 0
2 7 0 0 0 0 1 0

[RF]
1 125000 1 2 0 1 0 0 0 0 0 e

[ADC]
1 64 1000 0 0 0 0 0 0

[SHAPES]
shape_id 1
num_samples 2
1
1

shape_id 2
num_samples 2
0
0
";

/// Two-echo train: excitation, a +area gy prephaser, echo 1 (at ky = +area),
/// an equal −area gy rewinder, echo 2 (at ky = 0). The k-space-centre echo is
/// therefore the *second* one. Hand-computed: echo 1 at 141 µs (ky ≠ 0), echo 2
/// at 311 µs (ky = 0) ⇒ effective TE = 311 µs (not 141 µs), echo spacing
/// 311 − 141 = 170 µs.
const MULTI_ECHO: &str = "\
[VERSION]
major 1
minor 5
revision 1

[DEFINITIONS]
AdcRasterTime 1e-07
BlockDurationRaster 1e-05
FOV 0.25 0.25 0.005
GradientRasterTime 1e-05
RadiofrequencyRasterTime 1e-06
TotalDuration 0.00035

[BLOCKS]
1 1 1 0 0 0 0 0
2 10 0 0 1 0 0 0
3 7 0 0 0 0 1 0
4 10 0 0 2 0 0 0
5 7 0 0 0 0 1 0

[RF]
1 125000 1 2 0 1 0 0 0 0 0 e

[ADC]
1 64 1000 0 0 0 0 0 0

[TRAP]
1 1000 10 80 10 0
2 -1000 10 80 10 0

[SHAPES]
shape_id 1
num_samples 2
1
1

shape_id 2
num_samples 2
0
0
";

/// Two slices (RF frequency offsets 0 Hz and 1000 Hz), two excitations each,
/// interleaved, 100 µs apart per block. Hand-computed: n_slices 2, and each
/// slice's two excitations are 200 µs apart ⇒ TR = 200 µs. No ADC ⇒ TE and echo
/// spacing both `skip`.
const MULTI_SLICE: &str = "\
[VERSION]
major 1
minor 5
revision 1

[DEFINITIONS]
AdcRasterTime 1e-07
BlockDurationRaster 1e-05
FOV 0.25 0.25 0.005
GradientRasterTime 1e-05
RadiofrequencyRasterTime 1e-06
TotalDuration 0.0004

[BLOCKS]
1 10 1 0 0 0 0 0
2 10 2 0 0 0 0 0
3 10 1 0 0 0 0 0
4 10 2 0 0 0 0 0

[RF]
1 125000 1 2 0 1 0 0 0 0 0 e
2 125000 1 2 0 1 0 0 0 1000 0 e

[SHAPES]
shape_id 1
num_samples 2
1
1

shape_id 2
num_samples 2
0
0
";

/// A valid sequence with a gradient and an ADC but **no RF at all** — no
/// excitation, so every metric but scan time is unmeasurable.
const NO_RF: &str = "\
[VERSION]
major 1
minor 5
revision 1

[DEFINITIONS]
AdcRasterTime 1e-07
BlockDurationRaster 1e-05
FOV 0.25 0.25 0.005
GradientRasterTime 1e-05
RadiofrequencyRasterTime 1e-06
TotalDuration 0.0005

[BLOCKS]
1 30 0 1 0 0 0 0
2 20 0 0 0 0 1 0

[TRAP]
1 1000 50 100 50 0

[ADC]
1 64 1000 0 0 0 0 0 0
";

fn fixture(name: &str) -> String {
    format!("{}/../../fixtures/{name}.seq", env!("CARGO_MANIFEST_DIR"))
}

fn results_for(source: &str) -> Vec<CheckResult> {
    let seq = Sequence::from_source(source, DEFAULT_LARMOR_HZ).expect("source must parse");
    run_all(&CheckCtx {
        seq: &seq,
        profile: None,
    })
}

fn results_for_fixture(name: &str) -> Vec<CheckResult> {
    let seq = Sequence::from_file(fixture(name)).expect("fixture must parse");
    run_all(&CheckCtx {
        seq: &seq,
        profile: None,
    })
}

fn get<'a>(results: &'a [CheckResult], id: &str) -> &'a CheckResult {
    results
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no result with id `{id}` in {results:#?}"))
}

/// The numeric `measured` value of a metric (panics if it is missing/non-numeric).
fn measured(results: &[CheckResult], id: &str) -> f64 {
    let r = get(results, id);
    r.measured
        .as_ref()
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("`{id}` has no numeric measured value: {r:#?}"))
}

#[track_caller]
fn assert_close(actual: f64, expected: f64, tol: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: got {actual}, expected {expected} (±{tol})"
    );
}

#[test]
fn single_shot_gre_known_values() {
    let r = results_for(SINGLE_SHOT_GRE);

    assert_close(measured(&r, "metrics.flip_angle"), 90.0, 1e-6, "flip");
    assert_close(measured(&r, "metrics.scan_time"), 80e-6, 1e-9, "scan time");
    assert_close(measured(&r, "metrics.n_slices"), 1.0, 0.0, "n_slices");
    assert_close(measured(&r, "metrics.te"), 41e-6, 1e-9, "TE");
    // Single excitation per slice: TR falls back to the whole-sequence duration.
    assert_close(measured(&r, "metrics.tr"), 80e-6, 1e-9, "TR");
    // Single echo: no echo spacing to report.
    assert_eq!(get(&r, "metrics.echo_spacing").status, Status::Skip);
}

#[test]
fn effective_te_is_the_k_centre_echo_not_the_first() {
    let r = results_for(MULTI_ECHO);

    // The central ky line is the *second* echo (311 µs), not the first (141 µs).
    assert_close(measured(&r, "metrics.te"), 311e-6, 1e-9, "effective TE");
    assert!(
        measured(&r, "metrics.te") > 200e-6,
        "effective TE must be the mid-train k-centre echo, not the first echo"
    );
    assert_close(
        measured(&r, "metrics.echo_spacing"),
        170e-6,
        1e-9,
        "echo spacing",
    );
}

#[test]
fn multi_slice_tr_and_n_slices() {
    let r = results_for(MULTI_SLICE);

    assert_close(measured(&r, "metrics.n_slices"), 2.0, 0.0, "n_slices");
    assert_close(measured(&r, "metrics.tr"), 200e-6, 1e-9, "TR");
    // No readout ⇒ TE / echo spacing unmeasurable, reported as skips.
    assert_eq!(get(&r, "metrics.te").status, Status::Skip);
    assert_eq!(get(&r, "metrics.echo_spacing").status, Status::Skip);
}

#[test]
fn no_excitation_skips_everything_but_scan_time() {
    let r = results_for(NO_RF);

    // Scan time is always measurable.
    assert_eq!(get(&r, "metrics.scan_time").status, Status::Pass);
    assert_close(measured(&r, "metrics.scan_time"), 500e-6, 1e-9, "scan time");
    // Everything that needs an excitation is a first-class skip, never a failure.
    for id in [
        "metrics.tr",
        "metrics.te",
        "metrics.flip_angle",
        "metrics.n_slices",
        "metrics.echo_spacing",
    ] {
        assert_eq!(get(&r, id).status, Status::Skip, "{id} should skip");
    }
}

/// Criterion 1: the example file's metrics are sane and pinned. These values
/// are the documented regression baseline for the example.
#[test]
fn example_metrics_are_sane_and_pinned() {
    let r = results_for_fixture("t1_spgr_axial_brain");

    assert_close(
        measured(&r, "metrics.scan_time"),
        76.809_216,
        1e-5,
        "scan time",
    );
    assert_close(measured(&r, "metrics.n_slices"), 44.0, 0.0, "n_slices");
    assert_close(measured(&r, "metrics.tr"), 0.400_048, 1e-5, "TR");
    assert_close(measured(&r, "metrics.te"), 0.004_008, 1e-5, "TE");
    assert_close(measured(&r, "metrics.flip_angle"), 80.0, 1e-3, "flip");
    // A single-echo SPGR ⇒ no echo spacing.
    assert_eq!(get(&r, "metrics.echo_spacing").status, Status::Skip);
}

/// Criterion 3: on real echo-train fixtures the effective TE is the mid-train
/// k-space-centre echo (many echo-spacings in), not the first echo.
#[test]
fn echo_train_fixtures_pick_the_mid_train_k_centre_echo() {
    // HASTE: single-shot, long train. Effective TE 108 ms, ESP 12 ms — the
    // k-centre echo is the 9th, far past the first.
    let haste = results_for_fixture("haste");
    let (te, esp) = (
        measured(&haste, "metrics.te"),
        measured(&haste, "metrics.echo_spacing"),
    );
    assert_close(te, 0.108, 1e-4, "HASTE effective TE");
    assert_close(esp, 0.012, 1e-4, "HASTE echo spacing");
    assert!(
        te > 3.0 * esp,
        "HASTE TE must be mid-train, not the first echo"
    );

    // PROPELLER (rotated TSE blades): effective TE 84 ms, ESP 14 ms — the
    // k-centre echo is the 6th. This only comes out right because the
    // phase-encode area is measured in the logical (pre-rotation) frame.
    let prop = results_for_fixture("propeller-fse-axial");
    let (te, esp) = (
        measured(&prop, "metrics.te"),
        measured(&prop, "metrics.echo_spacing"),
    );
    assert_close(te, 0.083_998, 1e-4, "PROPELLER effective TE");
    assert_close(esp, 0.014, 1e-4, "PROPELLER echo spacing");
    assert!(
        te > 3.0 * esp,
        "PROPELLER TE must be mid-train, not the first echo"
    );
}

/// Metrics are measurements: across every parseable fixture, no `metrics.*`
/// result is ever a `fail` (or `warn`) — they are `pass` (measured) or `skip`.
#[test]
fn metrics_never_fail_or_warn() {
    for name in [
        "t1_spgr_axial_brain",
        "epi_rs",
        "haste",
        "propeller-fse-axial",
    ] {
        let r = results_for_fixture(name);
        for res in r.iter().filter(|res| res.id.starts_with("metrics.")) {
            assert!(
                matches!(res.status, Status::Pass | Status::Skip),
                "{name}: {} should be pass/skip, got {:?}",
                res.id,
                res.status
            );
        }
    }
}
