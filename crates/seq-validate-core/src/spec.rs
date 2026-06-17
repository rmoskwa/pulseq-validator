//! The optional expected-value spec assert mode.
//!
//! The file-only checks answer "what is this sequence?"; a spec answers "did I
//! build what I intended?" — the sharp pass/fail for CI and for AI tools that
//! emit a target spec alongside a build. A [`Spec`] is the YAML format
//! (`te_ms` / `tr_ms` / `flip_angle_deg` / `n_slices` / `echo_spacing_ms` /
//! `fov_mm[xyz]` / `matrix[xyz]` / `oversampling` / `scanner`); each field the
//! user provides becomes a `spec.*` [`CheckResult`] whose `measured` is reused
//! from the measured metrics/trajectory/hardware results and whose status is the tolerance comparison.
//!
//! Two deliberate policy choices keep the assertion lenient:
//!
//! - **Lenient**: only the fields the user provides are checked; an
//!   absent field is silently not-asserted. A "required-or-`none`"
//!   policy is deliberately relaxed — but the literal `none` (and YAML null)
//!   still parse as an explicit opt-out, so such specs load unmodified.
//! - **A spec field that is provided but cannot be measured** (e.g. echo spacing
//!   on a single-echo sequence, or a geometry axis no witness pins) is a `skip`,
//!   not a `fail`: a first-class non-failing result. The exit code is nonzero
//!   **iff** an asserted field is measured and out of tolerance.
//!
//! Geometry honors the dual-witness: an axis is asserted against the
//! param-algebra (`metrics.matrix` / `metrics.fov`) when it applies (Cartesian),
//! else the trajectory gate (`trajectory.matrix` / `trajectory.fov`). The
//! declared per-axis `oversampling` is divided out of the *physical* measured
//! count/FOV before comparing to the spec's nominal value.
//!
//! `scanner` selects the scanner [`crate::profile::Profile`]; it is an input, not
//! an asserted field, so it produces no `spec.*` result (the CLI resolves it).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use serde_yaml::{Mapping, Value as Yaml};

use crate::measurements::Measurements;
use crate::result::CheckResult;

/// A per-field tolerance. `Abs` is an absolute band in the field's own unit,
/// `Rel` a fraction of the expected magnitude, `Exact` strict equality (matrix
/// counts and the slice count). Defaults are seeded from the reference
/// implementation; a spec may override any field (see [`Spec::tolerances`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tolerance {
    /// `|measured − expected| ≤ x`, in the field's unit.
    Abs(f64),
    /// `|measured − expected| ≤ f · |expected|`.
    Rel(f64),
    /// `measured == expected`.
    Exact,
}

impl Tolerance {
    /// Whether `measured` satisfies the tolerance against `expected`.
    fn passes(self, measured: f64, expected: f64) -> bool {
        match self {
            Tolerance::Abs(a) => (measured - expected).abs() <= a,
            Tolerance::Rel(f) => {
                if expected == 0.0 {
                    measured == 0.0
                } else {
                    (measured - expected).abs() <= f * expected.abs()
                }
            }
            Tolerance::Exact => measured == expected,
        }
    }

    /// Human description of the band, for the result message.
    fn describe(self) -> String {
        match self {
            Tolerance::Abs(a) => format!("abs {a}"),
            Tolerance::Rel(f) => format!("rel {:.0}%", f * 100.0),
            Tolerance::Exact => "exact".to_string(),
        }
    }
}

/// The default tolerance for a `spec.*` field key, seeded from the reference
/// implementation's tolerance table.
fn default_tolerance(field: &str) -> Tolerance {
    match field {
        "te_ms" | "tr_ms" | "echo_spacing_ms" => Tolerance::Abs(0.1),
        "flip_angle_deg" => Tolerance::Rel(0.05),
        "fov_mm_x" | "fov_mm_y" => Tolerance::Rel(0.03),
        "fov_mm_z" => Tolerance::Rel(0.10),
        // matrix_{x,y,z} and n_slices are integer counts.
        _ => Tolerance::Exact,
    }
}

