# Step 5 — Trajectory gate + dual-witness geometry

**Goal:** Compute the k-space trajectory by gradient integration and verify
FOV/matrix generally — including where the Cartesian area-algebra can't.

**Depends on:** Steps 1–4.

## Why

A `.seq`'s real geometry lives in its gradients. Integrating them is
dimension-general: it follows permuted readouts (e.g. MPRAGE readout on Gz),
oblique/rotated blades and spokes, and non-Cartesian (radial/spiral) readouts
that area-algebra cannot. This is the second of the two geometry witnesses.

## Tasks — trajectory

- [x] Analytic trajectory: `k(t) = ∫ G dt` per axis from the IR, in 1/m, with
      each block's **rotation extension** applied (so PROPELLER/radial blades
      fan out instead of collapsing).
- [x] Per-axis routine (symmetric for x/y/z):
      - extent → presence test (extent above a shared jitter floor; the kz
        presence test **is** the 2D-vs-3D question — absent ⇒ z = 1).
      - if present → coverage = `(matrix − 1) / FOV` resolution invariant.
      - tighten to exact count + step **only** when the axis is a uniformly,
        fully-sampled grid (single modal step).
- [x] In-plane vs through-plane shape detected independently (stack-of-spirals =
      non-Cartesian in xy, Cartesian in z).
- [x] Acceleration handling: full extent but broken uniformity/fullness
      (GRAPPA/SENSE/CAIPI ACS) ⇒ fall back to coverage (undersampling-robust).
- [x] Oversampling: divide out declared per-axis factor before exact count/step
      (coverage is oversampling-invariant); a declared factor that disagrees
      with the trajectory fails the count (self-policing).
      *Coverage-invariance is in place and unit-tested; in file-only mode the
      factor is `1` (the trajectory reports the physical count/FOV, so a 2×
      readout reads as 480 mm / 384 — see the example below). Dividing out a
      **declared** factor and the self-policing count check ride with the spec in
      Step 7, which supplies the per-axis factor.*

## Tasks — dual-witness FOV/matrix

- [x] Param-algebra geometry (Step 4 sibling): measure FOV/matrix **only** when
      the single-readout-per-excitation Cartesian model holds (one ADC per
      excitation, flat readout gradient spanning the ADC window); otherwise
      `skip`.
- [x] Trajectory gate owns geometry whenever param-algebra `skip`s; reconcile
      the two witnesses when both apply (must agree within tolerance).

## Acceptance criteria

- Cartesian corpus: param-algebra and trajectory geometry agree.
- Non-Cartesian / permuted / accelerated fixtures: param-algebra `skip`s,
  trajectory gate still reports correct extent/coverage and 2D-vs-3D.
- Rotation-extension fixture (radial/PROPELLER) fans out correctly (not
  collapsed onto one strip).

## References

- pulsepal harness `kspace.py` (analytic trajectory) and `sim_traj.py`
  trajectory gate (our code) — the algorithms to port.

## Risks / notes

- Keep the jitter floor and "uniform grid" detection robust to numerical noise;
  these thresholds are the main source of false `fail`/`skip`.

## Status — complete

Implemented in `crates/seq-validate-core/src/trajectory.rs` (one `TrajectoryGeometry`
check emitting `metrics.{fov,matrix}` — the param-algebra witness — plus
`trajectory.{dimensionality,extent,fov,matrix,geometry_agreement}`), wired into
`checks::registry()`. Ported from the harness `kspace.py` (analytic trajectory)
and `sim_traj.py::measure_trajectory` (per-axis routine) and `param_check.py::measure`
(Cartesian area-algebra), in SI units. Measurements are `pass`/`skip`; the lone
`warn` is a dual-witness disagreement.

**Key simplification vs the harness.** We integrate the *interpreted* gradients,
which the parser already rotates per each block's `ROTATIONS` extension — so
PROPELLER blades and radial spokes fan out without re-applying any rotation matrix
here (the harness's `block_rotation` step is unnecessary). To make that hold for
*every* rotated sequence, the parser's `transform_grad` gained a general rotation
path (resample the axes onto a common breakpoint grid, mix through the 3×3
transform): `sos-liver.seq` (stack-of-stars), previously an `UnsupportedRotation`
error, now interprets cleanly (see `crates/pulseq-parse/NOTICE`).

**Acceptance — verified.**

- *Cartesian corpus agrees.* The 5 single-echo families recover the generated
  `matrix`/`fov_mm` exactly via the area-algebra and the trajectory witness agrees
  (`corpus_oracle::corpus_geometry_dual_witness`). The corpus generator now emits
  `matrix`/`fov_mm` (regenerated under MATLAB R2024a).
- *Non-Cartesian / permuted / accelerated → param-algebra `skip`s, trajectory
  reports.* EPI / mGRE (echo trains) defer; the trajectory still gives the correct
  phase-encode count and 2D-vs-3D. The accelerated (GRAPPA-pattern) → coverage
  fallback is unit-tested (`trajectory::tests::accelerated_axis_falls_back_to_coverage`).
- *Rotation fixture fans out.* PROPELLER sweeps both kx and ky (extent ≈ 580 1/m
  each — not collapsed); stack-of-stars is radial in-plane (kx/ky → coverage) and
  Cartesian through-plane (kz → exact 38-partition grid), the independent
  in-plane-vs-through-plane test (`tests/trajectory.rs`).

Measured on the v1.5.1 example (`t1_spgr_axial_brain.seq`): 2D, `matrix [384,192]`,
`fov [480,240] mm` — the readout is 2× oversampled, so the physical FOV/count is
double the nominal in x, and **both witnesses agree on it** (oversampling is divided
out only when the spec declares the factor, Step 7).

Tests: `trajectory.rs` inline unit tests (7), `tests/trajectory.rs` fixture pins (4),
`corpus_oracle::corpus_geometry_dual_witness` (8 sequences). Full workspace suite +
`clippy -D warnings` + `rustfmt` green.
