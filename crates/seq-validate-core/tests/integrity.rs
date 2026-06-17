//! Acceptance tests for the integrity checks.
//!
//! Strategy: a tiny, fully-controlled synthetic v1.5.1 sequence ([`BASE`]) that
//! passes every integrity check, then a hand-corruption per acceptance case —
//! each must flip exactly the right check to the right status/severity. The real
//! bundled example is used for the headline "all integrity checks pass on the
//! example file" criterion and to prove the signature recompute is correct on a
//! real, validly-signed file.
//!
//! Note the *layering*: structurally-broken corruption (a dangling reference) is
//! rejected by the parser as a harness error (exit 2) before any check runs — the
//! integrity checks assert on what survives parsing. That boundary is itself
//! tested (`dangling_reference_is_a_harness_error`).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use seq_validate_core::checks::run_all;
use seq_validate_core::{CheckCtx, CheckResult, DEFAULT_LARMOR_HZ, Report, Sequence, Status};

/// A minimal, valid Pulseq 1.5.1 sequence: a trapezoid-gradient block and an
/// ADC block. Block durations leave slack over their events, so a perturbed
/// delay stays inside its block (no parser "event too long" error masks the
/// raster check). It declares FOV, all rasters, and a matching `TotalDuration`,
/// and carries no `[SIGNATURE]`. Every integrity check passes or skips on it.
const BASE: &str = "\
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

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/t1_spgr_axial_brain.seq"
);

