//! Sequence-integrity checks.
//!
//! The cheapest, most certain checks: pure file/IR consistency, no scanner model
//! and no imaging-physics knowledge. They are also *layered behind the parser* —
//! the `pulseq-parse` model layer already rejects the structurally-broken files
//! (dangling event/shape IDs, missing raster definitions, an event longer than
//! its block, negative timings) as a parse error before any check runs.
//! So these checks assert only on what *survives* parsing and is still suspect:
//!
//! - [`RasterAlignment`] — every event timing lands on its declared raster.
//! - [`Timing`] — the computed total duration matches the `TotalDuration`
//!   definition.
//! - [`EventLegality`] — no block transmits and receives at once (RF during ADC).
//! - [`DeadTime`] — RF ring-down / ADC dead-time (scanner-specific; deferred to
//!   the scanner profile, reported here as a `skip`).
//! - [`VersionCheck`] — the `[VERSION]` is one we understand.
//! - [`SignatureCheck`] — the `[SIGNATURE]` md5, if present, recomputes.
//! - [`Definitions`] — raster times are positive and FOV is present and positive.
//!
//! Severity follows the spec: structural corruption (`off-raster`, non-positive
//! raster/FOV) is a `fail`/`error`; uncertain or cosmetic findings (duration or
//! signature mismatch, missing FOV) are a `warn`.

use serde_json::json;

use crate::checks::{Check, CheckCtx};
use crate::result::{Category, CheckResult};

/// The integrity checks, in report order. Wired into [`crate::checks::registry`].
pub(crate) fn checks() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(VersionCheck),
        Box::new(Definitions),
        Box::new(SignatureCheck),
        Box::new(RasterAlignment),
        Box::new(Timing),
        Box::new(EventLegality),
        Box::new(DeadTime),
    ]
}

/// Quotient-deviation tolerance for raster alignment: a timing value sits on its
/// raster when `value / raster` is within this of an integer. Event delays are
/// small (`value/raster` ≲ 1e5), so the floating-point error in the quotient is
/// ~1e-11 — far below this bound — while a real misalignment is ≥ ~0.5 of a
/// raster, far above it.
const RASTER_TOL: f64 = 1e-6;

/// Whether `value` lands on an integer multiple of `raster`. A non-positive
/// raster is reported by [`Definitions`], not here, so it is treated as vacuously
/// aligned to avoid a divide-by-zero and a duplicate finding.
fn on_raster(value: f64, raster: f64) -> bool {
    if raster <= 0.0 {
        return true;
    }
    let q = value / raster;
    (q - q.round()).abs() <= RASTER_TOL
}

/// Every event timing must start/last on its declared raster (§2.6): block
/// durations on `BlockDurationRaster`, gradient edges on `GradientRasterTime`,
/// RF delays on `RadiofrequencyRasterTime`, ADC delay/dwell on `AdcRasterTime`.
///
/// Each event aligns to *its own* raster, not a common one: a propeller readout's
/// RF delay can sit off the gradient raster yet on the (finer) RF raster, and an
/// EPI ADC delay off the gradient raster yet on the ADC raster — both legal.
/// Internal shape samples sit at raster *centers* and are intentionally not
/// checked (only event edges are edge-aligned).
struct RasterAlignment;

impl Check for RasterAlignment {
    fn category(&self) -> Category {
        Category::Integrity
    }
    fn name(&self) -> &'static str {
        "raster_alignment"
    }
    fn summary(&self) -> &'static str {
        "Every event timing lands on its own declared raster; fails when any edge is off-raster."
    }
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let seq = ctx.seq;
        let tr = seq.time_raster;

        let mut checked: u64 = 0;
        let mut misaligned: u64 = 0;
        let mut first: Vec<String> = Vec::new();
        let mut inspect = |i: usize, what: &str, value: f64, raster: f64, rname: &str| {
            checked += 1;
            if !on_raster(value, raster) {
                misaligned += 1;
                if first.len() < 3 {
                    first.push(format!(
                        "block {i} {what} {value:.4e}s is not a multiple of the {rname} raster {raster:.4e}s"
                    ));
                }
            }
        };

        for (i, b) in seq.blocks.iter().enumerate() {
            inspect(i, "duration", b.duration, tr.block, "block");
            if let Some(rf) = &b.rf {
                inspect(i, "RF delay", rf.delay, tr.rf, "RF");
            }
            for (axis, g) in [("GX", &b.gx), ("GY", &b.gy), ("GZ", &b.gz)] {
                if let Some(g) = g {
                    inspect(i, &format!("{axis} delay"), g.delay, tr.grad, "gradient");
                    inspect(
                        i,
                        &format!("{axis} end"),
                        g.delay + g.shape.duration,
                        tr.grad,
                        "gradient",
                    );
                }
            }
            if let Some(adc) = &b.adc {
                inspect(i, "ADC delay", adc.delay, tr.adc, "ADC");
                inspect(i, "ADC dwell", adc.dwell, tr.adc, "ADC");
            }
        }

        let id = self.id();
        let measured = json!({ "checked": checked, "misaligned": misaligned });
        let result = if misaligned == 0 {
            CheckResult::pass(
                id,
                format!("all {checked} event timings align to their declared rasters"),
            )
        } else {
            CheckResult::fail(
                id,
                format!(
                    "{misaligned} of {checked} event timings are off-raster; e.g. {}",
                    first.join("; ")
                ),
            )
        };
        vec![result.with_measured(measured)]
    }
}

