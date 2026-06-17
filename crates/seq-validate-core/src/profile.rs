//! The scanner-profile subsystem.
//!
//! A [`Profile`] is a curated, **sourced and versioned** set of per-scanner
//! hardware limits the `.seq` file does not itself carry: peak gradient / slew,
//! peak B1, the hardware raster grid, RF/ADC dead times, and (where modelled) the
//! PNS coil parameters. The hardware/safety checks (`crate::hardware`) validate a
//! sequence against the resolved profile; with no profile they `skip`, because a
//! wrong scanner must never be silently assumed.
//!
//! Profiles are bundled as Rust data rather than loaded from disk: the citation
//! for every number lives in the doc-comment beside it (the acceptance criterion
//! "each bundled profile number traces to a cited source"), the set is small and
//! compile-checked, and a user [`override`](Profile::apply_override) covers the
//! "this one field differs on my system" case curation can't anticipate. A future
//! step that loads external profile/spec YAML can reuse this same type.
//!
//! ## Resolution order
//!
//! [`resolve`] implements it: an explicit `--profile <name>` wins; else hardware
//! limits embedded in the file's `[DEFINITIONS]` ([`Profile::from_definitions`]);
//! else `Ok(None)` — the checks then `skip` with a clear, non-silent message. An
//! explicit but unknown profile name is an error, not a silent fallback.

use crate::ir::Sequence;

/// PNS (peripheral-nerve-stimulation) coil parameters for the IEC 60601-2-33:2022
/// nerve-impulse-response model (see `crate::hardware`). Only some profiles carry
/// one (it is coil-specific); a profile without it `skip`s the PNS check.
#[derive(Clone, Debug, PartialEq)]
pub struct Pns {
    /// Chronaxie `τ_c` [s] — the nerve impulse-response time constant.
    pub chronaxie_s: f64,
    /// Rheobase `r` — stimulation threshold for an infinite-duration constant slew.
    pub rheobase: f64,
    /// Effective coil length `α` (GE lingo). The model uses `Smin = r / α`.
    pub alpha: f64,
}

/// A scanner hardware/safety profile. Field units are SI-adjacent and match the
/// limit's natural unit (mT/m, T/m/s, µT, seconds) so the numbers read like the
/// scanner spec; the checks convert the interpreted IR (Hz/m, Hz) into these.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Resolution key (the `--profile` name); unique across bundled profiles.
    pub name: String,
    /// Vendor tag (`ge`, `siemens`, `generic`, …); informational.
    pub vendor: String,
    /// One-line human description.
    pub description: String,
    /// Provenance: where every number traces to. Surfaced in the `hardware.profile`
    /// result so a report is self-documenting about which scanner it assumed.
    pub source: String,
    /// Static field strength [T]; informational (γ conversions use ¹H, not B0).
    pub b0_t: f64,
    /// Peak gradient amplitude per axis [mT/m].
    pub max_grad_mt_m: f64,
    /// Peak slew rate per axis [T/m/s].
    pub max_slew_t_m_s: f64,
    /// Peak B1 [µT]. `f64::INFINITY` means "unknown / no limit" → the B1 check
    /// `skip`s rather than passing vacuously.
    pub max_b1_ut: f64,
    /// Gradient raster [s].
    pub grad_raster_s: f64,
    /// RF raster [s].
    pub rf_raster_s: f64,
    /// ADC raster [s] — the hardware sampling grid the ADC dwell must divide.
    pub adc_raster_s: f64,
    /// Block-duration raster [s].
    pub block_raster_s: f64,
    /// Minimum RF dead time before a pulse may start within its block [s].
    pub rf_dead_s: f64,
    /// Minimum RF ring-down after a pulse before the block may end [s].
    pub rf_ringdown_s: f64,
    /// Minimum ADC dead time before sampling may start within its block [s].
    pub adc_dead_s: f64,
    /// PNS model parameters, when the profile models PNS.
    pub pns: Option<Pns>,
}

