//! Acceptance tests for the hardware/safety checks: they
//! produce the correct `fail` — with the offending value and the limit — when a
//! synthetic sequence exceeds gradient amplitude, slew, B1, ADC dwell, dead time,
//! or PNS, and `skip` cleanly when no profile is selected. The amplitude/slew/B1
//! fixtures are built directly in the interpreted IR so each limit can be probed
//! in isolation, against a real bundled [`Profile`] (with single-field overrides
//! where a clean isolation needs it).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)] // test code: panics are the failure signal

use std::collections::BTreeMap;
use std::sync::Arc;

use num_complex::Complex64;
use seq_validate_core::checks::run_all;
use seq_validate_core::ir::{Adc, Block, BlockLabels, Gradient, Labels, Rf, RfUse, Shape};
use seq_validate_core::{
    CheckCtx, CheckResult, DEFAULT_LARMOR_HZ, Profile, Sequence, Status, TimeRaster, Version,
};

/// Hz/m for a gradient of `mt_per_m` milli-tesla per metre (¹H γ̄).
fn grad_hz(mt_per_m: f64) -> f64 {
    mt_per_m * 1e-3 * DEFAULT_LARMOR_HZ
}
/// Hz for an RF B1 of `ut` micro-tesla (¹H γ̄).
fn rf_hz(ut: f64) -> f64 {
    ut * 1e-6 * DEFAULT_LARMOR_HZ
}

/// A single-axis trapezoid gradient (`amp` Hz/m) with `rise = fall` and `flat`
/// seconds, placed on the gx axis.
fn trap_gx(amp_hz: f64, rise: f64, flat: f64) -> Gradient {
    Gradient {
        amp: amp_hz,
        delay: 0.0,
        shape: Arc::new(Shape {
            time: vec![0.0, rise, rise + flat, rise + flat + rise],
            amp: vec![0.0, 1.0, 1.0, 0.0],
            duration: rise + flat + rise,
        }),
    }
}

/// A constant-envelope RF event of peak `amp` Hz lasting `dur` seconds, starting
/// at `delay`.
fn rect_rf(amp_hz: f64, dur: f64, delay: f64) -> Rf {
    Rf {
        amp: amp_hz,
        phase: 0.0,
        delay,
        center: dur / 2.0,
        freq: 0.0,
        shape: Arc::new(Shape {
            time: vec![0.0, dur],
            amp: vec![Complex64::new(1.0, 0.0), Complex64::new(1.0, 0.0)],
            duration: dur,
        }),
        shims: vec![Complex64::new(1.0, 0.0)],
        rf_use: RfUse::Excitation,
    }
}

fn empty_block(duration: f64) -> Block {
    Block {
        duration,
        rf: None,
        gx: None,
        gy: None,
        gz: None,
        adc: None,
        triggers: vec![],
        labels: BlockLabels::default(),
    }
}

/// Wrap blocks into a minimal interpreted [`Sequence`]; timing fields are derived
/// so the checks see a self-consistent sequence.
fn make_seq(blocks: Vec<Block>) -> Sequence {
    let mut starts = Vec::with_capacity(blocks.len());
    let mut t = 0.0;
    for b in &blocks {
        starts.push(t);
        t += b.duration;
    }
    let logical_grad_areas = vec![[0.0; 3]; blocks.len()];
    Sequence {
        version: Version {
            major: 1,
            minor: 5,
            revision: 1,
            suppl: None,
        },
        name: None,
        fov: [0.2, 0.2, 0.2],
        time_raster: TimeRaster {
            grad: 4e-6,
            rf: 2e-6,
            adc: 2e-6,
            block: 4e-6,
        },
        definitions: BTreeMap::new(),
        signature: None,
        blocks,
        starts,
        logical_grad_areas,
        total_duration: t,
        warnings: vec![],
    }
}

/// Run the checks against `seq` + `profile` and return the result with `id`.
fn result_for(seq: &Sequence, profile: &Profile, id: &str) -> CheckResult {
    run_all(&CheckCtx {
        seq,
        profile: Some(profile),
    })
    .into_iter()
    .find(|r| r.id == id)
    .unwrap_or_else(|| panic!("no {id} result"))
}

#[test]
fn gradient_amplitude_over_limit_fails_with_value_and_limit() {
    // 60 mT/m on gx vs ge-premier's 50 mT/m; a 1 ms ramp keeps slew well under
    // limit so only amplitude trips.
    let g = trap_gx(grad_hz(60.0), 1e-3, 2e-3);
    let mut b = empty_block(4e-3);
    b.gx = Some(g);
    let seq = make_seq(vec![b]);
    let p = Profile::by_name("ge-premier").unwrap();

    let r = result_for(&seq, &p, "hardware.gradient_amplitude");
    assert_eq!(r.status, Status::Fail);
    assert!(
        r.message.contains("60.0") && r.message.contains("maxGrad 50.0"),
        "fail names offending value and limit: {}",
        r.message
    );
}

#[test]
fn slew_over_limit_fails_with_value_and_limit() {
    // 40 mT/m (within 50) but a 0.1 ms ramp ⇒ 400 T/m/s ≫ 150 T/m/s.
    let g = trap_gx(grad_hz(40.0), 1e-4, 1e-3);
    let mut b = empty_block(2e-3);
    b.gx = Some(g);
    let seq = make_seq(vec![b]);
    let p = Profile::by_name("ge-premier").unwrap();

    let r = result_for(&seq, &p, "hardware.slew_rate");
    assert_eq!(r.status, Status::Fail);
    assert!(
        r.message.contains("maxSlew 150.0"),
        "fail names the slew limit: {}",
        r.message
    );
}

