//! The human renderer (docs/02-crate-skeleton.md): grouped by category,
//! colorized, and well-formed for the zero-checks case.

use seq_validate_core::{CheckResult, Report, SequenceMeta, render};

fn meta() -> SequenceMeta {
    SequenceMeta {
        pulseq_version: "1.5.1".into(),
        name: Some("t1_spgr".into()),
        blocks: 42,
        duration_s: 1.5,
        parse_warnings: vec![],
    }
}

#[test]
fn empty_report_is_well_formed_plain_text() {
    let report = Report::new("scan.seq", meta(), vec![]);
    let out = render(&report, false, false);

    assert!(out.contains("scan.seq"), "header file");
    assert!(out.contains("Pulseq 1.5.1 · t1_spgr"), "sequence identity");
    assert!(out.contains("42 blocks · 1.500 s"), "parse stats");
    assert!(out.contains("No checks run."), "zero-checks line");
    assert!(
        out.contains("Summary: 0 passed, 0 failed, 0 warnings, 0 skipped"),
        "summary line"
    );
    assert!(!out.contains('\u{1b}'), "no ANSI escapes when color is off");
}

fn three_result_report() -> Report {
    Report::new(
        "scan.seq",
        meta(),
        vec![
            // Deliberately out of display order to prove the renderer sorts.
            CheckResult::fail("hardware.slew_rate", "too fast")
                .with_measured(220.0)
                .with_expected(200.0),
            CheckResult::skip("trajectory.fov_y", "single line"),
            CheckResult::pass("integrity.raster_alignment", "aligned"),
        ],
    )
}

#[test]
fn results_group_by_category_in_display_order() {
    let report = three_result_report();
    // Verbose so the measured/expected disclosure is exercised here too.
    let out = render(&report, false, true);

    let integrity = out.find("Sequence integrity").expect("integrity heading");
    let trajectory = out.find("K-space trajectory").expect("trajectory heading");
    let hardware = out.find("Hardware & safety").expect("hardware heading");
    assert!(
        integrity < trajectory && trajectory < hardware,
        "categories must print in DISPLAY_ORDER regardless of result order"
    );

    assert!(out.contains("PASS"));
    assert!(out.contains("FAIL"));
    assert!(out.contains("SKIP"));
    assert!(out.contains("integrity.raster_alignment"));
    assert!(out.contains("measured=220.0"));
    assert!(out.contains("expected=200.0"));
    assert!(out.contains("Summary: 1 passed, 1 failed, 0 warnings, 1 skipped"));
}

#[test]
fn default_human_report_hides_measured_and_expected() {
    // Without --verbose the prose message stands alone; the structured data is
    // reserved for the JSON form (the integration contract).
    let report = three_result_report();
    let out = render(&report, false, false);

    assert!(
        out.contains("too fast"),
        "the prose message still renders: {out}"
    );
    assert!(
        !out.contains("measured=") && !out.contains("expected="),
        "the structured data block is suppressed by default: {out}"
    );
}

#[test]
fn color_mode_emits_ansi_escapes() {
    let report = Report::new(
        "scan.seq",
        meta(),
        vec![CheckResult::pass("integrity.raster_alignment", "ok")],
    );
    let out = render(&report, true, false);
    assert!(
        out.contains('\u{1b}'),
        "expected ANSI escapes with color on"
    );
}

#[test]
fn harness_error_renders_the_message() {
    let report = Report::harness_error("broken.seq", "missing [VERSION] section");
    let out = render(&report, false, false);
    assert!(out.contains("broken.seq"));
    assert!(out.contains("error: missing [VERSION] section"));
    // No sequence stats and no check groups on a harness error.
    assert!(!out.contains("No checks run."));
}
