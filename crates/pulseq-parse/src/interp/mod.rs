use crate::error::{InterpreterError, InterpreterWarning};
use num_complex::Complex64;
use std::collections::HashMap;
use std::sync::Arc;

mod convert;
mod math;

pub use math::{Quaternion, Transform};

pub struct Sequence {
    pub name: Option<String>,
    /// Per-axis FOV `[m]` - based on [1, 1, 1] if .seq file did not define FOV
    pub fov: [f64; 3],
    pub blocks: Vec<Block>,
}

impl Sequence {
    /// Parameters:
    /// - `fov`: 3x4 affine transform applied to the sequence, must be unitary.
    /// - `larmor`: Larmor frequency `[Hz]` - used for relative freq/phase.
    /// - `soft_delays`: values for soft delays, keyed by their hint string.
    pub fn from_seq(
        seq: &crate::model::Sequence,
        fov: Transform,
        larmor: f64,
        soft_delays: HashMap<String, f64>,
    ) -> Result<(Self, Vec<InterpreterWarning>), InterpreterError> {
        if !fov.validate() {
            return Err(InterpreterError::NonUnitaryFov);
        }

        let mut warnings = Vec::new();
        let seq = convert::convert(seq, fov, larmor, soft_delays, &mut warnings)?;
        Ok((seq, warnings))
    }

    /// Absolute start time `[s]` of each block, in execution order. Blocks play
    /// back to back, so `starts[i]` is the cumulative sum of the durations of
    /// blocks `0..i`; `starts[0]` is `0.0`. A single O(n) pass.
    pub fn block_starts(&self) -> Vec<f64> {
        let mut starts = Vec::with_capacity(self.blocks.len());
        let mut t = 0.0;
        for b in &self.blocks {
            starts.push(t);
            t += b.duration;
        }
        starts
    }

    /// Total sequence duration `[s]`: the sum of all block durations.
    pub fn duration(&self) -> f64 {
        self.blocks.iter().map(|b| b.duration).sum()
    }
}

pub struct Block {
    /// `[s]`
    pub duration: f64,
    pub rf: Option<Rf>,
    pub gx: Option<Gradient>,
    pub gy: Option<Gradient>,
    pub gz: Option<Gradient>,
    pub adc: Option<Adc>,
    /// Triggers from the `triggers` extension active in this block.
    pub triggers: Vec<Trigger>,
    /// Label state from the `labelset` / `labelinc` extension.
    pub labels: BlockLabels,
}

#[derive(Default, Clone, Copy)]
pub struct BlockLabels {
    /// Repetition gating from the `ONCE` label.
    pub once: Once,
    /// `PMC` label - block can be prospectively motion-corrected.
    pub pmc: bool,
    /// `TRID` counter - marks the start (and identity) of a repeated seq part
    pub trid: i32,
}

/// tells if block should be measured only in the first or last repetition
#[derive(Default, Clone, Copy)]
pub enum Once {
    #[default]
    Always = 0,
    First = 1,
    Last = 2,
}

pub struct Trigger {
    pub typ: u32,
    pub channel: u32,
    /// `[s]`
    pub delay: f64,
    /// `[s]`
    pub duration: f64,
}

pub struct Rf {
    /// `[Hz]`
    pub amp: f64,
    /// `[rad]` - relative and offset components combined via the larmor frequency.
    pub phase: f64,
    /// `[s]`
    pub delay: f64,
    /// `[s]`
    pub center: f64,
    /// `[Hz]` - relative and offset components combined via the larmor frequency.
    pub freq: f64,
    /// Combined amplitude × exp(i × phase) base shape.
    pub shape: Arc<Shape<Complex64>>,
    /// Per-channel shim weights, one complex value per transmit channel.
    /// Sourced from either the official `rf_shims` extension on the block or
    /// the pTx (Martin) shim shape attached to the seq RF.
    /// A missing shim is represented as `vec![Complex64::new(1.0, 0.0)]`.
    pub shims: Vec<Complex64>,
    /// forwarded from raw sequence - specifies what purpose this pulse has.
    pub rf_use: crate::raw::RfUse,
}

/// Remove type distinction - Trap gradients are converted to Free with time shape
pub struct Gradient {
    /// `[Hz/m]` - already FOV-scaled and rotated.
    pub amp: f64,
    /// `[s]`
    pub delay: f64,
    pub shape: Arc<Shape<f64>>,
}

pub struct Adc {
    pub num: u32,
    /// `[s]`
    pub dwell: f64,
    /// `[s]`
    pub delay: f64,
    /// `[Hz]` - relative and offset components combined via the larmor frequency.
    pub freq: f64,
    /// `[rad]` - relative and offset components combined via the larmor frequency.
    pub phase: f64,
    /// Optional per-sample phase modulation, applied on top of `phase`.
    pub phase_shape: Option<Arc<Shape<f64>>>,
    /// Snapshot of the label state at the time this ADC fires.
    pub labels: Labels,
}

/// Per-ADC label state. Counters reflect the running value at this ADC,
/// boolean flags are sticky until cleared by another `LABELSET`.
#[derive(Default, Clone, Copy)]
pub struct Labels {
    pub slc: i32,
    pub seg: i32,
    pub rep: i32,
    pub avg: i32,
    pub set: i32,
    pub eco: i32,
    pub phs: i32,
    pub lin: i32,
    pub par: i32,
    pub acq: i32,
    pub nav: bool,
    pub rev: bool,
    pub sms: bool,
    pub ref_: bool,
    pub ima: bool,
    pub off: bool,
    pub noise: bool,
}

/// Sparse sample representation, mirroring `model::Shape` but with `time` and
/// `duration` in seconds (already multiplied by the appropriate raster during
/// seq→int lowering). `int` no longer carries a `time_raster`, since every
/// shape already knows its own absolute timing.
///
/// Invariants: same as `model::Shape`. `duration` is the total active extent in
/// seconds and may be larger than `*time.last()` (e.g. for shapes with samples
/// at centers `[0.5, 1.5, …, N-0.5] * raster`, duration is `N * raster`).
pub struct Shape<T> {
    /// Absolute times in seconds for each sample.
    pub time: Vec<f64>,
    /// Sample values aligned with `time` 1:1.
    pub amp: Vec<T>,
    /// Total active extent in seconds. Not necessarily `*time.last()`.
    pub duration: f64,
}

impl<T> Shape<T>
where
    T: Copy
        + std::ops::Add<Output = T>
        + std::ops::Sub<Output = T>
        + std::ops::Mul<f64, Output = T>,
{
    /// Linear interpolation at `time` (in seconds). Returns `amp[0]` for
    /// `time <= time[0]` and `*amp.last()` for `time >= time.last()`.
    #[allow(clippy::indexing_slicing)]
    pub fn interpolate(&self, time: f64) -> T {
        if time <= self.time[0] {
            return self.amp[0];
        }
        let last = self.time.len() - 1;
        if time >= self.time[last] {
            return self.amp[last];
        }
        let idx = self.time.iter().position(|&t| t >= time).unwrap_or(last);
        let t0 = self.time[idx - 1];
        let t1 = self.time[idx];
        let frac = (time - t0) / (t1 - t0);
        self.amp[idx - 1] + (self.amp[idx] - self.amp[idx - 1]) * frac
    }
}