/// A parsed expected-value spec.
///
/// Build with [`from_yaml_file`](Spec::from_yaml_file) / [`from_yaml_str`](Spec::from_yaml_str)
/// (a malformed spec is a `String` error → the CLI turns it into an exit-2
/// error), then run [`assert`](Spec::assert) against the file-only check results.
/// Every numeric field is `Option`: `None` means absent / opted-out (the literal
/// `none` or YAML null) and is simply not asserted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    /// Effective (k-centre) TE [ms].
    pub te_ms: Option<f64>,
    /// Repetition time [ms].
    pub tr_ms: Option<f64>,
    /// Excitation flip angle [deg].
    pub flip_angle_deg: Option<f64>,
    /// Slice/slab count (distinct excitation frequency offsets).
    pub n_slices: Option<i64>,
    /// Echo spacing [ms] (echo trains only).
    pub echo_spacing_ms: Option<f64>,
    /// `[x, y, z]` field of view [mm]; a per-axis `None` is not asserted.
    pub fov_mm: [Option<f64>; 3],
    /// `[x, y, z]` matrix size; a per-axis `None` is not asserted.
    pub matrix: [Option<i64>; 3],
    /// `[x, y, z]` per-axis oversampling factor (divided out of the measured
    /// geometry before comparison). Absent / `none` → `[1, 1, 1]`.
    pub oversampling: [f64; 3],
    /// Scanner profile stem (selects the scanner profile); an input, not asserted.
    pub scanner: Option<String>,
    /// Per-field tolerance overrides, keyed by `spec.*` field name (e.g.
    /// `te_ms`, `fov_mm_x`, `matrix_y`); fields absent here use [`default_tolerance`].
    pub tolerances: BTreeMap<String, Tolerance>,
}

