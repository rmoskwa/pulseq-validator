//! The result model and exit-code policy, against
//! synthetic results: `0` if no `fail`, `1` on any `fail`, `2` on a
//! harness/parse error; `warn`/`skip` never fail the run.

use seq_validate_core::{CheckResult, Report, SequenceMeta, Severity, Status, Summary};

/// A minimal success-report sequence stub. These tests assert only exit-code,
/// `has_failures`, and summary behavior — none read the sequence — but a success
/// report always carries one, so we hand `Report::new` a placeholder.
fn meta() -> SequenceMeta {
    SequenceMeta {
        pulseq_version: "1.5.1".into(),
        name: None,
        blocks: 0,
        duration_s: 0.0,
        parse_warnings: vec![],
    }
}

fn no_fail_results() -> Vec<CheckResult> {
    vec![
        CheckResult::pass("integrity.raster_alignment", "ok"),
        CheckResult::warn("metrics.te", "ambiguous"),
        CheckResult::skip("trajectory.fov_y", "n/a"),
    ]
}

#[test]
fn exit_zero_when_no_failures() {
    let report = Report::new("f.seq", meta(), no_fail_results());
    assert!(!report.has_failures());
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn exit_one_on_any_failure() {
    let mut results = no_fail_results();
    results.push(CheckResult::fail("integrity.block_timing", "off-raster"));
    let report = Report::new("f.seq", meta(), results);
    assert!(report.has_failures());
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn exit_two_on_harness_error() {
    let report = Report::harness_error("f.seq", "cannot parse");
    assert_eq!(report.exit_code(), 2);
    assert!(report.error.is_some());
    assert!(report.sequence.is_none());
    assert_eq!(report.summary, Summary::default());
}

#[test]
fn warn_and_skip_alone_do_not_fail() {
    let report = Report::new(
        "f.seq",
        meta(),
        vec![
            CheckResult::warn("metrics.te", "w"),
            CheckResult::skip("trajectory.fov_y", "s"),
        ],
    );
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn summary_tallies_each_status() {
    let report = Report::new(
        "f.seq",
        meta(),
        vec![
            CheckResult::pass("metrics.tr", "x"),
            CheckResult::pass("metrics.te", "x"),
            CheckResult::fail("integrity.block_timing", "x"),
            CheckResult::warn("metrics.flip", "x"),
            CheckResult::skip("trajectory.fov_y", "x"),
        ],
    );
    assert_eq!(
        report.summary,
        Summary {
            total: 5,
            pass: 2,
            fail: 1,
            warn: 1,
            skip: 1
        }
    );
}

#[test]
fn builders_set_status_and_default_severity() {
    let pass = CheckResult::pass("a.b", "m");
    assert_eq!(pass.status, Status::Pass);
    assert_eq!(pass.severity, Severity::Info);

    let fail = CheckResult::fail("a.b", "m");
    assert_eq!(fail.status, Status::Fail);
    assert_eq!(fail.severity, Severity::Error);

    let warn = CheckResult::warn("a.b", "m");
    assert_eq!(warn.status, Status::Warn);
    assert_eq!(warn.severity, Severity::Warn);

    let skip = CheckResult::skip("a.b", "m");
    assert_eq!(skip.status, Status::Skip);
    assert_eq!(skip.severity, Severity::Info);
}

#[test]
fn measured_expected_and_severity_overrides_attach() {
    let r = CheckResult::fail("hardware.slew_rate", "too fast")
        .with_measured(220.0)
        .with_expected(200.0)
        .with_severity(Severity::Warn);
    assert_eq!(r.measured, Some(serde_json::json!(220.0)));
    assert_eq!(r.expected, Some(serde_json::json!(200.0)));
    assert_eq!(r.severity, Severity::Warn);
}
