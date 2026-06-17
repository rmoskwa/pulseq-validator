//! The [`Report`] — per-check results plus sequence metadata — and its **stable
//! JSON** serialization, the project's integration contract.
//!
//! A `Report` is what `seq-validate` emits, in both the human and `--json`
//! forms. The JSON shape is versioned by [`SCHEMA_VERSION`] and pinned by the
//! schema document at `schema/report-v1.schema.json`; treat any change to the
//! field set as breaking, and bump the version when it happens.
//!
//! One `Report` type covers both outcomes so `--json` consumers always parse the
//! same schema:
//! - **success** — [`for_sequence`](Report::for_sequence): `error` is null,
//!   `sequence` is populated, `results` carries the checks.
//! - **harness/parse error** — [`harness_error`](Report::harness_error): `error`
//!   holds the message, `sequence` is null, `results` is empty.
//!
//! [`exit_code`](Report::exit_code) encodes the policy: `2` on harness error,
//! `1` on any `fail`, else `0` (`warn`/`skip` never fail the run).

use serde::{Deserialize, Serialize};

use crate::ir::Sequence;
use crate::result::{CheckResult, Status};

/// Version of the JSON report schema. Bumped on any breaking change to the
/// emitted shape; downstream consumers should pin it. Mirrors the `const` in
/// `schema/report-v1.schema.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Parsed-sequence metadata surfaced alongside the check results. Drawn from the
/// IR (`pulseq_version`, `name`) and parse stats (`blocks`, `duration_s`,
/// interpreter `parse_warnings`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SequenceMeta {
    /// Pulseq file-format version, e.g. `"1.5.1"`.
    pub pulseq_version: String,
    /// The `Name` definition, if the file declared one.
    pub name: Option<String>,
    /// Number of interpreted blocks.
    pub blocks: usize,
    /// Total sequence duration, in seconds.
    pub duration_s: f64,
    /// Non-fatal interpreter warnings raised while lowering to the IR.
    pub parse_warnings: Vec<String>,
}

impl SequenceMeta {
    /// Project the metadata out of an interpreted [`Sequence`].
    pub fn from_sequence(seq: &Sequence) -> Self {
        SequenceMeta {
            pulseq_version: seq.version.to_string(),
            name: seq.name.clone(),
            blocks: seq.blocks.len(),
            duration_s: seq.total_duration,
            parse_warnings: seq.warnings.clone(),
        }
    }
}

/// Tally of results by [`Status`]. Redundant with `results` but cheap, and it
/// saves every consumer from recomputing it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Total number of results.
    pub total: usize,
    pub pass: usize,
    pub fail: usize,
    pub warn: usize,
    pub skip: usize,
}

impl Summary {
    /// Tally a slice of results.
    pub fn of(results: &[CheckResult]) -> Summary {
        let mut s = Summary {
            total: results.len(),
            ..Summary::default()
        };
        for r in results {
            match r.status {
                Status::Pass => s.pass += 1,
                Status::Fail => s.fail += 1,
                Status::Warn => s.warn += 1,
                Status::Skip => s.skip += 1,
            }
        }
        s
    }
}

/// A complete validation report for one `.seq` file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Report {
    /// JSON schema version of this payload; see [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Input path, exactly as given on the CLI.
    pub file: String,
    /// Harness/parse error message; `null` on success. When set, `sequence` is
    /// `null` and [`exit_code`](Report::exit_code) is `2`.
    pub error: Option<String>,
    /// Parsed-sequence metadata; `null` iff `error` is set.
    pub sequence: Option<SequenceMeta>,
    /// Per-check results, in the order checks were run.
    pub results: Vec<CheckResult>,
    /// Status tally over `results`.
    pub summary: Summary,
}

impl Report {
    /// Build a success report from an interpreted sequence and its check results.
    pub fn for_sequence(
        file: impl Into<String>,
        seq: &Sequence,
        results: Vec<CheckResult>,
    ) -> Self {
        Self::new(file, SequenceMeta::from_sequence(seq), results)
    }

    /// Build a success report from already-computed sequence metadata and check
    /// results. A success report always carries its sequence, so this is how
    /// [`for_sequence`](Report::for_sequence) is built; a harness/parse failure
    /// (no sequence) goes through [`harness_error`](Report::harness_error)
    /// instead. Requiring the metadata here makes the
    /// "`sequence` is null iff `error` is set" contract unrepresentable to
    /// violate. Also handy for tests.
    pub fn new(file: impl Into<String>, sequence: SequenceMeta, results: Vec<CheckResult>) -> Self {
        let summary = Summary::of(&results);
        Report {
            schema_version: SCHEMA_VERSION,
            file: file.into(),
            error: None,
            sequence: Some(sequence),
            results,
            summary,
        }
    }

    /// Build a harness/parse-error report: no sequence, no results, the message
    /// recorded in `error`. Exits `2`.
    pub fn harness_error(file: impl Into<String>, message: impl Into<String>) -> Self {
        Report {
            schema_version: SCHEMA_VERSION,
            file: file.into(),
            error: Some(message.into()),
            sequence: None,
            results: Vec::new(),
            summary: Summary::default(),
        }
    }

    /// Whether any result is a [`Status::Fail`].
    pub fn has_failures(&self) -> bool {
        self.summary.fail > 0
    }

    /// The process exit code per policy: `2` on a harness/parse error, `1` on
    /// any `fail`, else `0`. `warn` and `skip` never fail the run.
    pub fn exit_code(&self) -> i32 {
        if self.error.is_some() {
            2
        } else if self.has_failures() {
            1
        } else {
            0
        }
    }

    /// Serialize to pretty, stable JSON (the integration contract).
    ///
    /// Panics only if a result carries a non-finite float (`NaN`/`Infinity`),
    /// which `serde_json` cannot represent; the IR never produces such values
    /// for a validly parsed sequence.
    #[allow(clippy::expect_used)] // invariant documented above; only non-finite floats can fail
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("Report is always JSON-serializable")
    }
}