/// The cumulative block duration must match the `TotalDuration` definition within
/// tolerance. (Block durations being non-negative and accommodating their events
/// is already guaranteed by the parser; the definition cross-check is what is left
/// to verify here.) A mismatch is a `warn`, not a `fail`: the authoritative
/// duration is the computed sum, and `TotalDuration` is declared convenience
/// metadata that an edit can leave stale.
struct Timing;

impl Check for Timing {
    fn category(&self) -> Category {
        Category::Integrity
    }
    fn name(&self) -> &'static str {
        "timing"
    }
    fn summary(&self) -> &'static str {
        "Computed total duration matches the TotalDuration definition; warns on a mismatch, skips when it is not declared."
    }
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let seq = ctx.seq;
        let id = self.id();
        let computed = seq.total_duration;

        let Some(raw) = seq.definitions.get("TotalDuration") else {
            return vec![
                CheckResult::skip(id, "no TotalDuration definition to cross-check")
                    .with_measured(computed),
            ];
        };
        let Ok(declared) = raw.trim().parse::<f64>() else {
            return vec![
                CheckResult::warn(
                    id,
                    format!("TotalDuration definition {raw:?} is not a number"),
                )
                .with_measured(computed),
            ];
        };

        let tol = 1e-6 * declared.abs().max(1.0) + 1e-9;
        let result = if (computed - declared).abs() <= tol {
            CheckResult::pass(
                id,
                "computed total duration matches the TotalDuration definition",
            )
        } else {
            CheckResult::warn(
                id,
                format!(
                    "computed total duration {computed:.6}s disagrees with the TotalDuration definition {declared:.6}s"
                ),
            )
        };
        vec![result.with_measured(computed).with_expected(declared)]
    }
}

/// No block may transmit and receive at once — an RF event and an ADC event in
/// the same block. This is flagged as a `warn` (transmit-during-receive is
/// usually a mistake, but the format permits it). Dangling event/shape references
/// — the other half of "event legality" — cannot occur here: the parser resolves
/// every reference before the IR exists, so an unresolved one cannot reach here.
struct EventLegality;

impl Check for EventLegality {
    fn category(&self) -> Category {
        Category::Integrity
    }
    fn name(&self) -> &'static str {
        "event_legality"
    }
    fn summary(&self) -> &'static str {
        "No block transmits and receives at once (RF during ADC); warns when one does."
    }
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let seq = ctx.seq;
        let id = self.id();

        let mut count: u64 = 0;
        let mut first: Vec<usize> = Vec::new();
        for (i, b) in seq.blocks.iter().enumerate() {
            if b.rf.is_some() && b.adc.is_some() {
                count += 1;
                if first.len() < 3 {
                    first.push(i);
                }
            }
        }

        let result = if count == 0 {
            CheckResult::pass(
                id,
                "no block transmits and receives at once; all event references resolve",
            )
        } else {
            CheckResult::warn(
                id,
                format!(
                    "{count} block(s) contain a simultaneous RF and ADC (transmit during receive); e.g. block {first:?}"
                ),
            )
            .with_measured(count)
        };
        vec![result]
    }
}

/// RF ring-down and ADC dead-time are scanner-specific limits, not file
/// properties, so the hard check lives against a scanner profile. Here
/// it is honestly reported as a `skip` so the dimension is visible in the report.
struct DeadTime;

impl Check for DeadTime {
    fn category(&self) -> Category {
        Category::Integrity
    }
    fn name(&self) -> &'static str {
        "dead_time"
    }
    fn summary(&self) -> &'static str {
        "RF ring-down / ADC dead-time are scanner-specific; always skips here and defers to hardware.dead_time when a profile is given."
    }
    fn run(&self, _ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        vec![CheckResult::skip(
            self.id(),
            "RF ring-down / ADC dead-time are scanner-specific; checked against a scanner profile",
        )]
    }
}