impl Spec {
    /// Parse a spec from a YAML file path.
    pub fn from_yaml_file(path: &Path) -> Result<Spec, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read spec {}: {e}", path.display()))?;
        Spec::from_yaml_str(&text)
    }

    /// Parse a spec from a YAML string. Unknown keys (free-form
    /// authoring guidance) are ignored; a wrong-typed known key is an error.
    pub fn from_yaml_str(text: &str) -> Result<Spec, String> {
        let yaml: Yaml = serde_yaml::from_str(text).map_err(|e| format!("invalid YAML: {e}"))?;
        let map = match &yaml {
            Yaml::Mapping(m) => m,
            // An empty document is a spec that asserts nothing.
            Yaml::Null => return Ok(Spec::default()),
            _ => return Err("spec must be a YAML mapping of fields".to_string()),
        };

        Ok(Spec {
            te_ms: scalar(map, "te_ms")?,
            tr_ms: scalar(map, "tr_ms")?,
            flip_angle_deg: scalar(map, "flip_angle_deg")?,
            n_slices: scalar_int(map, "n_slices")?,
            echo_spacing_ms: scalar(map, "echo_spacing_ms")?,
            fov_mm: vec3(map, "fov_mm", elem_f64)?,
            matrix: vec3(map, "matrix", elem_i64)?,
            oversampling: oversampling(map)?,
            scanner: scanner(map)?,
            tolerances: tolerances(map)?,
        })
    }

    /// Tolerance for a `spec.*` field key (override if the spec set one, else the
    /// built-in default).
    fn tolerance_for(&self, field: &str) -> Tolerance {
        self.tolerances
            .get(field)
            .copied()
            .unwrap_or_else(|| default_tolerance(field))
    }

    /// Assert every provided field against the typed [`Measurements`], emitting
    /// one `spec.*` [`CheckResult`] per provided field. `measured` is reused from
    /// the measured metrics/trajectory/hardware results (no re-measurement); a field whose source metric was
    /// not measured `skip`s.
    pub fn assert(&self, m: &Measurements) -> Vec<CheckResult> {
        let mut out = Vec::new();

        // Scalar metrics: source id (for the message), unit scale (IR seconds →
        // spec ms), expected.
        if let Some(e) = self.te_ms {
            out.push(self.scalar_result("te_ms", "metrics.te", m.te_s, 1e3, e));
        }
        if let Some(e) = self.tr_ms {
            out.push(self.scalar_result("tr_ms", "metrics.tr", m.tr_s, 1e3, e));
        }
        if let Some(e) = self.flip_angle_deg {
            out.push(self.scalar_result(
                "flip_angle_deg",
                "metrics.flip_angle",
                m.flip_deg,
                1.0,
                e,
            ));
        }
        if let Some(e) = self.echo_spacing_ms {
            out.push(self.scalar_result(
                "echo_spacing_ms",
                "metrics.echo_spacing",
                m.echo_spacing_s,
                1e3,
                e,
            ));
        }
        if let Some(e) = self.n_slices {
            out.push(self.count_result("n_slices", "metrics.n_slices", m.n_slices, e));
        }

        // Geometry: pick the authoritative witness once per quantity, then assert
        // each provided axis against it (oversampling divided out).
        let (matrix_w, matrix_label) = m.matrix.authoritative();
        for (axis, key) in [(0, "matrix_x"), (1, "matrix_y"), (2, "matrix_z")] {
            if let Some(e) = self.matrix.get(axis).copied().flatten() {
                out.push(self.matrix_result(key, axis, e, matrix_w, matrix_label));
            }
        }
        let (fov_w, fov_label) = m.fov.authoritative();
        for (axis, key) in [(0, "fov_mm_x"), (1, "fov_mm_y"), (2, "fov_mm_z")] {
            if let Some(e) = self.fov_mm.get(axis).copied().flatten() {
                out.push(self.fov_result(key, axis, e, fov_w, fov_label));
            }
        }
        out
    }

    /// Assert a scalar metric (in IR units `scale`d into the spec's unit).
    fn scalar_result(
        &self,
        field: &str,
        metric_id: &str,
        measured: Option<f64>,
        scale: f64,
        expected: f64,
    ) -> CheckResult {
        let id = format!("spec.{field}");
        let tol = self.tolerance_for(field);
        match measured {
            None => not_measurable(&id, field, metric_id, Value::from(expected)),
            Some(raw) => compare(&id, field, raw * scale, expected, tol, ""),
        }
    }

    /// Assert an integer count metric (`n_slices`).
    fn count_result(
        &self,
        field: &str,
        metric_id: &str,
        measured: Option<f64>,
        expected: i64,
    ) -> CheckResult {
        let id = format!("spec.{field}");
        let tol = self.tolerance_for(field);
        match measured {
            None => not_measurable(&id, field, metric_id, Value::from(expected)),
            #[allow(clippy::cast_possible_truncation)] // counts are small, exact in f64
            Some(raw) => compare_count(&id, field, raw.round() as i64, expected, tol, ""),
        }
    }

    /// Assert one matrix axis against the chosen witness, dividing out oversampling.
    fn matrix_result(
        &self,
        field: &str,
        axis: usize,
        expected: i64,
        witness: Option<&[Option<f64>]>,
        witness_label: &str,
    ) -> CheckResult {
        let id = format!("spec.{field}");
        let tol = self.tolerance_for(field);
        let os = self.oversampling.get(axis).copied().unwrap_or(1.0);
        match witness.and_then(|w| w.get(axis).copied().flatten()) {
            None => geometry_not_measurable(&id, field, Value::from(expected)),
            #[allow(clippy::cast_possible_truncation)] // counts are small, exact in f64
            Some(physical) => {
                let measured = (physical / os).round() as i64;
                let extra = oversampling_note(os, witness_label, &format!("{physical}"));
                compare_count(&id, field, measured, expected, tol, &extra)
            }
        }
    }

    /// Assert one FOV axis against the chosen witness, dividing out oversampling.
    fn fov_result(
        &self,
        field: &str,
        axis: usize,
        expected: f64,
        witness: Option<&[Option<f64>]>,
        witness_label: &str,
    ) -> CheckResult {
        let id = format!("spec.{field}");
        let tol = self.tolerance_for(field);
        let os = self.oversampling.get(axis).copied().unwrap_or(1.0);
        match witness.and_then(|w| w.get(axis).copied().flatten()) {
            None => geometry_not_measurable(&id, field, Value::from(expected)),
            Some(physical) => {
                let measured = physical / os;
                let extra = oversampling_note(os, witness_label, &format!("{physical:.1} mm"));
                compare(&id, field, measured, expected, tol, &extra)
            }
        }
    }
}

// --- result builders ---------------------------------------------------------

/// A passing/failing comparison of a float measurement against the spec.
fn compare(
    id: &str,
    field: &str,
    measured: f64,
    expected: f64,
    tol: Tolerance,
    extra: &str,
) -> CheckResult {
    let ok = tol.passes(measured, expected);
    let dev = (measured - expected).abs();
    let band = tol.describe();
    let msg = if ok {
        format!(
            "{field}: measured {measured:.3} matches expected {expected:.3} (within {band}){extra}"
        )
    } else {
        format!(
            "{field}: measured {measured:.3} vs expected {expected:.3} — |Δ| {dev:.3} exceeds {band}{extra}"
        )
    };
    let base = if ok {
        CheckResult::pass(id, msg)
    } else {
        CheckResult::fail(id, msg)
    };
    base.with_measured(measured).with_expected(expected)
}

