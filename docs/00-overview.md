# Design Overview

The shared understanding the build steps implement. Each decision below is
settled; the per-step docs (`01`–`07`) are the actionable plan.

## What it is

A **clean-room, Rust-first** `.seq` validator. Greenfield in this repo. The
pulsepal harness and mr-zero are **executable references, not code to copy**.

- **mr-zero / MRtwin**: AGPL-3.0 **and** a non-commercial EULA. Off-limits to
  derive from — including the future v2 simulator, which must be re-derived
  from physics literature, not their source.
- **Pulseq core / pulseq-rs**: MIT. Safe to fork with attribution — we forked
  `pulseq-rs` into `crates/pulseq-parse` (owned/developed here, NOTICE retained).
- **pulsepal harness**: our own code; port its logic freely.

Released under a **permissive license (MIT or Apache-2.0)**.

## Scope — v1 is static/analytic only

No Bloch simulation in v1. Four check categories ship:

1. **Derived imaging metrics** — TE (k-centre effective TE), TR, flip angle,
   n_slices, echo spacing, FOV, matrix, in-plane resolution, scan time, and a
   **heuristic contrast/weighting label** (inferred from TE/TR/FA/prep — not
   measured).
2. **K-space trajectory analysis** — `k = ∫G·dt`; extent/coverage, sampling
   uniformity, 2D-vs-3D detection. Dimension-general: follows permuted /
   non-Cartesian readouts and applies block rotation extensions.
3. **Hardware/safety limits** — gradient amplitude, slew, ADC dwell vs raster,
   B1/duration, basic PNS — against **bundled scanner profiles**.
4. **Sequence integrity** — raster alignment, block/timing consistency,
   overlaps, version/signature sanity.

**Dual-witness geometry** (ported from the harness): the param-algebra measures
FOV/matrix only when the Cartesian model holds and **skips** otherwise; the
trajectory gate verifies geometry generally. A `skip` is a first-class result,
not a failure.

## Architecture

- **Parser / IR**: fork `pulseq-rs` into the repo as `crates/pulseq-parse`,
  owned and developed here. It already supports Pulseq **v1.5.1** (the version of
  the example file). Use its `raw` (faithful event tables) and `interp`
  (interpreted: absolute timing + rotations + decompressed shapes) layers. Checks
  target the interpreted layer.
- **Monolith core now**, but every check is a **discrete unit** so the plugin
  boundary can be extracted later, once 2–3 real specialized pipelines
  (diffusion, elastography, non-Cartesian) reveal the right seams.
- **Interface**: a `seq-validate file.seq` CLI on a reusable **library crate**,
  emitting a human report **and stable JSON**. JSON is the integration contract
  — Python/web consumers need no bindings in v1.
- **Two modes, spec optional**:
  - *Default*: file-only — report metrics + flag violations.
  - *Optional*: an **expected-spec** for hard pass/fail. Reuses the harness
    schema (`te_ms / tr_ms / flip_angle_deg / n_slices / echo_spacing_ms /
    fov_mm[xyz] / matrix[xyz] / oversampling / scanner`) with `abs/rel/exact`
    tolerances and **lenient defaults** (check only the fields provided).
- **Result model**: each check →
  `{ id, status: pass | fail | warn | skip, measured, expected?, severity, message }`.
  CLI exits nonzero only on `fail`.

## Trust / correctness oracle

**Generated corpus** (PyPulseq/MATLAB sequences built with known input params)
**+ cross-check against Pulseq's own `testReport()` / built-in calculators** as
an independent oracle. Two-sided ground truth, wired into the test harness from
the first metric onward.

## Deferred to v2+

Bloch/phantom simulation, CUDA/GPU acceleration, the formal plugin boundary, and
specialized pipelines (diffusion, elastography, non-Cartesian-specific). The v2
simulator must be re-derived from physics literature — never from mr-zero.

## Build order

| Step | Doc | Summary |
|------|-----|---------|
| 1 | [01-vendor-parser.md](01-vendor-parser.md) | Fork pulseq-rs → `pulseq-parse` (owned), parse the v1.5.1 example file |
| 2 | [02-crate-skeleton.md](02-crate-skeleton.md) | Library crate + result model + JSON schema + CLI shell |
| 3 | [03-integrity-checks.md](03-integrity-checks.md) | Sequence-integrity checks (no scanner model) |
| 4 | [04-derived-metrics.md](04-derived-metrics.md) | Derived metrics + the corpus/oracle test harness |
| 5 | [05-trajectory-geometry.md](05-trajectory-geometry.md) | Trajectory gate + dual-witness geometry |
| 6 | [06-scanner-hardware.md](06-scanner-hardware.md) | Scanner-profile subsystem + hardware/safety checks |
| 7 | [07-spec-assert-mode.md](07-spec-assert-mode.md) | Optional expected-spec assert mode |

## Open items (sensible defaults exist; not blocking)

- Project/crate name.
- Exact default tolerances (seed from harness values).
- Scanner-profile data source (seed GE from harness `emit_sys_ge.py`; need
  Siemens / generic).
