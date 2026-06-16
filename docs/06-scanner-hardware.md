# Step 6 — Scanner-profile subsystem + hardware/safety checks

**Goal:** Validate the sequence against real per-scanner hardware limits.

**Depends on:** Steps 1–5.

## Why

Hardware limits are scanner-specific and the `.seq` file usually doesn't carry
them. This step introduces the **scanner-profile** concept (a curated,
versioned limit set) and the safety checks that use it.

## Tasks — scanner-profile subsystem

- [ ] Profile type: `maxGrad`, `maxSlew`, `B1max`, RF duty/SAR proxies, raster
      times, min dead/ring-down times, gradient/RF coil constraints, PNS model
      params.
- [ ] **Bundled profiles**: seed `GE` from harness `emit_sys_ge.py`; add a
      `Generic 3T` and at least one Siemens profile. Each profile is **sourced
      and versioned** (cite the origin of every number).
- [ ] **Resolution order**: explicit `--profile <name>` / spec `scanner` field →
      limits in the `.seq` `[DEFINITIONS]` if present → error/`skip` if none
      selected (don't silently apply a wrong scanner).
- [ ] **Override**: user may override any single field (flag or spec).

## Tasks — checks

- [ ] **Gradient amplitude** ≤ `maxGrad` per axis and combined.
- [ ] **Slew rate** ≤ `maxSlew` per axis and combined.
- [ ] **ADC dwell vs raster** — dwell legal for the scanner's ADC raster.
- [ ] **RF B1 / duration** — peak B1 and pulse durations within limits.
- [ ] **Basic PNS** — simple model (e.g. SAFE-style or threshold proxy);
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
- Vendor documentation / Pulseq community for limit values (cite in profile
  files).

## Risks / notes

- Profiles are a **curation liability**: a stale number → false `fail`/`pass`.
  Version them and make overrides easy.
- PNS is genuinely hard to model precisely; keep v1 conservative and clearly
  labeled as approximate.
