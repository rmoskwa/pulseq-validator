# Step 3 — Sequence-integrity checks

**Goal:** The first real, cheapest checks — pure file/IR consistency, no scanner
model, no imaging-physics knowledge. Catches malformed sequences early.

**Depends on:** Steps 1–2.

## Why

Highest catch-rate-per-effort and lowest risk. Establishes the check-authoring
pattern the later, harder checks follow.

## Tasks

- [x] **Raster alignment** — every event (RF, gradient, ADC, block duration)
      starts/lasts on its declared raster (`GradientRasterTime`,
      `RadiofrequencyRasterTime`, `AdcRasterTime`, `BlockDurationRaster`).
      Each event aligns to *its own* raster (verified against all fixtures: a
      propeller RF delay sits off the gradient raster but on the RF raster, an
      EPI ADC delay off the gradient raster but on the ADC raster).
- [x] **Block/timing consistency** — cumulative duration matches the
      `TotalDuration` definition (within tolerance). Non-negativity and the
      "events fit their block" invariant are *parser-enforced* (model-layer
      `validate()`), so a violation is a harness error, not a check.
- [x] **Event overlap / legality** — flags simultaneous RF + ADC (transmit
      during receive). Referenced shape/event IDs are parser-resolved, so a
      dangling reference is a harness error before the checks run.
- [x] **Dead-time / ring-down sanity** — scanner-specific, so reported as a
      `skip`; the hard check lands in Step 6 against a profile.
- [x] **Version / signature sanity** — `[VERSION]` recognized; `[SIGNATURE]`
      md5, if present, recomputes (warn on mismatch, not fail).
- [x] **Definitions sanity** — raster times positive; FOV present and positive
      (missing FOV warns; required rasters' presence is parser-enforced).

## Acceptance criteria

- All integrity checks `pass` on the example file.
- Hand-corrupted fixtures (misaligned event, dangling shape ID, bad signature,
  duration mismatch) each produce the correct `fail`/`warn` with a useful
  message.
- Severity is right: structural corruption = `fail`/`error`; cosmetic/uncertain
  = `warn`.

## References

- pulsepal harness `seq_file.py` / `consolidate.py` for parsing-side invariants.
- Pulseq spec for per-version timing/dead-time rules.

## Risks / notes

- Some "rules" are scanner-specific (dead-times) — keep those as `warn` here;
  the hard versions live in Step 6 against a profile.