/// A passing/failing comparison of an integer-count measurement.
fn compare_count(
    id: &str,
    field: &str,
    measured: i64,
    expected: i64,
    tol: Tolerance,
    extra: &str,
) -> CheckResult {
    let ok = tol.passes(measured as f64, expected as f64);
    let msg = if ok {
        format!(
            "{field}: measured {measured} matches expected {expected} ({}){extra}",
            tol.describe()
        )
    } else {
        format!(
            "{field}: measured {measured} vs expected {expected} ({}){extra}",
            tol.describe()
        )
    };
    let base = if ok {
        CheckResult::pass(id, msg)
    } else {
        CheckResult::fail(id, msg)
    };
    base.with_measured(measured).with_expected(expected)
}

/// A `skip` for a provided field whose source metric was not measured (the
/// sequence does not support it) — never a failure.
fn not_measurable(id: &str, field: &str, metric_id: &str, expected: Value) -> CheckResult {
    CheckResult::skip(
        id,
        format!("{field}: expected value given, but {metric_id} measured nothing for this sequence; not asserted"),
    )
    .with_expected(expected)
}

/// A `skip` for a geometry axis no witness could pin (non-Cartesian / accelerated
/// axis, or no clean grid) — never a failure.
fn geometry_not_measurable(id: &str, field: &str, expected: Value) -> CheckResult {
    CheckResult::skip(
        id,
        format!("{field}: expected value given, but neither geometry witness measured this axis; not asserted"),
    )
    .with_expected(expected)
}

/// Trailing message note recording the witness and (when ≠ 1) the oversampling
/// division applied to reach the comparable value.
fn oversampling_note(os: f64, witness_label: &str, physical_str: &str) -> String {
    if (os - 1.0).abs() > 1e-9 {
        format!(" [{witness_label}: physical {physical_str} ÷ oversampling {os}]")
    } else {
        format!(" [{witness_label}]")
    }
}

// --- YAML field extraction (lenient: number | `none` | null | absent) --------

/// Whether a YAML value is the literal `none` opt-out string (case-insensitive).
fn is_none_str(v: &Yaml) -> bool {
    v.as_str()
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("none"))
}

/// The YAML type name, for an error message.
fn type_name(v: &Yaml) -> &'static str {
    match v {
        Yaml::Null => "null",
        Yaml::Bool(_) => "a boolean",
        Yaml::Number(_) => "a number",
        Yaml::String(_) => "a string",
        Yaml::Sequence(_) => "a sequence",
        Yaml::Mapping(_) => "a mapping",
        Yaml::Tagged(_) => "a tagged value",
    }
}

/// A scalar float field: `Some(v)` for a number, `None` for absent / null /
/// `none`, `Err` for any other type.
fn scalar(map: &Mapping, key: &str) -> Result<Option<f64>, String> {
    match map.get(key) {
        None | Some(Yaml::Null) => Ok(None),
        Some(v) if is_none_str(v) => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| {
            format!(
                "spec field `{key}`: expected a number or `none`, got {}",
                type_name(v)
            )
        }),
    }
}

/// A scalar integer field (lenient about absent / null / `none` like [`scalar`]).
fn scalar_int(map: &Mapping, key: &str) -> Result<Option<i64>, String> {
    match map.get(key) {
        None | Some(Yaml::Null) => Ok(None),
        Some(v) if is_none_str(v) => Ok(None),
        Some(v) => v.as_i64().map(Some).ok_or_else(|| {
            format!(
                "spec field `{key}`: expected an integer or `none`, got {}",
                type_name(v)
            )
        }),
    }
}

/// A single `fov_mm` element: a number, or `None` for null / `none`.
fn elem_f64(v: &Yaml, key: &str, i: usize) -> Result<Option<f64>, String> {
    if matches!(v, Yaml::Null) || is_none_str(v) {
        return Ok(None);
    }
    v.as_f64().map(Some).ok_or_else(|| {
        format!(
            "spec field `{key}`[{i}]: expected a number or `none`, got {}",
            type_name(v)
        )
    })
}

/// A single `matrix` element: an integer, or `None` for null / `none`.
fn elem_i64(v: &Yaml, key: &str, i: usize) -> Result<Option<i64>, String> {
    if matches!(v, Yaml::Null) || is_none_str(v) {
        return Ok(None);
    }
    v.as_i64().map(Some).ok_or_else(|| {
        format!(
            "spec field `{key}`[{i}]: expected an integer or `none`, got {}",
            type_name(v)
        )
    })
}

