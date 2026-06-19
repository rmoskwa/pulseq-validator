//! Acceptance tests for the GE vendor-conformance checks (`vendor.ge_*`).
//!
//! These build synthetic interpreted sequences out of TRID-delimited segment
//! *instances* and assert the two checks behave per GE's segment rule: an
//! event-bearing sequence without any TRID label fails `vendor.ge_trid_present`;
//! instances of one TRID that replay the same structure pass
//! `vendor.ge_trid_consistency` (amplitude and pure-delay duration may vary),
//! while a changed block count or non-delay duration fails it and a changed event
//! presence warns. They also pin the modular seam: the checks only run under a GE
//! profile, and are absent under another vendor or with no profile.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)] // test code: panics are the failure signal

use std::collections::BTreeMap;
use std::sync::Arc;

use num_complex::Complex64;
use seq_validate_core::checks::run_all;
use seq_validate_core::ir::{Adc, Block, BlockLabels, Gradient, Labels, Rf, RfUse, Shape};
use seq_validate_core::{CheckCtx, Profile, Sequence, Status, TimeRaster, Version};

// --- IR builders (mirroring tests/hardware.rs) -------------------------------

fn trap_gx(amp_hz: f64) -> Gradient {
    Gradient {
        amp: amp_hz,
        delay: 0.0,
        shape: Arc::new(Shape {
            time: vec![0.0, 1e-3, 3e-3, 4e-3],
            amp: vec![0.0, 1.0, 1.0, 0.0],
            duration: 4e-3,
        }),
    }
}

fn rect_rf() -> Rf {
    Rf {
        amp: 1e3,
        phase: 0.0,
        delay: 0.0,
        center: 1e-3,
        freq: 0.0,
        shape: Arc::new(Shape {
            time: vec![0.0, 2e-3],
            amp: vec![Complex64::new(1.0, 0.0), Complex64::new(1.0, 0.0)],
            duration: 2e-3,
        }),
        shims: vec![Complex64::new(1.0, 0.0)],
        rf_use: RfUse::Excitation,
    }
}

fn adc() -> Adc {
    Adc {
        num: 128,
        dwell: 4e-6,
        delay: 0.0,
        freq: 0.0,
        phase: 0.0,
        phase_shape: None,
        labels: Labels::default(),
    }
}

fn block(duration: f64) -> Block {
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

/// Mark `b` as a TRID segment start with identity `trid`.
fn trid_start(mut b: Block, trid: i32) -> Block {
    b.labels = BlockLabels {
        trid,
        trid_set: true,
        ..BlockLabels::default()
    };
    b
}

/// One three-block segment instance of TRID `trid`: RF-excite block (the segment
/// start), a gradient+ADC readout block scaled by `grad_amp`, and a pure-delay
/// block of `delay_dur`. Amplitude and the trailing delay are the legitimately
/// varying parameters.
fn segment(trid: i32, grad_amp: f64, delay_dur: f64) -> Vec<Block> {
    let mut excite = block(4e-3);
    excite.rf = Some(rect_rf());
    let excite = trid_start(excite, trid);

    let mut readout = block(4e-3);
    readout.gx = Some(trap_gx(grad_amp));
    readout.adc = Some(adc());

    vec![excite, readout, block(delay_dur)]
}

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

fn ge() -> Profile {
    Profile::by_name("ge-premier").unwrap()
}

/// Status of the result with `id` under `profile`, or `None` if no such result
/// was emitted (e.g. the vendor check did not run for this profile).
fn status_of(seq: &Sequence, profile: Option<&Profile>, id: &str) -> Option<Status> {
    run_all(&CheckCtx { seq, profile })
        .into_iter()
        .find(|r| r.id == id)
        .map(|r| r.status)
}

fn concat(mut a: Vec<Block>, b: Vec<Block>) -> Vec<Block> {
    a.extend(b);
    a
}

// --- vendor.ge_trid_present --------------------------------------------------

#[test]
fn ge_sequence_without_trid_fails_present() {
    // Events but no TRID label anywhere: GE cannot segment it.
    let mut readout = block(4e-3);
    readout.gx = Some(trap_gx(1e4));
    readout.adc = Some(adc());
    let seq = make_seq(vec![block(4e-3), readout]);

    assert_eq!(
        status_of(&seq, Some(&ge()), "vendor.ge_trid_present"),
        Some(Status::Fail)
    );
    // With no segments, consistency has nothing to compare and skips.
    assert_eq!(
        status_of(&seq, Some(&ge()), "vendor.ge_trid_consistency"),
        Some(Status::Skip)
    );
}

#[test]
fn trid_labels_present_passes_present() {
    let seq = make_seq(segment(1, 1e4, 1e-3));
    assert_eq!(
        status_of(&seq, Some(&ge()), "vendor.ge_trid_present"),
        Some(Status::Pass)
    );
}

// --- vendor.ge_trid_consistency ----------------------------------------------

#[test]
fn consistent_instances_with_varying_amplitude_pass() {
    // Two instances of TRID 1: identical structure, different gradient amplitude
    // and different pure-delay duration — both are allowed to vary.
    let seq = make_seq(concat(segment(1, 1e4, 1e-3), segment(1, 2e4, 5e-3)));
    assert_eq!(
        status_of(&seq, Some(&ge()), "vendor.ge_trid_consistency"),
        Some(Status::Pass)
    );
}

#[test]
fn differing_block_count_fails() {
    let first = segment(1, 1e4, 1e-3);
    let mut second = segment(1, 1e4, 1e-3);
    second.push(block(2e-3)); // an extra block in the second instance
    let seq = make_seq(concat(first, second));
    assert_eq!(
        status_of(&seq, Some(&ge()), "vendor.ge_trid_consistency"),
        Some(Status::Fail)
    );
}

#[test]
fn differing_nondelay_duration_fails() {
    let first = segment(1, 1e4, 1e-3);
    let mut second = segment(1, 1e4, 1e-3);
    second[1].duration = 6e-3; // readout (non-delay) block duration changed
    let seq = make_seq(concat(first, second));
    assert_eq!(
        status_of(&seq, Some(&ge()), "vendor.ge_trid_consistency"),
        Some(Status::Fail)
    );
}

#[test]
fn differing_event_presence_warns() {
    // Second instance's readout block drops its gradient (e.g. scaled to exactly
    // zero) — event presence differs, the benign-but-flagged case.
    let first = segment(1, 1e4, 1e-3);
    let mut second = segment(1, 1e4, 1e-3);
    second[1].gx = None;
    let seq = make_seq(concat(first, second));
    assert_eq!(
        status_of(&seq, Some(&ge()), "vendor.ge_trid_consistency"),
        Some(Status::Warn)
    );
}

// --- modular vendor seam -----------------------------------------------------

#[test]
fn non_ge_profile_omits_vendor_checks() {
    let seq = make_seq(segment(1, 1e4, 1e-3));
    let generic = Profile::by_name("generic-3t").unwrap();
    assert_eq!(generic.vendor, "generic");
    assert_eq!(
        status_of(&seq, Some(&generic), "vendor.ge_trid_present"),
        None
    );
    assert_eq!(
        status_of(&seq, Some(&generic), "vendor.ge_trid_consistency"),
        None
    );
}

#[test]
fn no_profile_omits_vendor_checks() {
    let seq = make_seq(segment(1, 1e4, 1e-3));
    assert_eq!(status_of(&seq, None, "vendor.ge_trid_present"), None);
}
