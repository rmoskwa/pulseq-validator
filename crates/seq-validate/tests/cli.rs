//! End-to-end CLI acceptance: `seq-validate` runs to completion on the bundled
//! v1.5.1 example, the human and `--json` forms are well-formed and carry the
//! integrity results (which all pass on the clean example), and the
//! exit-code policy holds (0 on success, 2 on a harness/parse error).
#![allow(clippy::expect_used)] // test helper `run` intentionally panics on failure

use std::process::Command;

use seq_validate_core::serde_json::{self, Value};

const BIN: &str = env!("CARGO_BIN_EXE_seq-validate");
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/t1_spgr_axial_brain.seq"
);

/// Run the CLI and return `(exit_code, stdout, stderr)`. Color is suppressed via
/// `NO_COLOR` so stdout assertions are stable regardless of the test TTY.
fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN)
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("spawn seq-validate");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn human_report_on_example_runs_integrity_checks() {
    let (code, stdout, _) = run(&[FIXTURE]);
    assert_eq!(code, 0, "all integrity checks pass on the example → exit 0");
    assert!(stdout.contains("Pulseq 1.5.1"), "stdout: {stdout}");
    assert!(stdout.contains("50688 blocks"), "stdout: {stdout}");
    // The integrity section and its checks render from the registry.
    assert!(stdout.contains("Sequence integrity"), "stdout: {stdout}");
    assert!(
        stdout.contains("integrity.raster_alignment"),
        "stdout: {stdout}"
    );
    assert!(
        !stdout.contains("No checks run."),
        "checks now run: {stdout}"
    );
    // The example is clean: no failures or warnings, whatever the pass/skip mix.
    assert!(
        stdout.contains("0 failed, 0 warnings"),
        "example is clean: {stdout}"
    );
}

#[test]
fn json_report_includes_integrity_results() {
    let (code, stdout, _) = run(&[FIXTURE, "--json"]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&stdout).expect("--json emits valid JSON");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["error"], Value::Null);
    assert_eq!(v["sequence"]["pulseq_version"], "1.5.1");
    assert!(v["sequence"]["blocks"].as_u64().unwrap() > 0);

    let results = v["results"].as_array().expect("results is an array");
    assert!(!results.is_empty(), "integrity checks now produce results");
    assert!(
        results.iter().any(|r| r["id"] == "integrity.signature"),
        "expected an integrity.signature result among: {results:#?}"
    );
    assert_eq!(v["summary"]["fail"], 0, "the example has no failures");
    assert_eq!(v["summary"]["total"], results.len());
}

#[test]
fn missing_file_is_harness_error_exit_two() {
    let (code, _, _) = run(&["definitely-does-not-exist.seq"]);
    assert_eq!(code, 2, "a parse/IO failure is exit 2, not 1");
}

#[test]
fn garbage_file_is_parse_error_with_uniform_json() {
    let path = format!("{}/garbage.seq", env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(&path, "this is not a pulseq file\n").unwrap();

    let (code, stdout, _) = run(&[&path, "--json"]);
    assert_eq!(code, 2);
    // Even on a harness error, --json emits the same schema (error set, no sequence).
    let v: Value = serde_json::from_str(&stdout).expect("valid JSON on error too");
    assert_eq!(v["schema_version"], 1);
    assert!(v["error"].is_string());
    assert_eq!(v["sequence"], Value::Null);
}

const SPEC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/t1_spgr_axial_brain.spec.yaml"
);