/// A 3-vector field (`fov_mm` / `matrix`): each axis parsed by `elem`. Absent /
/// null / `none` (the whole field) → all axes unset. A sequence shorter than 3 is
/// padded with `None`; longer than 3 is an error (a typo, not a silent truncation).
fn vec3<T>(
    map: &Mapping,
    key: &str,
    elem: fn(&Yaml, &str, usize) -> Result<Option<T>, String>,
) -> Result<[Option<T>; 3], String>
where
    T: Copy,
{
    let mut out = [None, None, None];
    match map.get(key) {
        None | Some(Yaml::Null) => Ok(out),
        Some(v) if is_none_str(v) => Ok(out),
        Some(Yaml::Sequence(seq)) => {
            if seq.len() > 3 {
                return Err(format!(
                    "spec field `{key}`: expected up to 3 values [x, y, z], got {}",
                    seq.len()
                ));
            }
            for (i, e) in seq.iter().enumerate() {
                if let Some(slot) = out.get_mut(i) {
                    *slot = elem(e, key, i)?;
                }
            }
            Ok(out)
        }
        Some(v) => Err(format!(
            "spec field `{key}`: expected a [x, y, z] sequence or `none`, got {}",
            type_name(v)
        )),
    }
}

/// The per-axis oversampling factor. Absent / null / `none` → `[1, 1, 1]`; each
/// provided axis must be a positive number (a non-positive factor would make the
/// geometry division meaningless).
fn oversampling(map: &Mapping) -> Result<[f64; 3], String> {
    let mut out = [1.0, 1.0, 1.0];
    match map.get("oversampling") {
        None | Some(Yaml::Null) => Ok(out),
        Some(v) if is_none_str(v) => Ok(out),
        Some(Yaml::Sequence(seq)) => {
            if seq.len() > 3 {
                return Err(format!(
                    "spec field `oversampling`: expected up to 3 values, got {}",
                    seq.len()
                ));
            }
            for (i, e) in seq.iter().enumerate() {
                let f = e.as_f64().ok_or_else(|| {
                    format!(
                        "spec field `oversampling`[{i}]: expected a number, got {}",
                        type_name(e)
                    )
                })?;
                if !f.is_finite() || f <= 0.0 {
                    return Err(format!(
                        "spec field `oversampling`[{i}]: must be a positive factor, got {f}"
                    ));
                }
                if let Some(slot) = out.get_mut(i) {
                    *slot = f;
                }
            }
            Ok(out)
        }
        Some(v) => Err(format!(
            "spec field `oversampling`: expected a sequence or `none`, got {}",
            type_name(v)
        )),
    }
}

/// The scanner profile stem, or `None` for absent / null / `none`.
fn scanner(map: &Mapping) -> Result<Option<String>, String> {
    match map.get("scanner") {
        None | Some(Yaml::Null) => Ok(None),
        Some(v) if is_none_str(v) => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.trim().to_string()))
            .ok_or_else(|| {
                format!(
                    "spec field `scanner`: expected a profile name, got {}",
                    type_name(v)
                )
            }),
    }
}

/// Per-field tolerance overrides under an optional `tolerances:` mapping. Each
/// value is the string `exact`, or a `{ abs: x }` / `{ rel: f }` mapping.
fn tolerances(map: &Mapping) -> Result<BTreeMap<String, Tolerance>, String> {
    let mut out = BTreeMap::new();
    match map.get("tolerances") {
        None | Some(Yaml::Null) => Ok(out),
        Some(Yaml::Mapping(tm)) => {
            for (k, v) in tm {
                let field = k
                    .as_str()
                    .ok_or_else(|| "tolerances: keys must be field names".to_string())?;
                out.insert(field.to_string(), parse_tolerance(field, v)?);
            }
            Ok(out)
        }
        Some(v) => Err(format!(
            "spec field `tolerances`: expected a mapping, got {}",
            type_name(v)
        )),
    }
}

