# Step 6 — Scanner-profile subsystem + hardware/safety checks

**Goal:** Validate the sequence against real per-scanner hardware limits.

**Depends on:** Steps 1–5.

## Why

Hardware limits are scanner-specific and the `.seq` file usually doesn't carry
them. This step introduces the **scanner-profile** concept (a curated,
versioned limit set) and the safety checks that use it.

## Tasks — scanner-profile subsystem

- [x] Profile type: `maxGrad`, `maxSlew`, `B1max`, RF duty/SAR proxies, raster
      times, min dead/ring-down times, gradient/RF coil constraints, PNS model
      params.
- [x] **Bundled profiles**: seed `GE` from harness `emit_sys_ge.py`; add a
      `Generic 3T`. Each profile is **sourced
      and versioned** (cite the origin of every number).
- [x] **Resolution order**: explicit `--profile <name>` / spec `scanner` field →
      limits in the `.seq` `[DEFINITIONS]` if present → error/`skip` if none
      selected (don't silently apply a wrong scanner).
- [x] **Override**: user may override any single field (flag or spec).

## Tasks — checks

- [x] **Gradient amplitude** ≤ `maxGrad` per axis and combined.
- [x] **Slew rate** ≤ `maxSlew` per axis and combined.
- [x] **ADC dwell vs raster** — dwell legal for the scanner's ADC raster.
- [x] **RF B1 / duration** — peak B1 and pulse durations within limits.
- [x] **Basic PNS** — simple model (e.g. SAFE-style or threshold proxy);
      reported as `warn` unless a profile defines a hard limit.

## Acceptance criteria

- Example file passes against a matching profile.
- Fixtures that exceed each limit (slew, amplitude, B1) produce the correct
  `fail` with the offending value and the limit.
- Selecting no profile when none is embedded yields a clear, non-silent outcome.
- Each bundled profile number traces to a cited source.

## References

- pulsepal harness `emit_sys.py` / `emit_sys_ge.py` — per-vendor system limits
  (our code) — the seed for profiles.

## Risks / notes

- Profiles are a **curation liability**: a stale number → false `fail`/`pass`.
  Version them and make overrides easy.
- PNS is genuinely hard to model precisely; keep v1 conservative and clearly
  labeled as approximate.

## Status — done

- *Profile subsystem* (`profile.rs`). [`Profile`] carries the limits, raster grid,
  dead times and optional PNS params; every field cites its origin in the bundled
  data's `source` string. Two bundled profiles, each fully sourced: `ge-premier`
  (seeded from the harness `profiles/GE/ge-premier.yaml`) and `generic-3t` (Pulseq
  `mr.opts()` defaults at 3 T). Resolution order is `--profile` → file
  `[DEFINITIONS]` (`from_definitions`) → `skip`; an unknown name is an error, not a
  silent fallback. `--set field=value` overrides any single field.
- *Checks* (`hardware.rs`, all `hardware.*`): `gradient_amplitude` and `slew_rate`
  (per-axis hard limit; combined vector magnitude reported as informational
  `measured` data — **not** checked against the per-axis limit, which would
  false-positive on per-axis-limited amplifiers); `adc_dwell` (dwell divides the
  scanner ADC raster); `rf_b1` (peak B1 ≤ B1max); `dead_time` (RF dead/ring-down +
  ADC dead-time — the dimension `integrity.dead_time` defers here); and `pns` — the
  single-ramp closed form of the IEC 60601-2-33:2022 model (`pge2.pns`), `warn`
  only, clearly labelled approximate. `hardware.profile` reports the resolved
  scanner (or the clear, non-silent `skip` when none was selected), so the report
  is self-documenting without touching the JSON schema.
- *Units*: gradients (Hz/m) and RF (Hz) convert to mT/m, T/m/s and µT through the
  ¹H γ̄ (`DEFAULT_LARMOR_HZ`), field-strength-independent.

Measured on the v1.5.1 example (`t1_spgr_axial_brain.seq`) against `ge-premier`:
all hardware checks pass (peak slew 145 of 150 T/m/s, peak B1 10.5 of 20 µT, PNS
≈72 % — within normal mode), exit 0.

Tests: `profile.rs` inline unit tests (2), `tests/hardware.rs` synthetic
limit-breach fixtures (8: amplitude / slew / B1 / dwell / dead-time fail, PNS warn,
no-PNS skip, no-profile skip), `tests/cli.rs` acceptance (profile selects scanner,
unknown profile → exit 2, override drives a fail, no-profile non-silent skip). Full
workspace suite + `clippy -D warnings` + `rustfmt` green.
