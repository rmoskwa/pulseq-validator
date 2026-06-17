//! End-to-end CLI acceptance (docs/02-crate-skeleton.md): `seq-validate` runs to
//! completion on the bundled v1.5.1 example, the human and `--json` forms are
//! well-formed with zero checks, and the exit-code policy holds (0 on success,
//! 2 on a harness/parse error).

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
fn human_report_on_example_exits_zero_with_zero_checks() {
    let (code, stdout, _) = run(&[FIXTURE]);
    assert_eq!(code, 0, "successful run must exit 0");
    assert!(stdout.contains("Pulseq 1.5.1"), "stdout: {stdout}");
    assert!(stdout.contains("50688 blocks"), "stdout: {stdout}");
    assert!(stdout.contains("No checks run."), "stdout: {stdout}");
    assert!(stdout.contains("Summary: 0 passed, 0 failed, 0 warnings, 0 skipped"));
}

#[test]
fn json_report_is_well_formed_with_zero_checks() {
    let (code, stdout, _) = run(&[FIXTURE, "--json"]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&stdout).expect("--json emits valid JSON");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["error"], Value::Null);
    assert_eq!(v["sequence"]["pulseq_version"], "1.5.1");
    assert!(v["sequence"]["blocks"].as_u64().unwrap() > 0);
    assert!(v["results"].as_array().unwrap().is_empty(), "zero checks");
    assert_eq!(v["summary"]["total"], 0);
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

#[test]
fn spec_and_profile_flags_are_accepted_but_noted() {
    let (code, _, stderr) = run(&[FIXTURE, "--spec", "expected.yaml", "--profile", "ge"]);
    assert_eq!(code, 0, "accepted flags must not break the run");
    assert!(
        stderr.contains("not yet active"),
        "expected an inactivity note on stderr, got: {stderr}"
    );
}
