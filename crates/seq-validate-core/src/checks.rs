//! The discrete-check abstraction every downstream check plugs into.
//!
//! A check is a small, self-identifying unit: it declares its [`Category`] and a
//! `name`, and emits zero or more [`CheckResult`]s for a [`CheckCtx`]. The
//! built-in [`registry`] lists the checks the engine runs; it is intentionally
//! **empty** here — Step 2 (`docs/02-crate-skeleton.md`) locks this abstraction
//! and the result/JSON contracts, and Steps 3–6 populate it.
//!
//! There is no plugin / dynamic-loading machinery yet: that boundary is deferred
//! until a few real specialized pipelines reveal the right seam (see the
//! deferred-modularity decision in `docs/00-overview.md`). A monolithic registry
//! of discrete trait objects is enough, and extracting a plugin trait from it
//! later is a cheap refactor.

use crate::ir::Sequence;
use crate::result::{Category, CheckResult};

/// Everything a check may inspect.
///
/// Currently just the interpreted [`Sequence`]. Later steps add the optional
/// expected-spec (Step 7) and scanner profile (Step 6) as fields here, so the
/// [`Check::run`] signature never changes as inputs accrue.
pub struct CheckCtx<'a> {
    /// The interpreted sequence under validation.
    pub seq: &'a Sequence,
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

    /// Inspect the context and emit results.
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult>;
}

/// The checks the engine runs, in report order.
///
/// Empty until Step 3. A new check is registered by pushing its boxed instance
/// here; nothing else in the engine needs to change.
pub fn registry() -> Vec<Box<dyn Check>> {
    Vec::new()
}

/// Run every registered check against `ctx`, concatenating their results in
/// registry order.
pub fn run_all(ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
    registry().iter().flat_map(|check| check.run(ctx)).collect()
}