/// The `[VERSION]` must be one the checks understand. The parser already gates to
/// Pulseq 1.5.x, so this passes for any file that reaches the IR; it stays as a
/// defensive `warn` should a future parser admit a version these checks are not
/// tuned for, and it surfaces the version in the report.
struct VersionCheck;

impl Check for VersionCheck {
    fn category(&self) -> Category {
        Category::Integrity
    }
    fn name(&self) -> &'static str {
        "version"
    }
    fn summary(&self) -> &'static str {
        "The [VERSION] is one the checks understand (Pulseq 1.5.x); warns on an unrecognized version."
    }
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let v = &ctx.seq.version;
        let id = self.id();
        let result = if (v.major, v.minor) == (1, 5) {
            CheckResult::pass(id, format!("recognized Pulseq {v}"))
        } else {
            CheckResult::warn(
                id,
                format!("unrecognized Pulseq version {v}; integrity checks are tuned for 1.5.x"),
            )
        };
        vec![result.with_measured(v.to_string())]
    }
}

/// The `[SIGNATURE]` md5, if present, must recompute. A mismatch is a `warn` (the
/// file may have been edited post-export); an absent signature or an algorithm we
/// can't reproduce is a `skip`.
struct SignatureCheck;

impl Check for SignatureCheck {
    fn category(&self) -> Category {
        Category::Integrity
    }
    fn name(&self) -> &'static str {
        "signature"
    }
    fn summary(&self) -> &'static str {
        "The [SIGNATURE] md5, if present, recomputes; warns on a mismatch, skips when absent or the algorithm is unsupported."
    }
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let id = self.id();
        let Some(sig) = &ctx.seq.signature else {
            return vec![CheckResult::skip(id, "no [SIGNATURE] section")];
        };
        let Some(computed) = &sig.computed_hash else {
            return vec![
                CheckResult::skip(
                    id,
                    format!(
                        "signature algorithm {:?} not verified (only md5 supported)",
                        sig.algo
                    ),
                )
                .with_measured(sig.declared_hash.clone()),
            ];
        };

        let result = if computed.eq_ignore_ascii_case(sig.declared_hash.trim()) {
            CheckResult::pass(id, format!("{} signature verified", sig.algo))
                .with_measured(computed.clone())
        } else {
            CheckResult::warn(
                id,
                format!(
                    "{} signature mismatch: file declares {}, recomputed {computed}",
                    sig.algo, sig.declared_hash
                ),
            )
            .with_measured(computed.clone())
            .with_expected(sig.declared_hash.clone())
        };
        vec![result]
    }
}

/// Definitions sanity: every raster time must be positive, and FOV present and
/// positive. The mandatory raster definitions are parser-enforced to *exist*, but
/// not to be positive; FOV is optional, so its absence is a `warn` (geometry then
/// assumes unit FOV) while a non-positive raster or FOV axis is a structural
/// `fail`.
struct Definitions;

impl Check for Definitions {
    fn category(&self) -> Category {
        Category::Integrity
    }
    fn name(&self) -> &'static str {
        "definitions"
    }
    fn summary(&self) -> &'static str {
        "Required raster times are positive and FOV is present and positive; fails on a non-positive raster/FOV, warns when FOV is absent."
    }
    fn run(&self, ctx: &CheckCtx<'_>) -> Vec<CheckResult> {
        let seq = ctx.seq;
        let id = self.id();
        let tr = seq.time_raster;

        let rasters = [
            ("GradientRasterTime", tr.grad),
            ("RadiofrequencyRasterTime", tr.rf),
            ("AdcRasterTime", tr.adc),
            ("BlockDurationRaster", tr.block),
        ];
        if let Some((name, value)) = rasters.iter().copied().find(|(_, v)| *v <= 0.0) {
            return vec![CheckResult::fail(
                id,
                format!("non-positive raster: {name} = {value:e}s"),
            )];
        }

        if !seq.definitions.contains_key("FOV") {
            return vec![CheckResult::warn(
                id,
                "no FOV defined; geometry assumes a unit FOV",
            )];
        }
        let [fx, fy, fz] = seq.fov;
        if fx <= 0.0 || fy <= 0.0 || fz <= 0.0 {
            return vec![CheckResult::fail(
                id,
                format!("non-positive FOV axis: [{fx:e}, {fy:e}, {fz:e}] m"),
            )];
        }

        vec![
            CheckResult::pass(id, "required definitions present; rasters and FOV positive")
                .with_measured(json!({ "fov_m": [fx, fy, fz] })),
        ]
    }
}
