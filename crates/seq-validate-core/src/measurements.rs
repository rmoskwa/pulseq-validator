//! The typed home for the measured numbers.
//!
//! The file-only checks emit their headline numbers as untyped `measured` JSON
//! on the [`CheckResult`] vector (`metrics.te`, `trajectory.matrix`, …). Reading
//! one back means knowing its string id, its unit, and that `as_f64` may fail —
//! a wide interface for one number, re-implemented by every consumer.
//!
//! [`Measurements`] is that surface, typed once: [`from_results`](Measurements::from_results)
//! does the id lookup, the unit is the field's type, and the dual-witness
//! geometry routing lives in [`Geometry::authoritative`]. Consumers (the spec
//! assertions, the tests) read fields instead of re-parsing the vector. The JSON
//! contract is unchanged — `Measurements` is derived *from* the same results the
//! report serializes.

use serde_json::Value;

use crate::result::{CheckResult, Status};

/// A geometry quantity (matrix or FOV) and its two witnesses.
///
/// The param-algebra witness (`metrics.matrix` / `metrics.fov`) recovers the
/// nominal grid when the single-readout Cartesian model holds; the trajectory
/// gate (`trajectory.matrix` / `trajectory.fov`) recovers coverage from the
/// k-space path when it does not. Both arrays are `[x, y, z]` with a per-axis
/// `None` for an axis no witness pins. The raw witnesses are kept distinct so a
/// consumer that cares which one spoke (the dual-witness tests) can read it;
/// most consumers want [`authoritative`](Geometry::authoritative).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    /// The param-algebra witness's measured `[x, y, z]`, raw (any status).
    pub param: Option<Vec<Option<f64>>>,
    /// The status of the param-algebra check; the model only *holds* on `Pass`.
    pub param_status: Option<Status>,
    /// The trajectory-gate witness's measured `[x, y, z]`, raw.
    pub trajectory: Option<Vec<Option<f64>>>,
}

impl Geometry {
    /// The authoritative witness: the param-algebra when its check passed (the
    /// Cartesian model held), else the trajectory gate. Returns the per-axis
    /// values and a label for the message.
    pub fn authoritative(&self) -> (Option<&[Option<f64>]>, &'static str) {
        if self.param_status == Some(Status::Pass) {
            (self.param.as_deref(), "param-algebra")
        } else {
            (self.trajectory.as_deref(), "trajectory")
        }
    }
}

/// Every measured number a consumer reads, typed and resolved once.
///
/// Scalars carry their unit in the field name (`_s` seconds, `_deg` degrees);
/// `None` means the metric was not measurable for this sequence (a `skip`), so
/// consumers branch on `Option` instead of re-deriving status from absence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measurements {
    /// Effective (k-centre) echo time [s].
    pub te_s: Option<f64>,
    /// Repetition time [s].
    pub tr_s: Option<f64>,
    /// Excitation flip angle [deg].
    pub flip_deg: Option<f64>,
    /// Echo spacing [s] (echo trains only).
    pub echo_spacing_s: Option<f64>,
    /// Total scan time [s].
    pub scan_time_s: Option<f64>,
    /// Slice/slab count.
    pub n_slices: Option<f64>,
    /// K-space dimensionality (2 or 3).
    pub dimensionality: Option<f64>,
    /// K-space extent `[kx, ky, kz]`.
    pub extent: Option<Vec<Option<f64>>>,
    /// Matrix size, dual-witness.
    pub matrix: Geometry,
    /// Field of view [mm], dual-witness.
    pub fov: Geometry,
}

impl Measurements {
    /// Read the typed measurements out of the file-only check results. The one
    /// place that knows the metric ids, that `measured` is `as_f64`/`as_array`,
    /// and that a `null` axis is `None`.
    pub fn from_results(results: &[CheckResult]) -> Self {
        let scalar = |id: &str| {
            find(results, id)
                .and_then(|r| r.measured.as_ref())
                .and_then(Value::as_f64)
        };
        let array = |id: &str| {
            find(results, id)
                .and_then(|r| r.measured.as_ref())
                .and_then(Value::as_array)
                .map(|a| a.iter().map(Value::as_f64).collect())
        };
        let geometry = |param_id: &str, traj_id: &str| Geometry {
            param: array(param_id),
            param_status: find(results, param_id).map(|r| r.status),
            trajectory: array(traj_id),
        };
        Measurements {
            te_s: scalar("metrics.te"),
            tr_s: scalar("metrics.tr"),
            flip_deg: scalar("metrics.flip_angle"),
            echo_spacing_s: scalar("metrics.echo_spacing"),
            scan_time_s: scalar("metrics.scan_time"),
            n_slices: scalar("metrics.n_slices"),
            dimensionality: scalar("trajectory.dimensionality"),
            extent: array("trajectory.extent"),
            matrix: geometry("metrics.matrix", "trajectory.matrix"),
            fov: geometry("metrics.fov", "trajectory.fov"),
        }
    }
}

/// Find a result by id.
fn find<'a>(results: &'a [CheckResult], id: &str) -> Option<&'a CheckResult> {
    results.iter().find(|r| r.id == id)
}