fn results_for(source: &str) -> Vec<CheckResult> {
    let seq = Sequence::from_source(source, DEFAULT_LARMOR_HZ).expect("source must parse");
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

#[test]
fn base_sequence_passes_every_check() {
    let r = results_for(BASE);

    assert_eq!(get(&r, "integrity.version").status, Status::Pass);
    assert_eq!(get(&r, "integrity.definitions").status, Status::Pass);
    assert_eq!(get(&r, "integrity.raster_alignment").status, Status::Pass);
    assert_eq!(get(&r, "integrity.timing").status, Status::Pass);
    assert_eq!(get(&r, "integrity.event_legality").status, Status::Pass);
    // Scanner-specific / unsigned → first-class skips, not failures.
    assert_eq!(get(&r, "integrity.dead_time").status, Status::Skip);
    assert_eq!(get(&r, "integrity.signature").status, Status::Skip);

    assert_eq!(
        r.iter().filter(|x| x.status == Status::Fail).count(),
        0,
        "no failures on a clean sequence"
    );
}

#[test]
fn misaligned_event_fails_raster_alignment() {
    // Shift the trapezoid's start delay to 5 µs — off the 10 µs gradient raster,
    // but still inside its (300 µs) block so it survives parsing.
    let bad = BASE.replace("50 100 50 0", "50 100 50 5");
    let r = results_for(&bad);

    let res = get(&r, "integrity.raster_alignment");
    assert_eq!(res.status, Status::Fail);
    assert_eq!(res.severity, seq_validate_core::Severity::Error);
    assert!(
        res.message.contains("off-raster"),
        "message should explain the misalignment: {}",
        res.message
    );
}

#[test]
fn total_duration_mismatch_warns() {
    let bad = BASE.replace("TotalDuration 0.0005", "TotalDuration 0.0009");
    let r = results_for(&bad);

    let res = get(&r, "integrity.timing");
    assert_eq!(res.status, Status::Warn);
    // The computed (authoritative) duration and the declared one are both shown.
    assert!(res.measured.is_some() && res.expected.is_some());
    // A stale TotalDuration is suspicious, not fatal: it must not fail the run.
    assert_eq!(r.iter().filter(|x| x.status == Status::Fail).count(), 0);
}

#[test]
fn dangling_reference_is_a_harness_error() {
    // Point block 1's GX at a gradient id that doesn't exist. The parser rejects
    // this before the IR exists, so it is a harness error (exit 2), not a check.
    let dangling = BASE.replace("1 30 0 1 0 0 0 0", "1 30 0 7 0 0 0 0");
    let err = Sequence::from_source(&dangling, DEFAULT_LARMOR_HZ)
        .err()
        .expect("a dangling reference must not parse");
    let msg = err.to_string();
    assert!(
        msg.contains('7') && msg.to_lowercase().contains("exist"),
        "expected a 'referenced GX 7 does not exist' style error, got: {msg}"
    );
}

#[test]
fn bad_signature_warns() {
    let bad = format!("{BASE}[SIGNATURE]\nType md5\nHash 00000000000000000000000000000000\n");
    let r = results_for(&bad);

    let res = get(&r, "integrity.signature");
    assert_eq!(res.status, Status::Warn);
    assert!(
        res.message.to_lowercase().contains("mismatch"),
        "message should report the mismatch: {}",
        res.message
    );
    // Tampering with the hash is a warning, never a hard failure.
    assert_eq!(r.iter().filter(|x| x.status == Status::Fail).count(), 0);
}

#[test]
fn missing_fov_warns() {
    let bad = BASE.replace("FOV 0.25 0.25 0.005\n", "");
    let r = results_for(&bad);

    let res = get(&r, "integrity.definitions");
    assert_eq!(res.status, Status::Warn);
    assert!(res.message.to_lowercase().contains("fov"));
}

#[test]
fn nonpositive_raster_fails() {
    let bad = BASE.replace("GradientRasterTime 1e-05", "GradientRasterTime 0");
    let r = results_for(&bad);

    let res = get(&r, "integrity.definitions");
    assert_eq!(res.status, Status::Fail);
    assert_eq!(res.severity, seq_validate_core::Severity::Error);
    // The zero raster is reported once, by `definitions` — raster_alignment must
    // not also blow up dividing by it.
    assert_eq!(get(&r, "integrity.raster_alignment").status, Status::Pass);
}

#[test]
fn example_passes_all_and_signature_verifies() {
    let seq = Sequence::from_file(EXAMPLE).expect("bundled example must parse");
    let r = run_all(&CheckCtx {
        seq: &seq,
        profile: None,
    });

    // Acceptance: all integrity checks pass on the example file (none fail/warn).
    assert_eq!(
        r.iter()
            .filter(|x| matches!(x.status, Status::Fail | Status::Warn))
            .count(),
        0,
        "the example should produce no failures or warnings: {r:#?}"
    );
    // The signature recompute is correct on a real, validly-signed file.
    assert_eq!(get(&r, "integrity.signature").status, Status::Pass);
    assert_eq!(get(&r, "integrity.definitions").status, Status::Pass);
    assert_eq!(get(&r, "integrity.raster_alignment").status, Status::Pass);
    assert_eq!(get(&r, "integrity.timing").status, Status::Pass);
}

#[test]
fn exit_codes_follow_policy() {
    // Clean sequence → exit 0.
    let seq = Sequence::from_source(BASE, DEFAULT_LARMOR_HZ).unwrap();
    let clean = Report::for_sequence(
        "base",
        &seq,
        run_all(&CheckCtx {
            seq: &seq,
            profile: None,
        }),
    );
    assert_eq!(clean.exit_code(), 0);

    // A check failure → exit 1.
    let bad = BASE.replace("50 100 50 0", "50 100 50 5");
    let seq = Sequence::from_source(&bad, DEFAULT_LARMOR_HZ).unwrap();
    let failing = Report::for_sequence(
        "bad",
        &seq,
        run_all(&CheckCtx {
            seq: &seq,
            profile: None,
        }),
    );
    assert_eq!(failing.exit_code(), 1);

    // Parse-blocking corruption → harness error → exit 2.
    let dangling = BASE.replace("1 30 0 1 0 0 0 0", "1 30 0 7 0 0 0 0");
    let err = Sequence::from_source(&dangling, DEFAULT_LARMOR_HZ)
        .err()
        .expect("dangling reference must not parse");
    let harness = Report::harness_error("dangling", err.to_string());
    assert_eq!(harness.exit_code(), 2);
}
