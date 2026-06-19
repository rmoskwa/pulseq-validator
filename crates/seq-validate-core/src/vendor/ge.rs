//! GE-specific conformance checks.
//!
//! GE's interpreter pre-compiles each *segment* — the run of blocks delimited by
//! a `TRID` label — once, then replays that compiled structure for every
//! instance. Two static rules follow, neither with a Siemens/Philips analogue:
//!
//! - **`vendor.ge_trid_present`** — the sequence must carry `TRID` segment labels
//!   at all. Without them the GE interpreter cannot segment the sequence; the old
//!   MATLAB harness could not even parse such a file. Fails when an event-bearing
//!   sequence has no `TRID` label.
//! - **`vendor.ge_trid_consistency`** — every instance of a given `TRID` must
//!   replay the same structure: same block count, same per-block event presence,
//!   same non-delay-block durations. Amplitudes, RF phase/freq offsets, block
//!   rotation, and pure-delay-block *durations* may vary; event presence and
//!   structural timing may not.
//!
//! The comparison runs on the interpreted IR, which bakes block rotation into the
//! gradients (PROPELLER blades, rotated spokes), so it compares only rotation-
//! and amplitude-invariant witnesses: event presence, ADC sampling, RF timing,
//! and block duration. Gradient *shape/window* is deliberately **not** compared:
//! the interpreter resamples rotated gradients onto a shared grid, so a gradient's
//! active window is not stable across rotated instances (it would false-positive
//! on every PROPELLER blade). Gradient structure is screened by presence (does the
//! block drive any gradient) and block duration; finer gradient-shape divergence
//! is left to GE's own downstream gate. This is the first-line structural screen,
//! mirroring the old harness's error-vs-warning split (block-count /
//! non-delay-duration → fail; other structure → warn).

use serde_json::json;

use crate::checks::{Check, CheckCtx, CheckDoc};
use crate::ir::{Block, Sequence};
use crate::result::{Category, CheckResult};

const PRESENT_ID: &str = "vendor.ge_trid_present";
const CONSISTENCY_ID: &str = "vendor.ge_trid_consistency";

/// Block-duration equality tolerance [s] (matches the GE harness's 1e-6 s).
const DUR_TOL_S: f64 = 1e-6;
/// Timing slack for structural-witness comparisons [s].
const TIMING_TOL_S: f64 = 1e-7;
/// Cap on the number of per-block discrepancies listed in a result message.
const MAX_LISTED: usize = 5;

/// The GE conformance check, wired into [`crate::vendor::checks`].
pub(crate) fn checks() -> Vec<Box<dyn Check>> {
    vec![Box::new(GeTrid)]
}

struct GeTrid;

impl Check for GeTrid {
    fn category(&self) -> Category {
        Category::Vendor
    }
    fn name(&self) -> &'static str {
        // Unused: this check emits two explicit ids, not `<category>.<name>`.
        "ge_trid"
    }
    fn vendor_scope(&self) -> &'static [&'static str] {
        &["ge"]
    }

    fn docs(&self) -> Vec<CheckDoc> {
        vec![
            CheckDoc::new(
                PRESENT_ID,
                "GE requires TRID-delimited segments; fails when an event-bearing sequence carries no TRID label.",
            ),
            CheckDoc::new(
                CONSISTENCY_ID,
                "Each TRID's instances replay the same structure (block count, event presence, non-delay durations); fails on a structural break, warns on lesser divergence, skips without TRID labels.",
            ),
        ]
    }

    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let seq = ctx.seq;
        let has_trid = seq.blocks.iter().any(|b| b.labels.trid_set);
        let consistency = if has_trid {
            consistency_result(seq)
        } else {
            CheckResult::skip(CONSISTENCY_ID, "no TRID segment labels; nothing to compare")
        };
        vec![present_result(seq), consistency]
    }
}

