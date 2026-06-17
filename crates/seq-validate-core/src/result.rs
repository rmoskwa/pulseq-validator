//! The result model — the uniform shape every check emits.
//!
//! A single check produces one or more [`CheckResult`]s, each carrying a stable
//! [`id`](CheckResult::id), an outcome [`Status`], a [`Severity`], an optional
//! `measured`/`expected` pair, and a human `message`. This is the spec's exact
//! six-field model (`docs/02-crate-skeleton.md`); it is the atom the JSON
//! contract and the human renderer are built from, so its field set is treated
//! as breaking to change.
//!
//! [`Category`] is **not** part of the serialized result: it is a presentation
//! grouping derived from the `id` prefix (`integrity.raster_alignment` →
//! [`Category::Integrity`]). Keeping it out of `CheckResult` lets the JSON stay
//! flat while the renderer still groups by category.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The outcome of a check.
///
/// `pass`/`fail` are the asserted outcomes; `warn` flags something noteworthy
/// that is not a hard failure; `skip` records an inapplicable check (a
/// first-class result, never a failure — see the dual-witness geometry note in
/// `docs/00-overview.md`). Only `fail` drives a nonzero exit code.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    Warn,
    Skip,
}

/// How seriously to treat a non-passing result.
///
/// Severity is orthogonal to [`Status`]: it conveys *importance* to a consumer
/// (a `fail` may be an `error`, a `warn` an `info`), where status conveys the
/// *outcome*. It does not affect the exit code.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
    Info,
}

/// One check's verdict on one thing.
///
/// Construct via the [`pass`](CheckResult::pass) / [`fail`](CheckResult::fail) /
/// [`warn`](CheckResult::warn) / [`skip`](CheckResult::skip) builders, then
/// optionally attach values with [`with_measured`](CheckResult::with_measured),
/// [`with_expected`](CheckResult::with_expected), and override the default
/// severity with [`with_severity`](CheckResult::with_severity).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CheckResult {
    /// Stable `<category>.<name>` identifier (e.g. `integrity.raster_alignment`).
    /// The category prefix is what the renderer groups by; see [`Category`].
    pub id: String,
    /// The outcome.
    pub status: Status,
    /// What the check observed, if it measured anything (`null` otherwise).
    pub measured: Option<Value>,
    /// What the check expected — typically from a spec; `null` when there is no
    /// expectation (file-only mode).
    pub expected: Option<Value>,
    /// How seriously to treat a non-pass.
    pub severity: Severity,
    /// Human-readable explanation of the result.
    pub message: String,
}

impl CheckResult {
    /// A passing result (default severity [`Severity::Info`]).
    pub fn pass(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, Status::Pass, Severity::Info, message)
    }

    /// A failing result (default severity [`Severity::Error`]). The only status
    /// that drives a nonzero exit code.
    pub fn fail(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, Status::Fail, Severity::Error, message)
    }

    /// A warning: noteworthy but not a hard failure (default severity
    /// [`Severity::Warn`]).
    pub fn warn(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, Status::Warn, Severity::Warn, message)
    }

    /// An inapplicable check (default severity [`Severity::Info`]). Never a
    /// failure.
    pub fn skip(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, Status::Skip, Severity::Info, message)
    }

    fn new(
        id: impl Into<String>,
        status: Status,
        severity: Severity,
        message: impl Into<String>,
    ) -> Self {
        CheckResult {
            id: id.into(),
            status,
            measured: None,
            expected: None,
            severity,
            message: message.into(),
        }
    }

    /// Attach the measured value.
    pub fn with_measured(mut self, value: impl Into<Value>) -> Self {
        self.measured = Some(value.into());
        self
    }

    /// Attach the expected value.
    pub fn with_expected(mut self, value: impl Into<Value>) -> Self {
        self.expected = Some(value.into());
        self
    }

    /// Override the default severity for this status.
    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}

/// A check category — the four families from `docs/00-overview.md`, plus an
/// `Other` catch-all.
///
/// This is a **presentation grouping only**: it is derived from a result's `id`
/// prefix ([`from_id`](Category::from_id)) and is not serialized. A check
/// declares its category (see [`crate::checks::Check`]); the default `id` is
/// `"<slug>.<name>"`, which is what keeps the prefix and the category in sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// Raster alignment, block/timing consistency, overlaps, version sanity.
    Integrity,
    /// Derived imaging metrics: TE, TR, flip angle, FOV, matrix, …
    Metrics,
    /// K-space trajectory extent/coverage/uniformity, 2D-vs-3D.
    Trajectory,
    /// Hardware/safety limits against a scanner profile.
    Hardware,
    /// Anything whose id prefix is unrecognized.
    Other,
}

impl Category {
    /// Categories in the order the human renderer prints them.
    pub const DISPLAY_ORDER: &'static [Category] = &[
        Category::Integrity,
        Category::Metrics,
        Category::Trajectory,
        Category::Hardware,
        Category::Other,
    ];

    /// The lowercase id prefix (e.g. `integrity`).
    pub fn slug(self) -> &'static str {
        match self {
            Category::Integrity => "integrity",
            Category::Metrics => "metrics",
            Category::Trajectory => "trajectory",
            Category::Hardware => "hardware",
            Category::Other => "other",
        }
    }

    /// The human heading the renderer prints for this category.
    pub fn title(self) -> &'static str {
        match self {
            Category::Integrity => "Sequence integrity",
            Category::Metrics => "Derived metrics",
            Category::Trajectory => "K-space trajectory",
            Category::Hardware => "Hardware & safety",
            Category::Other => "Other",
        }
    }

    /// Classify a result id by its prefix (the text before the first `.`).
    /// Unrecognized prefixes map to [`Category::Other`].
    pub fn from_id(id: &str) -> Category {
        match id.split('.').next().unwrap_or("") {
            "integrity" => Category::Integrity,
            "metrics" => Category::Metrics,
            "trajectory" => Category::Trajectory,
            "hardware" => Category::Hardware,
            _ => Category::Other,
        }
    }
}