/// Parse one tolerance override: `exact` (string) or `{abs|rel: <number>}`.
fn parse_tolerance(field: &str, v: &Yaml) -> Result<Tolerance, String> {
    if v.as_str()
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("exact"))
    {
        return Ok(Tolerance::Exact);
    }
    if let Yaml::Mapping(m) = v {
        if let Some(a) = m.get("abs").and_then(Yaml::as_f64) {
            return finite_nonneg(field, "abs", a).map(Tolerance::Abs);
        }
        if let Some(r) = m.get("rel").and_then(Yaml::as_f64) {
            return finite_nonneg(field, "rel", r).map(Tolerance::Rel);
        }
    }
    Err(format!(
        "tolerances.{field}: expected `exact`, `{{abs: <n>}}`, or `{{rel: <n>}}`"
    ))
}

/// Validate a tolerance band magnitude: a negative or non-finite band would
/// silently reject every value, so it is an error rather than a degenerate pass.
fn finite_nonneg(field: &str, kind: &str, x: f64) -> Result<f64, String> {
    if x.is_finite() && x >= 0.0 {
        Ok(x)
    } else {
        Err(format!(
            "tolerances.{field}: `{kind}` must be a finite, non-negative number, got {x}"
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]
    use super::*;

    #[test]
    fn none_and_absent_fields_are_not_asserted() {
        // `none`, YAML null, and absence all collapse to "not asserted".
        let s = Spec::from_yaml_str("te_ms: none\ntr_ms: ~\nflip_angle_deg: 15\n").unwrap();
        assert_eq!(s.te_ms, None);
        assert_eq!(s.tr_ms, None);
        assert_eq!(s.flip_angle_deg, Some(15.0));
        assert_eq!(s.n_slices, None);
    }

    #[test]
    fn unknown_keys_and_nested_blocks_are_ignored() {
        // Real-world specs often carry free-form `acquisition:` / `notes:` blocks and a
        // `name:`; none of them are gated, so the spec must load and ignore them.
        let yaml = "\
name: propeller
family: fse-propeller
te_ms: 84
acquisition:\n  readout: fse\n  etl: 16\nnotes: >\n  free text\n";
        let s = Spec::from_yaml_str(yaml).unwrap();
        assert_eq!(s.te_ms, Some(84.0));
    }

    #[test]
    fn geometry_vectors_and_oversampling_parse() {
        let s = Spec::from_yaml_str(
            "matrix: [192, 192, 1]\nfov_mm: [240, 240, 5]\noversampling: [2, 1, 1]\n",
        )
        .unwrap();
        assert_eq!(s.matrix, [Some(192), Some(192), Some(1)]);
        assert_eq!(s.fov_mm, [Some(240.0), Some(240.0), Some(5.0)]);
        assert_eq!(s.oversampling, [2.0, 1.0, 1.0]);
    }

    #[test]
    fn none_geometry_vector_disables_all_axes() {
        let s = Spec::from_yaml_str("matrix: none\nfov_mm: none\noversampling: none\n").unwrap();
        assert_eq!(s.matrix, [None, None, None]);
        assert_eq!(s.fov_mm, [None, None, None]);
        assert_eq!(s.oversampling, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn wrong_typed_field_is_an_error() {
        assert!(Spec::from_yaml_str("te_ms: hello\n").is_err());
        assert!(Spec::from_yaml_str("oversampling: [0, 1, 1]\n").is_err()); // non-positive
        assert!(Spec::from_yaml_str("matrix: [1, 2, 3, 4]\n").is_err()); // too long
        // A negative tolerance band would reject every value → an error, not a pass.
        assert!(Spec::from_yaml_str("tolerances:\n  te_ms: {abs: -1.0}\n").is_err());
    }

    #[test]
    fn tolerance_override_parses_and_applies() {
        let s =
            Spec::from_yaml_str("te_ms: 10\ntolerances:\n  te_ms: {abs: 1.0}\n  matrix_x: exact\n")
                .unwrap();
        assert_eq!(s.tolerance_for("te_ms"), Tolerance::Abs(1.0));
        assert_eq!(s.tolerance_for("matrix_x"), Tolerance::Exact);
        // An un-overridden field keeps its default.
        assert_eq!(s.tolerance_for("flip_angle_deg"), Tolerance::Rel(0.05));
    }

    #[test]
    fn tolerance_semantics() {
        assert!(Tolerance::Abs(0.1).passes(10.05, 10.0));
        assert!(!Tolerance::Abs(0.1).passes(10.2, 10.0));
        assert!(Tolerance::Rel(0.05).passes(81.0, 80.0));
        assert!(!Tolerance::Rel(0.05).passes(90.0, 80.0));
        assert!(Tolerance::Exact.passes(192.0, 192.0));
        assert!(!Tolerance::Exact.passes(191.0, 192.0));
    }
}
