//! `seq-validate-core` — the Pulseq `.seq` validator engine.
//!
//! The foundation every check sits on is a **stable interpreted IR** built on
//! our [`pulseq_parse`] parser ([`ir`]). On top of it sit the surfaces every
//! check plugs into:
//!
//! - the [`result`] model — [`CheckResult`] (`status`/`severity`/`measured`/…),
//! - the [`checks`] abstraction — a discrete [`Check`] unit + an (empty) registry,
//! - the [`report`] aggregation + its **stable JSON** contract ([`Report`]),
//! - the human [`render`]er.
//!
//! The thin `seq-validate` binary crate drives this library and applies the
//! exit-code policy ([`Report::exit_code`]). The category modules populate the
//! registry.
//!
//! ```no_run
//! use seq_validate_core::{checks, Report, Sequence};
//!
//! let seq = Sequence::from_file("scan.seq")?;
//! let results = checks::run_all(&checks::CheckCtx { seq: &seq, profile: None });
//! let report = Report::for_sequence("scan.seq", &seq, results);
//! print!("{}", seq_validate_core::render(&report, false, false));
//! std::process::exit(report.exit_code());
//! # Ok::<(), seq_validate_core::Error>(())
//! ```

pub mod checks;
pub mod ir;
pub mod measurements;
pub mod profile;
pub mod render;
pub mod report;
pub mod result;
pub mod spec;

mod hardware;
mod integrity;
mod metrics;
mod trajectory;
mod vendor;
mod waveform;

pub use ir::{DEFAULT_LARMOR_HZ, Error, Sequence, Signature, TimeRaster, Version, raw_sections};

pub use checks::{Check, CheckCtx, CheckDoc};
pub use measurements::{Geometry, Measurements};
pub use profile::{Pns, Profile};
pub use render::render;
pub use report::{REPORT_SCHEMA, Report, SCHEMA_VERSION, SequenceMeta, Summary};
pub use result::{Category, CheckResult, Severity, Status};
pub use spec::{SPEC_SCHEMA, Spec, Tolerance};

/// The parser crate, re-exported so consumers can reach the `raw` / `model` /
/// `interp` layers directly when the IR isn't enough (debugging, round-trip).
pub use pulseq_parse;

/// `serde_json`, re-exported so consumers can build `measured`/`expected`
/// [`serde_json::Value`]s without depending on a matching version themselves.
pub use serde_json;
