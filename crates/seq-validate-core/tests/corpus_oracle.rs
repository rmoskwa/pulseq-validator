//! The generated-corpus oracle for the derived metrics.
//!
//! Two-sided ground truth for every derived metric. For each committed corpus
//! sequence under `corpus/data/` the validator's measurements are checked against:
//!
//!   1. **the generated inputs** (`<name>.params.json`) — recover-the-inputs: the
//!      known parameters the MATLAB `mr`-toolbox generator was given, and
//!   2. **Pulseq's own self-report** (`<name>.report.json`) — the TE / TR /
//!      duration `seq.testReport()` measures independently from k-space.
//!
//! The artifacts are produced (and regenerated) by `corpus/matlab/generate_corpus.m`;
//! MATLAB is **not** needed here — CI runs the Rust validator against the
//! committed files. A `null` field is an explicit "not comparable for this
//! family" (e.g. EPI's per-slice TR, or a multi-echo's ambiguous testReport TE)
//! and is skipped — see the generator's notes.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fs;
use std::path::{Path, PathBuf};

use seq_validate_core::checks::run_all;
use seq_validate_core::serde_json::{self, Value};
use seq_validate_core::{CheckCtx, CheckResult, Measurements, Sequence, Status};

/// `(metric id, sidecar field, tolerance)`. Tolerance bands in SI units:
/// TE/TR/echo-spacing to 0.1 ms, flip to
/// 0.05°, slice count exact, scan time to 1 ms (testReport prints 6 decimals).
const FIELDS: &[(&str, &str, f64)] = &[
    ("metrics.te", "te_s", 1e-4),
    ("metrics.tr", "tr_s", 1e-4),
    ("metrics.flip_angle", "flip_deg", 0.05),
    ("metrics.n_slices", "n_slices", 0.0),
    ("metrics.echo_spacing", "echo_spacing_s", 1e-4),
    ("metrics.scan_time", "scan_time_s", 1e-3),
];

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/data")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// A numeric sidecar field, or `None` when absent / JSON `null` (an explicit
/// not-comparable marker).
fn field(v: &Value, key: &str) -> Option<f64> {
    v.get(key).filter(|x| !x.is_null()).and_then(Value::as_f64)
}

#[test]
fn corpus_recovers_inputs_and_matches_self_report() {
    let dir = corpus_dir();
    let mut seqs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir {dir:?}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "seq"))
        .collect();
    seqs.sort();
    assert!(!seqs.is_empty(), "no corpus sequences found in {dir:?}");

    let mut failures: Vec<String> = Vec::new();
    let (mut generated_checks, mut oracle_checks) = (0u32, 0u32);

    for seqf in &seqs {
        let stem = seqf
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf-8 stem");
        let seq = Sequence::from_file(seqf).unwrap_or_else(|e| panic!("{stem} must parse: {e}"));
        let results = run_all(&CheckCtx {
            seq: &seq,
            profile: None,
        });

        // Metrics are measurements: none may ever fail.
        for r in results.iter().filter(|r| r.id.starts_with("metrics.")) {
            assert_ne!(
                r.status,
                Status::Fail,
                "{stem}: {} unexpectedly failed",
                r.id
            );
        }

        let params = read_json(&seqf.with_extension("params.json"));
        let report = read_json(&seqf.with_extension("report.json"));
        let measured = |id: &str| -> Option<f64> {
            results
                .iter()
                .find(|r| r.id == id)
                .and_then(|r| r.measured.as_ref())
                .and_then(Value::as_f64)
        };

        for &(id, key, tol) in FIELDS {
            let mv = measured(id);
            for (label, src) in [("generated", &params), ("self-report", &report)] {
                let Some(target) = field(src, key) else {
                    continue;
                };
                if label == "generated" {
                    generated_checks += 1;
                } else {
                    oracle_checks += 1;
                }
                match mv {
                    None => failures.push(format!(
                        "{stem}: {id} vs {label}: expected {target}, but the validator did not measure it"
                    )),
                    Some(m) => {
                        let ok = if tol == 0.0 { m == target } else { (m - target).abs() <= tol };
                        if !ok {
                            failures.push(format!(
                                "{stem}: {id} vs {label}: measured {m}, expected {target} (±{tol})"
                            ));
                        }
                    }
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} corpus oracle mismatch(es):\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Sanity: both sides of the oracle actually exercised something.
    assert!(generated_checks > 0 && oracle_checks > 0);
    eprintln!(
        "[corpus] {} sequences, {generated_checks} recover-the-inputs + {oracle_checks} self-report checks passed",
        seqs.len()
    );
}

// ---------------------------------------------------------------------------
// Dual-witness geometry.
// ---------------------------------------------------------------------------

fn result<'a>(results: &'a [CheckResult], id: &str) -> Option<&'a CheckResult> {
    results.iter().find(|r| r.id == id)
}

fn status_of(results: &[CheckResult], id: &str) -> Option<Status> {
    result(results, id).map(|r| r.status)
}

fn i64_array(v: &Value, key: &str) -> Vec<i64> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn f64_array(v: &Value, key: &str) -> Vec<f64> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