#[test]
fn matching_spec_passes_with_a_spec_section() {
    // The committed spec matches the example within tolerance → exit 0, and the
    // Spec-assertions section renders with `spec.*` results.
    let (code, stdout, _) = run(&[FIXTURE, "--spec", SPEC]);
    assert_eq!(code, 0, "a matching spec passes: {stdout}");
    assert!(stdout.contains("Spec assertions"), "stdout: {stdout}");
    assert!(stdout.contains("spec.te_ms"), "stdout: {stdout}");
    // Oversampling is divided out: physical matrix_x 384 → nominal 192.
    assert!(
        stdout.contains("spec.matrix_x") && stdout.contains("matches expected 192"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("0 failed"), "stdout: {stdout}");
}

#[test]
fn perturbing_one_field_fails_exactly_that_check() {
    // A spec with one out-of-tolerance field fails exactly that field → exit 1.
    let path = format!("{}/perturbed.yaml", env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(&path, "te_ms: 99.0\ntr_ms: 400.048\n").unwrap();
    let (code, stdout, _) = run(&[FIXTURE, "--spec", &path]);
    assert_eq!(code, 1, "an out-of-tolerance field is a fail → exit 1");
    assert!(
        stdout.contains("FAIL  spec.te_ms"),
        "the perturbed field fails: {stdout}"
    );
    assert!(
        stdout.contains("PASS  spec.tr_ms"),
        "the in-tolerance field still passes: {stdout}"
    );
    assert!(stdout.contains("1 failed"), "exactly one fail: {stdout}");
}

#[test]
fn omitted_spec_fields_are_silently_not_checked() {
    // Lenient policy: only the provided field is asserted; no error for the rest.
    let path = format!("{}/single.yaml", env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(&path, "flip_angle_deg: 80\n").unwrap();
    let (code, stdout, _) = run(&[FIXTURE, "--spec", &path, "--json"]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&stdout).expect("valid JSON");
    let spec_ids: Vec<&str> = v["results"]
        .as_array()
        .expect("results")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .filter(|id| id.starts_with("spec."))
        .collect();
    assert_eq!(
        spec_ids,
        vec!["spec.flip_angle_deg"],
        "only the provided field becomes a spec check: {spec_ids:?}"
    );
}

#[test]
fn missing_spec_file_is_a_harness_error_exit_two() {
    let (code, _, _) = run(&[FIXTURE, "--spec", "definitely-no-such-spec.yaml"]);
    assert_eq!(code, 2, "an unreadable spec is a harness error, not a fail");
    let (_, stdout, _) = run(&[FIXTURE, "--spec", "definitely-no-such-spec.yaml", "--json"]);
    let v: Value = serde_json::from_str(&stdout).expect("error report is still valid JSON");
    assert!(
        v["error"].as_str().unwrap_or("").contains("spec"),
        "the error names the spec: {stdout}"
    );
}

#[test]
fn spec_scanner_field_selects_the_profile() {
    // With no --profile, the spec's `scanner` drives the hardware checks.
    let path = format!("{}/scanner.yaml", env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(&path, "scanner: ge-premier\nte_ms: 4.008\n").unwrap();
    let (code, stdout, _) = run(&[FIXTURE, "--spec", &path]);
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(
        stdout.contains("hardware.profile") && stdout.contains("ge-premier"),
        "the spec's scanner selected the profile: {stdout}"
    );
}

#[test]
fn profile_selects_scanner_and_runs_hardware_checks() {
    // The example targets GE rasters (grad/block 4 µs, rf/adc 2 µs) and passes the
    // ge-premier hardware limits → exit 0 with the Hardware section populated.
    let (code, stdout, _) = run(&[FIXTURE, "--profile", "ge-premier"]);
    assert_eq!(code, 0, "the example passes ge-premier: {stdout}");
    assert!(stdout.contains("Hardware & safety"), "stdout: {stdout}");
    assert!(
        stdout.contains("hardware.profile") && stdout.contains("ge-premier"),
        "the resolved profile is named in the report: {stdout}"
    );
    assert!(stdout.contains("hardware.slew_rate"), "stdout: {stdout}");
    // No hardware failures or warnings on the clean, matching example.
    assert!(stdout.contains("0 failed, 0 warnings"), "stdout: {stdout}");
}

#[test]
fn unknown_profile_is_a_clear_error_exit_two() {
    let (code, _, _) = run(&[FIXTURE, "--profile", "no-such-scanner"]);
    assert_eq!(
        code, 2,
        "an unknown profile is an error, not a silent fallback"
    );
    let (_, stdout, _) = run(&[FIXTURE, "--profile", "no-such-scanner", "--json"]);
    let v: Value = serde_json::from_str(&stdout).expect("error report is still valid JSON");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or("")
            .contains("unknown scanner profile"),
        "error names the bad profile: {stdout}"
    );
}

#[test]
fn no_profile_skips_hardware_non_silently() {
    // File-only mode: hardware checks skip with a clear, non-silent message and
    // the run still succeeds (exit 0, no fail/warn introduced).
    let (code, stdout, _) = run(&[FIXTURE]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("hardware.profile") && stdout.contains("no scanner profile"),
        "the no-profile outcome is visible, not silent: {stdout}"
    );
}

#[test]
fn set_override_can_drive_a_hardware_fail() {
    // Tightening maxSlew below the sequence's peak slew turns the slew check red.
    let (code, stdout, _) = run(&[FIXTURE, "--profile", "ge-premier", "--set", "maxSlew=100"]);
    assert_eq!(
        code, 1,
        "an override-induced limit breach is a fail → exit 1"
    );
    assert!(
        stdout.contains("hardware.slew_rate") && stdout.contains("exceeds maxSlew 100"),
        "the fail names the offending value and the overridden limit: {stdout}"
    );
}

#[test]
fn set_non_finite_override_is_a_clear_error_not_a_silent_disable() {
    // `nan` (and overflow → `inf`) parse as f64 but would make the limit vacuously
    // pass; the override must be rejected as a harness error (exit 2), never used.
    let (code, _, _) = run(&[FIXTURE, "--profile", "ge-premier", "--set", "maxGrad=nan"]);
    assert_eq!(
        code, 2,
        "a non-finite override is an error, not a silently-disabled check"
    );
    let (_, stdout, _) = run(&[
        FIXTURE,
        "--profile",
        "ge-premier",
        "--set",
        "maxGrad=nan",
        "--json",
    ]);
    let v: Value = serde_json::from_str(&stdout).expect("error report is still valid JSON");
    assert!(
        v["error"].as_str().unwrap_or("").contains("finite"),
        "the error explains the non-finite override: {stdout}"
    );
}

#[test]
fn verbose_discloses_measured_data_that_the_default_hides() {
    // Default human report: prose messages only, no structured data blob.
    let (code, plain, _) = run(&[FIXTURE, "--profile", "ge-premier"]);
    assert_eq!(code, 0);
    assert!(
        !plain.contains("measured="),
        "the default human report omits the structured data: {plain}"
    );
    // --verbose appends each check's measured/expected data inline.
    let (code, verbose, _) = run(&[FIXTURE, "--profile", "ge-premier", "--verbose"]);
    assert_eq!(code, 0);
    assert!(
        verbose.contains("measured="),
        "--verbose discloses the structured data inline: {verbose}"
    );
}
