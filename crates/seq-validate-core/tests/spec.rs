//! Acceptance tests for the expected-spec assert mode.
//!
//! These exercise [`Spec::assert`] against the *real* file-only results of the
//! bundled fixtures, so the unit conversions, oversampling division, and
//! dual-witness routing are checked end-to-end (not on synthetic results):
//!
//!   - a spec matching the Cartesian example passes, with geometry coming from
//!     the param-algebra witness and the declared oversampling divided out;
//!   - perturbing any field beyond its tolerance fails exactly that field;
//!   - a provided field the sequence cannot measure `skip`s (never `fail`s);
//!   - on an echo-train (EPI) family the param-algebra defers, so geometry is
//!     asserted against the trajectory witness (and the readout axis it cannot
//!     pin `skip`s).
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use seq_validate_core::{CheckCtx, CheckResult, Sequence, Spec, Status, checks::run_all};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// File-only results for a fixture (what the spec assertions read from). The
/// returned results are fully owned, so the sequence can drop here.
fn file_results(name: &str) -> Vec<CheckResult> {
    let seq = Sequence::from_file(fixture(name)).unwrap_or_else(|e| panic!("{name} parses: {e}"));
    run_all(&CheckCtx {
        seq: &seq,
        profile: None,
    })
}

fn status_of(results: &[CheckResult], id: &str) -> Option<Status> {
    results.iter().find(|r| r.id == id).map(|r| r.status)
}

#[test]
fn cartesian_spec_passes_with_oversampling_divided_out() {
    let file = file_results("t1_spgr_axial_brain.seq");
    // The committed example spec (physical 384/480 ÷ oversampling 2 = 192/240).
    let spec = Spec::from_yaml_str(
        "te_ms: 4.008\ntr_ms: 400.048\nflip_angle_deg: 80\nn_slices: 44\n\
         matrix: [192, 192, 1]\nfov_mm: [240, 240]\noversampling: [2, 1, 1]\n",
    )
    .unwrap();
    let spec_results = spec.assert(&file);

    // Every asserted field is present and passes; none fails.
    assert!(!spec_results.is_empty());
    for r in &spec_results {
        assert_eq!(
            r.status,
            Status::Pass,
            "{} should pass: {}",
            r.id,
            r.message
        );
        assert!(r.expected.is_some(), "{} carries its expected value", r.id);
    }
    // The oversampling division is what makes matrix_x pass against the nominal 192.
    let mx = spec_results
        .iter()
        .find(|r| r.id == "spec.matrix_x")
        .unwrap();
    assert_eq!(mx.measured.as_ref().and_then(|v| v.as_i64()), Some(192));
    assert!(mx.message.contains("oversampling 2"), "msg: {}", mx.message);
}

#[test]
fn out_of_tolerance_field_fails_only_itself() {
    let file = file_results("t1_spgr_axial_brain.seq");
    // tr is perturbed well past its 0.1 ms band; flip stays correct.
    let spec = Spec::from_yaml_str("tr_ms: 250.0\nflip_angle_deg: 80\n").unwrap();
    let spec_results = spec.assert(&file);

    assert_eq!(status_of(&spec_results, "spec.tr_ms"), Some(Status::Fail));
    assert_eq!(
        status_of(&spec_results, "spec.flip_angle_deg"),
        Some(Status::Pass)
    );
    assert_eq!(
        spec_results
            .iter()
            .filter(|r| r.status == Status::Fail)
            .count(),
        1,
        "exactly the perturbed field fails"
    );
}

#[test]
fn unmeasurable_field_skips_never_fails() {
    let file = file_results("t1_spgr_axial_brain.seq");
    // SPGR is single-echo: echo spacing is not measurable. Asserting it must
    // `skip`, not `fail` (a first-class non-failing result).
    let spec = Spec::from_yaml_str("echo_spacing_ms: 5.0\n").unwrap();
    let spec_results = spec.assert(&file);
    let esp = spec_results
        .iter()
        .find(|r| r.id == "spec.echo_spacing_ms")
        .unwrap();
    assert_eq!(esp.status, Status::Skip, "msg: {}", esp.message);
    assert!(
        esp.expected.is_some(),
        "the unmet expectation is still recorded"
    );
}

#[test]
fn tolerance_override_widens_the_band() {
    let file = file_results("t1_spgr_axial_brain.seq");
    // tr is ~400.05 ms; expecting 400.6 is out of the default 0.1 ms band but
    // inside an overridden 1 ms band.
    let strict = Spec::from_yaml_str("tr_ms: 400.6\n").unwrap();
    assert_eq!(
        status_of(&strict.assert(&file), "spec.tr_ms"),
        Some(Status::Fail)
    );

    let loose = Spec::from_yaml_str("tr_ms: 400.6\ntolerances:\n  tr_ms: {abs: 1.0}\n").unwrap();
    assert_eq!(
        status_of(&loose.assert(&file), "spec.tr_ms"),
        Some(Status::Pass)
    );
}

#[test]
fn echo_train_geometry_uses_the_trajectory_witness() {
    // EPI is an echo train: the param-algebra (`metrics.matrix`) skips, so the
    // trajectory gate owns geometry. Its clean phase-encode axis (ky = 64) is
    // asserted; the readout axis it cannot pin as a clean grid `skip`s.
    let file = file_results("epi_rs.seq");
    assert_eq!(
        status_of(&file, "metrics.matrix"),
        Some(Status::Skip),
        "EPI: param-algebra defers"
    );
    let spec = Spec::from_yaml_str("matrix: [64, 64, 1]\n").unwrap();
    let spec_results = spec.assert(&file);

    let my = spec_results
        .iter()
        .find(|r| r.id == "spec.matrix_y")
        .unwrap();
    assert_eq!(my.status, Status::Pass, "msg: {}", my.message);
    assert!(my.message.contains("trajectory"), "msg: {}", my.message);
    // The readout axis is not a clean grid for EPI → unmeasurable → skip.
    assert_eq!(
        status_of(&spec_results, "spec.matrix_x"),
        Some(Status::Skip)
    );
}