/// `vendor.ge_trid_present`: the sequence must carry TRID labels.
fn present_result(seq: &Sequence) -> CheckResult {
    let n_trid = seq.blocks.iter().filter(|b| b.labels.trid_set).count();
    if n_trid > 0 {
        return CheckResult::pass(
            PRESENT_ID,
            format!("{n_trid} TRID segment label(s) present"),
        )
        .with_measured(json!(n_trid));
    }
    // No TRID labels. An empty / event-free sequence has nothing to segment.
    if !seq.blocks.iter().any(is_event_block) {
        return CheckResult::skip(
            PRESENT_ID,
            "sequence has no event blocks; no TRID labels required",
        );
    }
    CheckResult::fail(
        PRESENT_ID,
        "no TRID segment labels found; GE's interpreter requires each TR/segment to be \
         delimited by a TRID label (mr.addTRID at the top of each TR loop body).",
    )
    .with_measured(json!(0))
}

/// `vendor.ge_trid_consistency`: every instance of a TRID replays the same
/// structure. Compares each instance against the first instance of its TRID.
fn consistency_result(seq: &Sequence) -> CheckResult {
    let instances = instances(seq);
    // Group instances by TRID value, preserving first-seen order.
    let mut groups: Vec<(i32, Vec<&Instance>)> = Vec::new();
    for inst in &instances {
        match groups.iter_mut().find(|(t, _)| *t == inst.trid) {
            Some((_, members)) => members.push(inst),
            None => groups.push((inst.trid, vec![inst])),
        }
    }

    let mut fails: Vec<String> = Vec::new();
    let mut warns: Vec<String> = Vec::new();
    for (trid, members) in &groups {
        let Some(reference) = members.first() else {
            continue;
        };
        for (k, inst) in members.iter().enumerate().skip(1) {
            compare(*trid, k, reference, inst, &mut fails, &mut warns);
        }
    }

    let measured = json!({ "segments": groups.len(), "instances": instances.len() });
    if !fails.is_empty() {
        return CheckResult::fail(
            CONSISTENCY_ID,
            summarize("inconsistent TRID segments", &fails),
        )
        .with_measured(measured);
    }
    if !warns.is_empty() {
        return CheckResult::warn(
            CONSISTENCY_ID,
            summarize(
                "TRID instances resolve to differing structure (often benign — e.g. an \
                 arbitrary gradient scaled to exactly 0 instead of eps)",
                &warns,
            ),
        )
        .with_measured(measured);
    }
    CheckResult::pass(
        CONSISTENCY_ID,
        format!(
            "{} instance(s) across {} TRID segment(s) replay consistently",
            instances.len(),
            groups.len()
        ),
    )
    .with_measured(measured)
}

/// One segment instance: its TRID identity and the blocks it spans (from its
/// TRID set-site up to, but excluding, the next set-site or the sequence end).
struct Instance<'a> {
    trid: i32,
    blocks: &'a [Block],
}

/// Split the sequence into segment instances at each TRID set-site. Blocks before
/// the first set-site (sequence preamble) belong to no segment and are ignored.
fn instances(seq: &Sequence) -> Vec<Instance<'_>> {
    let starts: Vec<usize> = seq
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.labels.trid_set)
        .map(|(i, _)| i)
        .collect();
    starts
        .iter()
        .enumerate()
        .filter_map(|(k, &s)| {
            let end = starts.get(k + 1).copied().unwrap_or(seq.blocks.len());
            let blocks = seq.blocks.get(s..end)?;
            let trid = blocks.first()?.labels.trid;
            Some(Instance { trid, blocks })
        })
        .collect()
}

