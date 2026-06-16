// This module describes a pulseq sequence, boiled down to the necessary info.
use std::{
    collections::HashMap,
    ops::{Add, Mul, Sub},
    path::Path,
    sync::Arc,
};

use num_complex::Complex64;

use crate::{
    error::{self, ConversionError, EventType, ValidationError},
    raw::{self, Section},
};

mod convert;

pub mod extensions;
pub use extensions::Extension;

pub struct Sequence {
    pub time_raster: TimeRaster,
    pub name: Option<String>,
    pub fov: Option<(f64, f64, f64)>,
    pub definitions: HashMap<String, String>,
    pub blocks: Vec<Block>,
    /// Soft-delay `id → hint` table, collected from the `delays` extension
    pub soft_delay_hints: HashMap<u32, String>,
}

impl Sequence {
    pub fn from_parsed_file(sections: Vec<Section>) -> Result<Self, error::Error> {
        let tmp = convert::from_raw(sections)?;
        tmp.validate()?;
        Ok(tmp)
    }

    pub fn from_source(source: &str) -> Result<Self, error::Error> {
        Self::from_parsed_file(raw::parse_file(source)?)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, error::Error> {
        let source = std::fs::read_to_string(path)?;
        Self::from_source(&source)
    }

    pub fn validate(&self) -> Result<(), error::ValidationError> {
        // NOTE: We could check if block IDs are in some order or at least not
        // duplicated, but as they are never really used, this might be too strict

        // Check if no event is longer than the duration of its block
        for block in &self.blocks {
            // Passes through dur if its Some(..) and more than block.duration
            let check = |dur: Option<f64>, ty: EventType| {
                dur.map_or(Ok(()), |dur| {
                    if dur > block.duration + f64::EPSILON {
                        Err(ValidationError::EventTooLong {
                            ty,
                            block_id: block.id,
                            dur,
                            block_dur: block.duration,
                        })
                    } else {
                        Ok(())
                    }
                })
            };
            let grad_raster = self.time_raster.grad;

            check(
                block.rf.as_ref().map(|rf| rf.duration(self.time_raster.rf)),
                EventType::Rf,
            )?;
            check(
                block.gx.as_ref().map(|gx| gx.duration(grad_raster)),
                EventType::Gx,
            )?;
            check(
                block.gy.as_ref().map(|gy| gy.duration(grad_raster)),
                EventType::Gy,
            )?;
            check(
                block.gz.as_ref().map(|gz| gz.duration(grad_raster)),
                EventType::Gz,
            )?;
            check(block.adc.as_ref().map(|adc| adc.duration()), EventType::Adc)?;
        }

        // Check things like no negative times
        for block in &self.blocks {
            let id = block.id;
            use EventType::*;
            block.gx.as_ref().map_or(Ok(()), |x| x.validate(Gx, id))?;
            block.gy.as_ref().map_or(Ok(()), |x| x.validate(Gy, id))?;
            block.gz.as_ref().map_or(Ok(()), |x| x.validate(Gz, id))?;
            block.adc.as_ref().map_or(Ok(()), |x| x.validate(id))?;
        }

        Ok(())
    }
}

/// Event raster times in seconds, mandatory in the `[DEFINITIONS]` of every
/// Pulseq 1.4+ file (this crate targets 1.5+).
pub struct TimeRaster {
    pub grad: f64,
    pub rf: f64,
    pub adc: f64,
    pub block: f64,
}

pub struct Block {
    /// Blocks are stored in a simple vector, instead of a HashMap with their ID
    /// as value, because they are not referenced but executed top to bottom.
    /// Its own ID is stored inside of the Block for error reporting.
    pub id: u32,
    pub duration: f64,
    pub rf: Option<Arc<Rf>>,
    pub gx: Option<Arc<Gradient>>,
    pub gy: Option<Arc<Gradient>>,
    pub gz: Option<Arc<Gradient>>,
    pub adc: Option<Arc<Adc>>,
    pub ext: Vec<Extension>,
}

pub struct Rf {
    /// Unit: `[Hz]`
    pub amp: f64,
    /// (rel_to_larmor, offset) - Unit: (`[rad/Hz]`, `[rad]`)
    pub phase: (f64, f64),
    /// Unit: `[s]`
    pub delay: f64,
    /// Unit: `[s]`
    pub center: f64,
    /// (rel_to_larmor, offset) - Unit: (`[Hz/Hz]`, `[Hz]`)
    pub freq: (f64, f64),
    /// Combined amplitude × exp(i × phase) shape
    pub shape: Arc<Shape<Complex64>>,
    /// pTx extension: per-channel amplitude × exp(i × phase)
    pub shim_shape: Option<Arc<Shape<Complex64>>>,
    pub rf_use: RfUse,
}

/// Sparse sample representation: each pair `(time[i], amp[i])` is a breakpoint
/// at the sample's *center* in raster ticks; values between breakpoints are
/// linearly interpolated (see `interpolate`). The shape's total active extent
/// is `duration` ticks, which can be larger than `*time.last()` (e.g. when
/// samples sit at centers `[0.5, …, N-0.5]` the duration is `N`, not `N-0.5`).
///
/// SPEC NOTE: pulseq has three time-shape modes, all decoded into this same
/// representation by `ShapeLib::get`:
/// - `time_id = 0`: uniform centers `time = [0.5, 1.5, …, N-0.5]`, duration `N`.
/// - `time_id = -1` (pulseq 1.5+): half-tick grid `time = [0.5, 1.0, 1.5, …,
///   N-0.5]` with `M = 2N-1` samples (M must be odd), duration `N = (M+1)/2`.
/// - `time_id = x > 0`: explicit sample times from shape `x`, duration =
///   `*time.last()` (typically `0` for the first entry and `N` for the last,
///   per the pulseq Free-gradient convention — we don't enforce this).
///
/// Invariants (enforced by `Shape::new`):
/// - `time.len() == amp.len()`
/// - `time` is non-empty and strictly increasing
/// - all `time[i] >= 0.0` and `time[i] <= duration as f64`
pub struct Shape<T> {
    /// Sample positions in raster ticks (may be fractional for `time_id = -1`).
    pub time: Vec<f64>,
    /// Sample values aligned with `time` 1:1.
    pub amp: Vec<T>,
    /// Total active extent in raster ticks. Not necessarily `*time.last()`.
    pub duration: u32,
}

impl<T> Shape<T> {
    /// Validate invariants. `duration` is supplied by the caller because for
    /// the centered conventions (`time_id ∈ {0, -1}`) it doesn't equal
    /// `*time.last()`; the conversion layer is responsible for picking it.
    pub fn new(time: Vec<f64>, amp: Vec<T>, duration: u32) -> Result<Self, ConversionError> {
        if time.len() != amp.len() {
            return Err(ConversionError::TimeShapeMismatch {
                shape_len: amp.len(),
                time_len: time.len(),
            });
        }
        if time.is_empty() {
            return Err(ConversionError::EmptyShape);
        }
        if !time.array_windows().all(|[w1, w2]| w1 < w2) {
            return Err(ConversionError::TimeShapeNonIncreasing);
        }
        let dur_f = duration as f64;
        if time.iter().any(|&t| t < 0.0 || t > dur_f) {
            return Err(ConversionError::TimeShapeNegative);
        }
        Ok(Self {
            time,
            amp,
            duration,
        })
    }
}

impl<T> Shape<T>
where
    T: Copy + Add<Output = T> + Sub<Output = T> + Mul<f64, Output = T>,
{
    /// Linear interpolation at `time` (in raster ticks). Returns `amp[0]` for
    /// `time <= time[0]` and `*amp.last()` for `time >= time.last()`. Lifted
    /// from the previous `expand_shape` so callers (simulators or scanner
    /// raster expansion) can sample at any point without re-implementing it.
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

impl Shape<Complex64> {
    /// Used to compute rf centers in pre 1.5 sequences.
    /// This is a very rough approximation - it assumes the center is the point with the highest amplitude.
    /// Returns the index into the shape that is closest to the center of the pulse (need to multiply with rf raster).
    pub fn calc_center(&self) -> usize {
        self.amp
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.norm().total_cmp(&b.1.norm()))
            .map_or(0, |(i, _)| i)
    }
}

pub use crate::raw::RfUse;

pub enum Gradient {
    Free {
        /// Unit: `[Hz/m]`
        amp: f64,
        /// Unit: `[s]`
        delay: f64,
        // Shapes
        shape: Arc<Shape<f64>>,
    },
    Trap {
        /// Unit: `[Hz/m]`
        amp: f64,
        /// Unit: `[s]`
        rise: f64,
        /// Unit: `[s]`
        flat: f64,
        /// Unit: `[s]`
        fall: f64,
        /// Unit: `[s]`
        delay: f64,
    },
}

pub struct Adc {
    pub num: u32,
    /// Unit: `[s]`
    pub dwell: f64,
    /// Unit: `[s]`
    pub delay: f64,
    /// (rel_to_larmor, offset) - Unit: (`[Hz/Hz]`, `[Hz]`)
    pub freq: (f64, f64),
    /// (rel_to_larmor, offset) - Unit: (`[rad/Hz]`, `[rad]`)
    pub phase: (f64, f64),
    /// No examples given - assuming `[rad]` shape?
    pub phase_shape: Option<Arc<Shape<f64>>>,
}

// Helper functions and other impls

impl Rf {
    pub fn duration(&self, rf_raster: f64) -> f64 {
        self.delay + self.shape.duration as f64 * rf_raster
    }
}

impl Gradient {
    pub fn duration(&self, grad_raster: f64) -> f64 {
        match self {
            Gradient::Free { shape, delay, .. } => delay + shape.duration as f64 * grad_raster,
            Gradient::Trap {
                rise,
                flat,
                fall,
                delay,
                ..
            } => delay + rise + flat + fall,
        }
    }