impl Profile {
    /// The bundled profile names, in `--profile` help order.
    pub fn bundled_names() -> &'static [&'static str] {
        &["ge-premier", "generic-3t"]
    }

    /// Look up a bundled profile by its [`name`](Profile::name). The spec
    /// stems `generic` / `default` are accepted as aliases for `generic-3t`, so a
    /// spec that names the profile either way loads unmodified.
    pub fn by_name(name: &str) -> Option<Profile> {
        match name {
            "ge-premier" => Some(ge_premier()),
            "generic-3t" | "generic" | "default" => Some(generic_3t()),
            _ => None,
        }
    }

    /// Build a profile from hardware limits embedded in the file's `[DEFINITIONS]`,
    /// or `None` when the file carries none. Standard Pulseq files do not embed
    /// amplitude limits (only rasters + FOV), so this is the second resolution
    /// rung, used by files that *do* carry `maxGrad` / `maxSlew` definitions; the
    /// raster grid is taken from the file's own raster definitions.
    ///
    /// Units follow `mr.opts`: `maxGrad` [mT/m], `maxSlew` [T/m/s], `maxB1` [µT],
    /// dead times [s]. `maxGrad` and `maxSlew` are required (without them no
    /// amplitude/slew check is possible); everything else defaults.
    pub fn from_definitions(seq: &Sequence) -> Option<Profile> {
        let num = |k: &str| {
            seq.definitions
                .get(k)
                .and_then(|v| v.trim().parse::<f64>().ok())
        };
        let max_grad_mt_m = num("maxGrad")?;
        let max_slew_t_m_s = num("maxSlew")?;
        let tr = seq.time_raster;
        Some(Profile {
            name: "file-definitions".into(),
            vendor: "unknown".into(),
            description: "hardware limits read from the file's [DEFINITIONS]".into(),
            source: "the .seq file's own [DEFINITIONS] (maxGrad / maxSlew, optional \
                     maxB1 + dead times, raster grid from the file)"
                .into(),
            b0_t: num("B0").unwrap_or(0.0),
            max_grad_mt_m,
            max_slew_t_m_s,
            max_b1_ut: num("maxB1").unwrap_or(f64::INFINITY),
            grad_raster_s: tr.grad,
            rf_raster_s: tr.rf,
            adc_raster_s: tr.adc,
            block_raster_s: tr.block,
            rf_dead_s: num("rfDeadTime").unwrap_or(0.0),
            rf_ringdown_s: num("rfRingdownTime").unwrap_or(0.0),
            adc_dead_s: num("adcDeadTime").unwrap_or(0.0),
            pns: None,
        })
    }

    /// Override a single limit field by name (the `--set field=value` path). Field
    /// names accept both the snake-case [`Profile`] field and its `mr.opts` alias
    /// (e.g. `max_grad_mt_m` or `maxGrad`). Returns `Err` for an unknown field so a
    /// typo fails loudly instead of silently doing nothing, and for a non-finite
    /// value (`nan` / `inf`, e.g. an accidental overflow): a non-finite limit would
    /// make every comparison vacuously pass, silently disabling a safety check.
    pub fn apply_override(&mut self, field: &str, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!(
                "--set {field}: {value} is not a finite number (a non-finite limit \
                 would silently disable the check)"
            ));
        }
        match field {
            "max_grad_mt_m" | "maxGrad" => self.max_grad_mt_m = value,
            "max_slew_t_m_s" | "maxSlew" => self.max_slew_t_m_s = value,
            "max_b1_ut" | "maxB1" => self.max_b1_ut = value,
            "b0_t" | "B0" => self.b0_t = value,
            "grad_raster_s" => self.grad_raster_s = value,
            "rf_raster_s" => self.rf_raster_s = value,
            "adc_raster_s" => self.adc_raster_s = value,
            "block_raster_s" => self.block_raster_s = value,
            "rf_dead_s" | "rfDeadTime" => self.rf_dead_s = value,
            "rf_ringdown_s" | "rfRingdownTime" => self.rf_ringdown_s = value,
            "adc_dead_s" | "adcDeadTime" => self.adc_dead_s = value,
            other => {
                return Err(format!(
                    "unknown override field {other:?}; known fields: max_grad_mt_m, \
                     max_slew_t_m_s, max_b1_ut, b0_t, {{grad,rf,adc,block}}_raster_s, \
                     rf_dead_s, rf_ringdown_s, adc_dead_s"
                ));
            }
        }
        Ok(())
    }
}

