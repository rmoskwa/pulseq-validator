//! The JSON report is the integration contract, so
//! it gets contract-grade tests: every emitted report — success, empty, and
//! harness-error — validates against the published JSON Schema
//! (`schema/report-v1.schema.json`), round-trips through serde unchanged, and
//! carries the pinned schema version.
#![allow(clippy::expect_used, clippy::panic)] // test helpers intentionally panic on failure

use seq_validate_core::{CheckResult, Report, SCHEMA_VERSION, SequenceMeta, Severity};
use serde_json::Value;

const SCHEMA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/report-v1.schema.json");

fn schema() -> Value {
    let text = std::fs::read_to_string(SCHEMA_PATH).expect("schema file is readable");
    serde_json::from_str(&text).expect("schema file is valid JSON")
}

/// Validate a report's emitted JSON against the schema, panicking with every
/// violation if it fails.
fn assert_schema_valid(report: &Report) {
    let validator = jsonschema::validator_for(&schema()).expect("schema compiles");
    let instance: Value = serde_json::from_str(&report.to_json()).expect("report emits valid JSON");
    if !validator.is_valid(&instance) {
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|e| format!("  - {e}"))
            .collect();
        panic!(
            "emitted report violates report-v1.schema.json:\n{}\n--- payload ---\n{}",
            errors.join("\n"),
            report.to_json()
        );
    }
}

fn sample_meta() -> SequenceMeta {
    SequenceMeta {
        pulseq_version: "1.5.1".into(),
        name: Some("t1_spgr".into()),
        blocks: 50_688,
        duration_s: 76.809_216,
        parse_warnings: vec![],
    }
}

/// Results exercising every status, severity override, and measured/expected
/// combination — so the schema is tested against the full shape, not a subset.
fn sample_results() -> Vec<CheckResult> {
    vec![
        CheckResult::pass(
            "integrity.raster_alignment",
            "all blocks on the 4 µs raster",
        )
        .with_measured(4e-6),
        CheckResult::fail("integrity.block_timing", "block 3 duration is off-raster")
            .with_measured(3.9e-6)
            .with_expected(4e-6),
        CheckResult::warn("metrics.te", "TE is ambiguous for this multi-echo readout"),
        CheckResult::skip(
            "trajectory.fov_y",
            "single phase-encode line; FOV_y unmeasurable",
        ),
        CheckResult::pass("hardware.slew_rate", "within profile limit")
            .with_measured(180.0)
            .with_expected(200.0)
            .with_severity(Severity::Info),
    ]
}

#[test]
fn schema_file_compiles() {
    jsonschema::validator_for(&schema()).expect("report-v1.schema.json must be a valid schema");
}

#[test]
fn populated_report_is_schema_valid() {
    let report = Report::new("scan.seq", sample_meta(), sample_results());
    assert_schema_valid(&report);
}

#[test]
fn empty_report_is_schema_valid() {
    // A well-formed report with zero checks.
    let report = Report::new("scan.seq", sample_meta(), vec![]);
    assert_schema_valid(&report);
}

#[test]
fn harness_error_report_is_schema_valid() {
    let report = Report::harness_error("broken.seq", "malformed .seq: missing [VERSION] section");
    assert_schema_valid(&report);
}

#[test]
fn report_round_trips_through_serde() {
    let report = Report::new("scan.seq", sample_meta(), sample_results());
    let back: Report = serde_json::from_str(&report.to_json()).expect("report deserializes back");
    assert_eq!(back, report, "report did not round-trip unchanged");
}

#[test]
fn schema_version_is_pinned_and_consistent() {
    // The payload, the public constant, and the schema's `const` all agree.
    let report = Report::new("scan.seq", sample_meta(), vec![]);
    let emitted: Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(
        emitted["schema_version"].as_u64(),
        Some(SCHEMA_VERSION as u64)
    );
    assert_eq!(
        schema()["properties"]["schema_version"]["const"].as_u64(),
        Some(SCHEMA_VERSION as u64),
        "schema const is out of sync with SCHEMA_VERSION"
    );
}
