//! `seq-validate-core` — the Pulseq `.seq` validator engine.
//!
//! Step 1 (`docs/01-vendor-parser.md`) established the foundation every check
//! sits on: a **stable interpreted IR** built on our [`pulseq_parse`] parser
//! ([`ir`]). Step 2 (`docs/02-crate-skeleton.md`) adds the surfaces every check
//! plugs into:
//!
//! - the [`result`] model — [`CheckResult`] (`status`/`severity`/`measured`/…),
//! - the [`checks`] abstraction — a discrete [`Check`] unit + an (empty) registry.

pub mod checks;
pub mod ir;
pub mod result;

pub use ir::{DEFAULT_LARMOR_HZ, Error, Sequence, TimeRaster, Version, raw_sections};

pub use checks::{Check, CheckCtx};
pub use result::{Category, CheckResult, Severity, Status};

/// The parser crate, re-exported so consumers can reach the `raw` / `model` /
/// `interp` layers directly when the IR isn't enough (debugging, round-trip).
pub use pulseq_parse;

/// `serde_json`, re-exported so consumers can build `measured`/`expected`
/// [`serde_json::Value`]s without depending on a matching version themselves.
pub use serde_json;