    fn validate(&self, ty: EventType, block_id: u32) -> Result<(), error::ValidationError> {
        match self {
            Gradient::Free { delay, .. } => {
                if *delay < 0.0 {
                    Err(ValidationError::NegativeTiming {
                        ty,
                        block_id,
                        timing: *delay,
                    })
                } else {
                    Ok(())
                }
            }
            Gradient::Trap {
                rise,
                flat,
                fall,
                delay,
                ..
            } => {
                if *rise < 0.0 {
                    Err(ValidationError::NegativeTiming {
                        ty,
                        block_id,
                        timing: *rise,
                    })
                } else if *flat < 0.0 {
                    Err(ValidationError::NegativeTiming {
                        ty,
                        block_id,
                        timing: *flat,
                    })
                } else if *fall < 0.0 {
                    Err(ValidationError::NegativeTiming {
                        ty,
                        block_id,
                        timing: *fall,
                    })
                } else if *delay < 0.0 {
                    Err(ValidationError::NegativeTiming {
                        ty,
                        block_id,
                        timing: *delay,
                    })
                } else {
                    Ok(())
                }
            }
        }
    }
}

impl Adc {
    pub fn duration(&self) -> f64 {
        self.delay + self.num as f64 * self.dwell
    }

    fn validate(&self, block_id: u32) -> Result<(), error::ValidationError> {
        if self.dwell < 0.0 {
            Err(ValidationError::NegativeTiming {
                ty: EventType::Adc,
                block_id,
                timing: self.dwell,
            })
        } else if self.delay < 0.0 {
            Err(ValidationError::NegativeTiming {
                ty: EventType::Adc,
                block_id,
                timing: self.delay,
            })
        } else {
            Ok(())
        }
    }
}