#[test]
fn peak_b1_over_limit_fails_with_value_and_limit() {
    // 30 µT peak vs ge-premier's 20 µT.
    let mut b = empty_block(3e-3);
    b.rf = Some(rect_rf(rf_hz(30.0), 2e-3, 100e-6));
    let seq = make_seq(vec![b]);
    let p = Profile::by_name("ge-premier").unwrap();

    let r = result_for(&seq, &p, "hardware.rf_b1");
    assert_eq!(r.status, Status::Fail);
    assert!(
        r.message.contains("30.0") && r.message.contains("B1max 20.0"),
        "fail names offending value and limit: {}",
        r.message
    );
}

#[test]
fn adc_dwell_off_scanner_raster_fails() {
    // 2.5 µs dwell is not a multiple of ge-premier's 2 µs ADC raster.
    let mut b = empty_block(1e-3);
    b.adc = Some(Adc {
        num: 128,
        dwell: 2.5e-6,
        delay: 0.0,
        freq: 0.0,
        phase: 0.0,
        phase_shape: None,
        labels: Labels::default(),
    });
    let seq = make_seq(vec![b]);
    let p = Profile::by_name("ge-premier").unwrap();

    let r = result_for(&seq, &p, "hardware.adc_dwell");
    assert_eq!(r.status, Status::Fail);
    assert!(
        r.message.contains("not a multiple"),
        "fail explains the raster mismatch: {}",
        r.message
    );
}

#[test]
fn dead_time_violation_fails() {
    // ADC starts at t=0 but the (overridden) profile demands a 100 µs ADC dead time.
    let mut b = empty_block(1e-3);
    b.adc = Some(Adc {
        num: 64,
        dwell: 2e-6,
        delay: 0.0,
        freq: 0.0,
        phase: 0.0,
        phase_shape: None,
        labels: Labels::default(),
    });
    let seq = make_seq(vec![b]);
    let mut p = Profile::by_name("ge-premier").unwrap();
    p.apply_override("adcDeadTime", 100e-6).unwrap();

    let r = result_for(&seq, &p, "hardware.dead_time");
    assert_eq!(r.status, Status::Fail);
    assert!(
        r.message.contains("adcDeadTime"),
        "fail names the violated constraint: {}",
        r.message
    );
}

#[test]
fn pns_over_threshold_warns_never_fails() {
    // A steep, long ramp drives the PNS proxy past 100%. With amplitude/slew
    // limits raised out of the way, only the PNS estimate reacts — and it `warn`s,
    // never `fail`s (it is an approximate proxy).
    let g = trap_gx(grad_hz(150.0), 1e-3, 0.0);
    let mut b = empty_block(2e-3);
    b.gx = Some(g);
    let seq = make_seq(vec![b]);
    let mut p = Profile::by_name("ge-premier").unwrap();
    p.apply_override("maxGrad", 1e4).unwrap();
    p.apply_override("maxSlew", 1e4).unwrap();

    let r = result_for(&seq, &p, "hardware.pns");
    assert_eq!(
        r.status,
        Status::Warn,
        "over-threshold PNS warns: {}",
        r.message
    );
    assert!(r.message.contains("PNS") && r.message.contains('%'));
}

#[test]
fn adc_dwell_skips_when_profile_has_no_positive_raster() {
    // A non-positive ADC raster (overridden here; also reachable from a
    // file-definitions profile) cannot gate dwell divisibility. The check must
    // `skip`, not pass vacuously — the 2.5 µs dwell would FAIL a real 2 µs raster.
    let mut b = empty_block(1e-3);
    b.adc = Some(Adc {
        num: 64,
        dwell: 2.5e-6,
        delay: 0.0,
        freq: 0.0,
        phase: 0.0,
        phase_shape: None,
        labels: Labels::default(),
    });
    let seq = make_seq(vec![b]);
    let mut p = Profile::by_name("ge-premier").unwrap();
    p.apply_override("adc_raster_s", 0.0).unwrap();

    let r = result_for(&seq, &p, "hardware.adc_dwell");
    assert_eq!(r.status, Status::Skip);
}

#[test]
fn generic_3t_profile_skips_pns() {
    // The vendor-neutral profile carries no PNS model ⇒ the PNS check skips.
    let g = trap_gx(grad_hz(150.0), 1e-3, 0.0);
    let mut b = empty_block(2e-3);
    b.gx = Some(g);
    let seq = make_seq(vec![b]);
    let p = Profile::by_name("generic-3t").unwrap();

    let r = result_for(&seq, &p, "hardware.pns");
    assert_eq!(r.status, Status::Skip);
}

#[test]
fn no_profile_emits_a_single_non_silent_skip() {
    // File-only mode: exactly the hardware.profile skip, nothing failing/warning.
    let seq = make_seq(vec![empty_block(1e-3)]);
    let results = run_all(&CheckCtx {
        seq: &seq,
        profile: None,
    });
    let hw: Vec<&CheckResult> = results
        .iter()
        .filter(|r| r.id.starts_with("hardware."))
        .collect();
    assert_eq!(hw.len(), 1, "only the resolution result when no profile");
    assert_eq!(hw[0].id, "hardware.profile");
    assert_eq!(hw[0].status, Status::Skip);
    assert!(hw[0].message.contains("no scanner profile"));
}