/// Resolve the profile for a run from the explicit `--profile` name (if any) and
/// the file, per the resolution order. `Ok(None)` means no profile was
/// selected and none was embedded — the caller runs the checks anyway and they
/// `skip`. An explicit but unknown name is an `Err` (never a silent fallback).
pub fn resolve(name: Option<&str>, seq: &Sequence) -> Result<Option<Profile>, String> {
    if let Some(n) = name {
        return Profile::by_name(n).map(Some).ok_or_else(|| {
            format!(
                "unknown scanner profile {n:?}; available: {}",
                Profile::bundled_names().join(", ")
            )
        });
    }
    Ok(Profile::from_definitions(seq))
}

// --- bundled profiles --------------------------------------------------------
//
// Every number below is cited. Editing a number is a curation act: update the
// `source` and the inline note together (a stale number is a false pass/fail).

/// GE SIGNA Premier (HRMW gradient coil).
fn ge_premier() -> Profile {
    Profile {
        name: "ge-premier".into(),
        vendor: "ge".into(),
        description: "GE SIGNA Premier (HRMW)".into(),
        source: "Pulseq documentation for the GE SIGNA Premier (HRMW) gradient coil (2026)".into(),
        b0_t: 3.0,
        max_grad_mt_m: 50.0,
        max_slew_t_m_s: 150.0,
        max_b1_ut: 20.0,
        grad_raster_s: 4e-6,
        rf_raster_s: 2e-6,
        adc_raster_s: 2e-6,
        block_raster_s: 4e-6,
        rf_dead_s: 100e-6,
        rf_ringdown_s: 60e-6,
        adc_dead_s: 0.0,
        pns: Some(Pns {
            chronaxie_s: 642.4e-6,
            rheobase: 17.9,
            alpha: 0.310,
        }),
    }
}

/// A vendor-neutral 3 T profile. The limits
/// and raster grid are the Pulseq toolbox `mr.opts()` defaults; only B0 is set to
/// 3 T. Simulation-oriented (no PNS model) and deliberately conservative — not a
/// substitute for a real scanner's safety limits.
fn generic_3t() -> Profile {
    Profile {
        name: "generic-3t".into(),
        vendor: "generic".into(),
        description: "Vendor-neutral 3 T (Pulseq mr.opts defaults); simulation-oriented".into(),
        source: "Pulseq toolbox mr.opts() no-argument defaults (pulseq matlab/+mr/opts.m): \
                 maxGrad 40 mT/m, maxSlew 170 T/m/s, maxB1 20 µT, rasters grad/block 10 µs, \
                 rf 1 µs, adc 0.1 µs, zero dead times. B0 set to 3 T. Vendor-neutral and \
                 conservative — no PNS model."
            .into(),
        b0_t: 3.0,
        max_grad_mt_m: 40.0,
        max_slew_t_m_s: 170.0,
        max_b1_ut: 20.0,
        grad_raster_s: 10e-6,
        rf_raster_s: 1e-6,
        adc_raster_s: 0.1e-6,
        block_raster_s: 10e-6,
        rf_dead_s: 0.0,
        rf_ringdown_s: 0.0,
        adc_dead_s: 0.0,
        pns: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn bundled_profiles_resolve_and_cite_sources() {
        for name in Profile::bundled_names() {
            let p = Profile::by_name(name).unwrap();
            assert_eq!(&p.name, name);
            assert!(!p.source.is_empty(), "{name} must cite a source");
            assert!(p.max_grad_mt_m > 0.0 && p.max_slew_t_m_s > 0.0);
        }
        assert!(Profile::by_name("no-such-scanner").is_none());
    }

    #[test]
    fn override_changes_one_field_and_rejects_typos() {
        let mut p = Profile::by_name("ge-premier").unwrap();
        p.apply_override("maxGrad", 33.0).unwrap();
        assert_eq!(p.max_grad_mt_m, 33.0);
        assert_eq!(p.max_slew_t_m_s, 150.0, "other fields untouched");
        assert!(p.apply_override("maxGrradd", 1.0).is_err());
    }

    #[test]
    fn override_rejects_non_finite_value() {
        // A non-finite limit (nan / inf, e.g. an accidental overflow) would make
        // `over(value, limit)` vacuously false and silently disable the check.
        let mut p = Profile::by_name("ge-premier").unwrap();
        assert!(p.apply_override("maxGrad", f64::NAN).is_err());
        assert!(p.apply_override("maxSlew", f64::INFINITY).is_err());
        assert_eq!(
            p.max_grad_mt_m, 50.0,
            "a rejected override leaves the field intact"
        );
    }
}
