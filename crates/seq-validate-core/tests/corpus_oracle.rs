//! The generated-corpus oracle harness (`docs/04-derived-metrics.md`).
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
use seq_validate_core::{CheckCtx, Sequence, Status};

/// `(metric id, sidecar field, tolerance)`. Tolerances follow the harness
/// `param_check.py` bands, in SI units: TE/TR/echo-spacing to 0.1 ms, flip to
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
        let results = run_all(&CheckCtx { seq: &seq });

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
