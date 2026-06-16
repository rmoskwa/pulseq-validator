//! `seq-validate-core` — the Pulseq `.seq` validator engine.
//!
//! Step 1 (see `docs/01-vendor-parser.md`) establishes the foundation every
//! downstream check sits on: a **stable interpreted IR** built on our
//! [`pulseq_parse`] parser. Later steps add the result model, the checks, and
//! the CLI on top of this same crate.
//!
//! ```no_run
//! let seq = seq_validate_core::Sequence::from_file("scan.seq")?;
//! println!("{} blocks, {:.3} s", seq.blocks.len(), seq.total_duration);
//! for (start, block) in seq.timed() {
//!     if block.adc.is_some() {
//!         println!("ADC at {start:.6} s");
//!     }
//! }
//! # Ok::<(), seq_validate_core::Error>(())
//! ```

pub mod checks;
pub mod ir;

pub use ir::{raw_sections, Error, Sequence, TimeRaster, Version, DEFAULT_LARMOR_HZ};

/// The parser crate, re-exported so consumers can reach the `raw` / `model` /
/// `interp` layers directly when the IR isn't enough (debugging, round-trip).
pub use pulseq_parse;
