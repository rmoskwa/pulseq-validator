//! End-to-end CLI acceptance: `seq-validate` runs to completion on the bundled
//! v1.5.1 example, the human and `--json` forms are well-formed and carry the
//! Step-3 integrity results (which all pass on the clean example), and the
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
    // Step 3 populates the registry: the integrity section and its checks render.
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

#[test]
fn spec_and_profile_flags_are_accepted_but_noted() {
    let (code, _, stderr) = run(&[FIXTURE, "--spec", "expected.yaml", "--profile", "ge"]);
    assert_eq!(code, 0, "accepted flags must not break the run");
    assert!(
        stderr.contains("not yet active"),
        "expected an inactivity note on stderr, got: {stderr}"
    );
}
