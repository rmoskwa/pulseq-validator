//! Trajectory fixture pins.
//!
//! Real-sequence checks for the trajectory gate + dual-witness geometry on the
//! bundled fixtures, complementing the synthetic unit tests in `trajectory.rs`
//! and the generated-corpus oracle in `corpus_oracle.rs`. These assert the
//! qualitative behaviour the design promises on hard cases the corpus does not
//! cover: a rotated-blade readout, a stack-of-stars (non-Cartesian in-plane +
//! Cartesian through-plane), a ramp-sampled EPI, and an oversampled Cartesian
//! readout — pinned to robust structural properties, not brittle exact values.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use seq_validate_core::checks::run_all;
use seq_validate_core::{CheckCtx, CheckResult, Measurements, Sequence, Status};

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");

fn results_for(name: &str) -> Vec<CheckResult> {
    let path = Path::new(FIXTURE_DIR).join(name);
    let seq = Sequence::from_file(&path).unwrap_or_else(|e| panic!("{name} must parse: {e}"));
    run_all(&CheckCtx {
        seq: &seq,
        profile: None,
    })
}

fn find<'a>(results: &'a [CheckResult], id: &str) -> &'a CheckResult {
    results
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("missing result {id}"))
}

fn status(results: &[CheckResult], id: &str) -> Status {
    find(results, id).status
}

/// An oversampled Cartesian readout: both witnesses still agree (the 2× readout
/// oversampling is physical and seen identically by area-algebra and trajectory).
#[test]
fn t1_spgr_dual_witness_agrees() {
    let r = results_for("t1_spgr_axial_brain.seq");
    let m = Measurements::from_results(&r);
    assert_eq!(m.dimensionality.unwrap(), 2.0);
    assert_eq!(status(&r, "metrics.matrix"), Status::Pass);
    assert_eq!(
        status(&r, "trajectory.geometry_agreement"),
        Status::Pass,
        "area-algebra and trajectory must agree even with readout oversampling"
    );
}

/// PROPELLER: rotated Cartesian blades. The blades must FAN OUT — large extent on
/// *both* in-plane axes (not collapsed onto one strip) — and the union is not a
/// single clean grid, so the exact matrix `skip`s while extent (coverage) holds.
#[test]
fn propeller_blades_fan_out() {
    let r = results_for("propeller-fse-axial.seq");
    let m = Measurements::from_results(&r);
    assert_eq!(m.dimensionality.unwrap(), 2.0);
    // Echo train ⇒ param-algebra defers.
    assert_eq!(status(&r, "metrics.matrix"), Status::Skip);
    // The rotated blades sweep both kx and ky — the fan-out witness.
    let ext = m.extent.unwrap();
    let (kx, ky) = (ext[0].unwrap(), ext[1].unwrap());
    assert!(
        kx > 300.0 && ky > 300.0,
        "blades must fan across both axes (kx={kx}, ky={ky}), not collapse onto one strip"
    );
    // No single clean grid ⇒ exact matrix is a skip (coverage is the witness).
    assert_eq!(status(&r, "trajectory.matrix"), Status::Skip);
}

/// Stack-of-stars: non-Cartesian (radial) in-plane, Cartesian through-plane. The
/// in-plane axes must fall to coverage (no exact count) while the partition axis
/// is detected as an exact grid — the independent in-plane-vs-through-plane test.
#[test]
fn stack_of_stars_inplane_vs_throughplane() {
    let r = results_for("sos-liver.seq");
    let m = Measurements::from_results(&r);
    assert_eq!(
        m.dimensionality.unwrap(),
        3.0,
        "stack-of-stars encodes kz ⇒ 3D"
    );
    assert_eq!(
        status(&r, "metrics.matrix"),
        Status::Skip,
        "rotated readout"
    );
    // kx, ky are non-Cartesian (radial) ⇒ no exact count; kz is a clean grid.
    let mat = m.matrix.trajectory.unwrap();
    assert!(mat[0].is_none(), "kx is radial (coverage), not a grid");
    assert!(mat[1].is_none(), "ky is radial (coverage), not a grid");
    assert!(
        mat[2].is_some_and(|n| n > 1.0),
        "kz partitions are a clean grid: {:?}",
        mat[2]
    );
}

/// Ramp-sampled EPI: the echo train defers the param-algebra, and the trajectory
/// recovers the (clean) phase-encode blip count even though the ramp-sampled
/// readout is not a uniform kx grid.
#[test]
fn epi_ramp_sampled_phase_encode_grid() {
    let r = results_for("epi_rs.seq");
    let m = Measurements::from_results(&r);
    assert_eq!(m.dimensionality.unwrap(), 2.0);
    assert_eq!(status(&r, "metrics.matrix"), Status::Skip);
    // ky blips are a clean grid; kx (ramp-sampled) is coverage.
    let mat = m.matrix.trajectory.unwrap();
    assert!(
        mat[1].is_some_and(|n| n > 1.0),
        "phase-encode blips form a clean grid: {:?}",
        mat[1]
    );
}