/// The dual-witness geometry, checked against the known generation inputs
/// (`matrix` / `fov_mm` in `params.json`).
///
/// Two families of assertion:
///
///   1. **Cartesian families** (one ADC per excitation — `echo_spacing_s: null`):
///      the param-algebra witness (`metrics.matrix` / `metrics.fov`) recovers the
///      generated matrix exactly and FOV within tolerance, AND the trajectory
///      witness agrees with it (`trajectory.geometry_agreement: pass`).
///   2. **Echo-train families** (EPI/mGRE — `echo_spacing_s` set): the param-algebra
///      `skip`s (the single-line Cartesian model does not apply) and the
///      trajectory gate still measures the geometry — its phase-encode count and
///      the 2D-vs-3D dimensionality match the generated inputs.
#[test]
fn corpus_geometry_dual_witness() {
    let dir = corpus_dir();
    let mut seqs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir {dir:?}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "seq"))
        .collect();
    seqs.sort();
    assert!(!seqs.is_empty(), "no corpus sequences found in {dir:?}");

    let mut failures: Vec<String> = Vec::new();
    let (mut cartesian_checked, mut echotrain_checked) = (0u32, 0u32);

    for seqf in &seqs {
        let stem = seqf
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf-8 stem");
        let seq = Sequence::from_file(seqf).unwrap_or_else(|e| panic!("{stem} must parse: {e}"));
        let results = run_all(&CheckCtx {
            seq: &seq,
            profile: None,
        });
        let m = Measurements::from_results(&results);
        let params = read_json(&seqf.with_extension("params.json"));

        let matrix = i64_array(&params, "matrix"); // [x, y, z]
        let fov = f64_array(&params, "fov_mm"); // [x, y, z] mm
        assert_eq!(matrix.len(), 3, "{stem}: params.matrix must be [x,y,z]");
        assert_eq!(fov.len(), 3, "{stem}: params.fov_mm must be [x,y,z]");
        let is_3d = matrix[2] > 1;
        // Single-readout Cartesian model ⟺ not an echo train ⟺ no echo spacing.
        let cartesian = field(&params, "echo_spacing_s").is_none();

        // No geometry result may ever fail.
        for r in results.iter().filter(|r| {
            r.id.starts_with("trajectory.") || r.id == "metrics.fov" || r.id == "metrics.matrix"
        }) {
            if r.status == Status::Fail {
                failures.push(format!(
                    "{stem}: {} unexpectedly failed: {}",
                    r.id, r.message
                ));
            }
        }

        // Dimensionality headline: 3D iff a partition axis is encoded.
        let want_dims = if is_3d { 3.0 } else { 2.0 };
        match m.dimensionality {
            Some(d) if d == want_dims => {}
            other => failures.push(format!(
                "{stem}: trajectory.dimensionality measured {other:?}, expected {want_dims}"
            )),
        }

        let m_status = status_of(&results, "metrics.matrix");
        let agree = status_of(&results, "trajectory.geometry_agreement");

        if cartesian {
            cartesian_checked += 1;
            // Param-algebra applies and recovers the generated matrix exactly.
            if m_status != Some(Status::Pass) {
                failures.push(format!(
                    "{stem}: metrics.matrix should be `pass` (Cartesian), got {m_status:?}"
                ));
            } else {
                #[allow(clippy::cast_possible_truncation)] // matrix counts are small, exact in f64
                let meas: Vec<i64> = m
                    .matrix
                    .param
                    .as_ref()
                    .map(|a| a.iter().filter_map(|x| x.map(|v| v as i64)).collect())
                    .unwrap_or_default();
                if meas != matrix {
                    failures.push(format!(
                        "{stem}: metrics.matrix measured {meas:?}, expected {matrix:?}"
                    ));
                }
            }
            // FOV in-plane (and through-plane when 3D) within 2%.
            let n_axes = if is_3d { 3 } else { 2 };
            let meas_fov = m.fov.param.as_ref();
            for (axis, &want) in fov.iter().enumerate().take(n_axes) {
                let mv = meas_fov.and_then(|a| a.get(axis)).copied().flatten();
                match mv {
                    Some(m) if (m - want).abs() <= 0.02 * want.abs() => {}
                    other => failures.push(format!(
                        "{stem}: metrics.fov[{axis}] measured {other:?}, expected {want} mm (±2%)"
                    )),
                }
            }
            // The two witnesses must agree.
            if agree != Some(Status::Pass) {
                failures.push(format!(
                    "{stem}: trajectory.geometry_agreement should be `pass`, got {agree:?}"
                ));
            }
        } else {
            echotrain_checked += 1;
            // Param-algebra defers; the trajectory gate owns geometry.
            if m_status != Some(Status::Skip) {
                failures.push(format!(
                    "{stem}: metrics.matrix should `skip` for an echo train, got {m_status:?}"
                ));
            }
            if agree != Some(Status::Skip) {
                failures.push(format!(
                    "{stem}: trajectory.geometry_agreement should `skip` (one witness), got {agree:?}"
                ));
            }
            // The trajectory still recovers the phase-encode count (ky blips are a
            // clean grid even when the readout is not).
            #[allow(clippy::cast_possible_truncation)] // phase-encode count is small, exact in f64
            let ky = m
                .matrix
                .trajectory
                .as_ref()
                .and_then(|a| a.get(1))
                .copied()
                .flatten()
                .map(|v| v as i64);
            if ky != Some(matrix[1]) {
                failures.push(format!(
                    "{stem}: trajectory matrix_y measured {ky:?}, expected {} (phase-encode lines)",
                    matrix[1]
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} dual-witness geometry mismatch(es):\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        cartesian_checked > 0 && echotrain_checked > 0,
        "expected both Cartesian and echo-train families in the corpus"
    );
    eprintln!(
        "[corpus geometry] {cartesian_checked} Cartesian (dual-witness agree) + \
         {echotrain_checked} echo-train (trajectory-only) families verified"
    );
}
