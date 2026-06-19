//! The discrete-check abstraction every downstream check plugs into.
//!
//! A check is a small, self-identifying unit: it declares its [`Category`] and a
//! `name`, and emits zero or more [`CheckResult`]s for a [`CheckCtx`]. The
//! built-in [`registry`] lists the checks the engine runs. This abstraction and
//! the result/JSON contracts are stable; the category modules (such as the
//! `integrity` module) populate the registry.
//!
//! There is no plugin / dynamic-loading machinery yet: that boundary is deferred
//! until a few real specialized pipelines reveal the right seam. A monolithic
//! registry of discrete trait objects is enough, and extracting a plugin trait
//! from it later is a cheap refactor.

use crate::ir::Sequence;
use crate::profile::Profile;
use crate::result::{Category, CheckResult};

/// Everything a check may inspect.
///
/// The interpreted [`Sequence`] plus the resolved scanner [`Profile`]
/// (`None` in file-only mode, when the hardware checks `skip`). Further inputs
/// (such as an optional expected-spec) are added as additional fields here, so
/// the [`Check::run`] signature never changes as inputs accrue.
pub struct CheckCtx<'a> {
    /// The interpreted sequence under validation.
    pub seq: &'a Sequence,
    /// The resolved scanner profile for hardware/safety checks, if any.
    pub profile: Option<&'a Profile>,
}

/// A discrete, registrable validation unit.
///
/// Implementors are typically zero-sized structs. The default [`id`](Check::id)
/// composes the [`category`](Check::category) slug with [`name`](Check::name)
/// into a stable dotted identifier (e.g. `integrity.raster_alignment`) — the
/// same identifier the JSON contract and the human renderer group by, so keep
/// `name` unique within a category and stable across releases.
pub trait Check {
    /// The category this check belongs to (drives result grouping).
    fn category(&self) -> Category;

    /// A short, stable name, unique within the category.
    fn name(&self) -> &'static str;

    /// The stable result id, `"<category-slug>.<name>"` by default.
    fn id(&self) -> String {
        format!("{}.{}", self.category().slug(), self.name())
    }

    /// A one-line catalog summary: what this check verifies and when it skips.
    /// Used by the default [`docs`](Check::docs) for the common single-result
    /// check; an aggregate check that emits several distinct ids leaves this empty
    /// and overrides `docs()` to describe each id instead.
    fn summary(&self) -> &'static str {
        ""
    }

    /// The catalog entries this check contributes — one [`CheckDoc`] per stable
    /// result `id` it can emit. The default pairs [`id`](Check::id) with
    /// [`summary`](Check::summary) for a one-result check; a check that emits
    /// several ids (the metrics / trajectory / hardware aggregates) overrides this
    /// to enumerate each id and its own one-liner. [`catalog`] collects these
    /// across the registry for `--list-checks`.
    fn docs(&self) -> Vec<CheckDoc> {
        vec![CheckDoc::new(self.id(), self.summary())]
    }

    /// The scanner vendors this check applies to, by [`Profile::vendor`] tag
    /// (e.g. `&["ge"]`). The default `&[]` means **vendor-agnostic** — the check
    /// runs for every profile (and in file-only mode). A non-empty scope makes
    /// the check vendor-specific: [`run_all`] only invokes it when the active
    /// profile's vendor is listed, so a check encoding one vendor's structural
    /// rule stays inert for every other vendor without each check repeating the
    /// gate itself. The [`catalog`] still lists it regardless, so the whole check
    /// space remains discoverable via `--list-checks`.
    fn vendor_scope(&self) -> &'static [&'static str] {
        &[]
    }

    /// Inspect the context and emit results.
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult>;
}

/// One entry in the discoverable check catalog: a stable result `id` and a
/// one-line `summary` of what it verifies (and when it skips). Built from the
/// [`registry`] by [`catalog`] and surfaced by `seq-validate --list-checks`, so an
/// agent can enumerate the check space rather than reverse-engineer it from a lone
/// result `id`.
#[derive(Debug, Clone)]
pub struct CheckDoc {
    /// The stable result id (e.g. `trajectory.geometry_agreement`).
    pub id: String,
    /// One line: what the check verifies, and when it skips.
    pub summary: &'static str,
}

impl CheckDoc {
    /// A catalog entry pairing a result `id` with its one-line `summary`.
    pub fn new(id: impl Into<String>, summary: &'static str) -> Self {
        CheckDoc {
            id: id.into(),
            summary,
        }
    }
}

/// The checks the engine runs, in report order.
///
/// The sequence-integrity checks, the derived-metrics check, the trajectory gate
/// with dual-witness geometry, and the hardware/safety checks each contribute
/// their own category module, concatenated here. Nothing else in the engine
/// needs to change when a check is added.
pub fn registry() -> Vec<Box<dyn Check>> {
    let mut checks = crate::integrity::checks();
    checks.extend(crate::metrics::checks());
    checks.extend(crate::trajectory::checks());
    checks.extend(crate::hardware::checks());
    checks.extend(crate::vendor::checks());
    checks
}

/// Run every registered check against `ctx`, concatenating their results in
/// registry order. A check with a non-empty [`vendor_scope`](Check::vendor_scope)
/// runs only when the active profile's vendor is in its scope; with no profile,
/// vendor-specific checks are skipped entirely (their vendor is unknown).
pub fn run_all(ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
    let vendor = ctx.profile.map(|p| p.vendor.as_str());
    registry()
        .iter()
        .filter(|check| {
            let scope = check.vendor_scope();
            scope.is_empty() || vendor.is_some_and(|v| scope.contains(&v))
        })
        .flat_map(|check| check.run(ctx))
        .collect()
}

/// The discoverable check catalog: one [`CheckDoc`] per result `id` the registry
/// can emit, in registry order. Generated from [`registry`] — each check declares
/// its own entries — so it never drifts from what the engine runs. The CLI's
/// `--list-checks` groups these by [`Category`] for display.
pub fn catalog() -> Vec<CheckDoc> {
    registry().iter().flat_map(|check| check.docs()).collect()
}
