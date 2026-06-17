//! Unit tests for the typed measurements surface.
//!
//! `Measurements::from_results` is the one place that maps metric ids → typed
//! fields, reads `measured` as scalar/array, and routes the dual-witness
//! geometry. These build synthetic result vectors and assert each of those
//! behaviours directly, so the spec/test consumers can trust the surface.
#![allow(clippy::unwrap_used)]

use seq_validate_core::measurements::Measurements;
use seq_validate_core::serde_json::json;
use seq_validate_core::{CheckResult, Status};

#[test]
fn scalars_read_by_id_with_units_in_the_type() {
    let results = vec![
        CheckResult::pass("metrics.te", "").with_measured(0.041),
        CheckResult::pass("metrics.tr", "").with_measured(0.08),
        CheckResult::pass("metrics.flip_angle", "").with_measured(90.0),
        CheckResult::pass("metrics.scan_time", "").with_measured(0.08),
        CheckResult::pass("metrics.n_slices", "").with_measured(2),
    ];
    let m = Measurements::from_results(&results);
    assert_eq!(m.te_s, Some(0.041));
    assert_eq!(m.tr_s, Some(0.08));
    assert_eq!(m.flip_deg, Some(90.0));
    assert_eq!(m.scan_time_s, Some(0.08));
    assert_eq!(m.n_slices, Some(2.0));
}

#[test]
fn a_skipped_metric_is_none_not_a_guessed_zero() {
    // A `skip` carries no `measured`; the typed field is `None`, so consumers
    // branch on the Option instead of re-deriving status from absence.
    let results = vec![CheckResult::skip("metrics.te", "no readout")];
    let m = Measurements::from_results(&results);
    assert_eq!(m.te_s, None);
    // A metric that never ran is also None.
    assert_eq!(m.tr_s, None);
}

#[test]
fn array_axes_parse_with_null_meaning_unpinned() {
    let results = vec![
        CheckResult::pass("trajectory.extent", "").with_measured(json!([320.0, 320.0, null])),
    ];
    let m = Measurements::from_results(&results);
    assert_eq!(m.extent, Some(vec![Some(320.0), Some(320.0), None]));
}

#[test]
fn geometry_authoritative_is_param_algebra_when_it_passed() {
    // Cartesian: the param-algebra check passed, so it is the witness.
    let results = vec![
        CheckResult::pass("metrics.matrix", "").with_measured(json!([192, 192, 1])),
        CheckResult::pass("trajectory.matrix", "").with_measured(json!([200, 200, 1])),
    ];
    let m = Measurements::from_results(&results);
    let (values, label) = m.matrix.authoritative();
    assert_eq!(label, "param-algebra");
    assert_eq!(values, Some([Some(192.0), Some(192.0), Some(1.0)].as_slice()));
    // Both raw witnesses remain readable.
    assert_eq!(m.matrix.param_status, Some(Status::Pass));
    assert!(m.matrix.trajectory.is_some());
}

#[test]
fn geometry_authoritative_falls_to_trajectory_when_param_defers() {
    // Echo train: the param-algebra `skip`s, so the trajectory gate is the witness.
    let results = vec![
        CheckResult::skip("metrics.matrix", "echo train"),
        CheckResult::pass("trajectory.matrix", "").with_measured(json!([null, 64, 1])),
    ];
    let m = Measurements::from_results(&results);
    let (values, label) = m.matrix.authoritative();
    assert_eq!(label, "trajectory");
    assert_eq!(values, Some([None, Some(64.0), Some(1.0)].as_slice()));
    assert_eq!(m.matrix.param_status, Some(Status::Skip));
}