/// Compare one instance against the reference instance of the same TRID, pushing
/// any block-count / duration breaks onto `fails` and lesser structural
/// divergences onto `warns`.
fn compare(
    trid: i32,
    k: usize,
    reference: &Instance,
    inst: &Instance,
    fails: &mut Vec<String>,
    warns: &mut Vec<String>,
) {
    let loc = format!("TRID {trid} instance #{}", k + 1);
    if reference.blocks.len() != inst.blocks.len() {
        fails.push(format!(
            "{loc}: {} block(s), first instance has {}",
            inst.blocks.len(),
            reference.blocks.len()
        ));
        return;
    }
    for (j, (rb, ib)) in reference.blocks.iter().zip(inst.blocks).enumerate() {
        let (rs, is) = (Sig::of(rb), Sig::of(ib));
        // Event presence (incl. pure-delay classification) must match.
        if rs.presence() != is.presence() {
            warns.push(format!("{loc} block {j}: event presence differs"));
            continue;
        }
        // Non-delay blocks must hold identical duration across instances; a
        // pure-delay block's duration is free to vary.
        if !rs.is_pure_delay() && (rb.duration - ib.duration).abs() > DUR_TOL_S {
            fails.push(format!(
                "{loc} block {j}: non-delay block duration {:.6e}s vs {:.6e}s",
                ib.duration, rb.duration
            ));
            continue;
        }
        // Remaining structural witnesses (rotation/amplitude invariant).
        if !rs.structurally_eq(&is) {
            warns.push(format!(
                "{loc} block {j}: structure differs (ADC/RF timing)"
            ));
        }
    }
}

/// A block's rotation- and amplitude-invariant structural signature. Gradient
/// shape/window is intentionally absent — see the module docs.
struct Sig {
    has_rf: bool,
    has_adc: bool,
    has_grad: bool,
    adc_num: u32,
    adc_dwell: f64,
    adc_delay: f64,
    rf_delay: f64,
    rf_center: f64,
    rf_dur: f64,
    rf_samples: usize,
}

impl Sig {
    fn of(b: &Block) -> Self {
        Sig {
            has_rf: b.rf.is_some(),
            has_adc: b.adc.is_some(),
            has_grad: b.gx.is_some() || b.gy.is_some() || b.gz.is_some(),
            adc_num: b.adc.as_ref().map_or(0, |a| a.num),
            adc_dwell: b.adc.as_ref().map_or(0.0, |a| a.dwell),
            adc_delay: b.adc.as_ref().map_or(0.0, |a| a.delay),
            rf_delay: b.rf.as_ref().map_or(0.0, |r| r.delay),
            rf_center: b.rf.as_ref().map_or(0.0, |r| r.center),
            rf_dur: b.rf.as_ref().map_or(0.0, |r| r.shape.duration),
            rf_samples: b.rf.as_ref().map_or(0, |r| r.shape.time.len()),
        }
    }

    fn presence(&self) -> (bool, bool, bool) {
        (self.has_rf, self.has_adc, self.has_grad)
    }

    fn is_pure_delay(&self) -> bool {
        !self.has_rf && !self.has_adc && !self.has_grad
    }

    /// Structural equality on the rotation/amplitude-invariant witnesses, given
    /// presence already matches.
    fn structurally_eq(&self, o: &Sig) -> bool {
        if self.has_adc
            && (self.adc_num != o.adc_num
                || !close(self.adc_dwell, o.adc_dwell)
                || !close(self.adc_delay, o.adc_delay))
        {
            return false;
        }
        if self.has_rf
            && (self.rf_samples != o.rf_samples
                || !close(self.rf_delay, o.rf_delay)
                || !close(self.rf_center, o.rf_center)
                || !close(self.rf_dur, o.rf_dur))
        {
            return false;
        }
        true
    }
}

fn is_event_block(b: &Block) -> bool {
    b.rf.is_some() || b.adc.is_some() || b.gx.is_some() || b.gy.is_some() || b.gz.is_some()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= TIMING_TOL_S
}

/// A headline plus up to [`MAX_LISTED`] bulleted discrepancies, with an elision
/// note when there are more.
fn summarize(headline: &str, items: &[String]) -> String {
    let mut s = format!("{headline}: {} issue(s)", items.len());
    for item in items.iter().take(MAX_LISTED) {
        s.push_str("\n  - ");
        s.push_str(item);
    }
    if items.len() > MAX_LISTED {
        s.push_str(&format!("\n  … and {} more", items.len() - MAX_LISTED));
    }
    s
}
