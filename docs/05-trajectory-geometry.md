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

- [ ] Analytic trajectory: `k(t) = ∫ G dt` per axis from the IR, in 1/m, with
      each block's **rotation extension** applied (so PROPELLER/radial blades
      fan out instead of collapsing).
- [ ] Per-axis routine (symmetric for x/y/z):
      - extent → presence test (extent above a shared jitter floor; the kz
        presence test **is** the 2D-vs-3D question — absent ⇒ z = 1).
      - if present → coverage = `(matrix − 1) / FOV` resolution invariant.
      - tighten to exact count + step **only** when the axis is a uniformly,
        fully-sampled grid (single modal step).
- [ ] In-plane vs through-plane shape detected independently (stack-of-spirals =
      non-Cartesian in xy, Cartesian in z).
- [ ] Acceleration handling: full extent but broken uniformity/fullness
      (GRAPPA/SENSE/CAIPI ACS) ⇒ fall back to coverage (undersampling-robust).
- [ ] Oversampling: divide out declared per-axis factor before exact count/step
      (coverage is oversampling-invariant); a declared factor that disagrees
      with the trajectory fails the count (self-policing).

## Tasks — dual-witness FOV/matrix

- [ ] Param-algebra geometry (Step 4 sibling): measure FOV/matrix **only** when
      the single-readout-per-excitation Cartesian model holds (one ADC per
      excitation, flat readout gradient spanning the ADC window); otherwise
      `skip`.
- [ ] Trajectory gate owns geometry whenever param-algebra `skip`s; reconcile
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
