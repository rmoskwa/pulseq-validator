# Step 3 — Sequence-integrity checks

**Goal:** The first real, cheapest checks — pure file/IR consistency, no scanner
model, no imaging-physics knowledge. Catches malformed sequences early.

**Depends on:** Steps 1–2.

## Why

Highest catch-rate-per-effort and lowest risk. Establishes the check-authoring
pattern the later, harder checks follow.

## Tasks

- [ ] **Raster alignment** — every event (RF, gradient, ADC, block duration)
      starts/lasts on its declared raster (`GradientRasterTime`,
      `RadiofrequencyRasterTime`, `AdcRasterTime`, `BlockDurationRaster`).
- [ ] **Block/timing consistency** — block durations are non-negative and
      accommodate their contained events; cumulative duration matches
      `TotalDuration` definition (within tolerance).
- [ ] **Event overlap / legality** — no illegal overlaps (e.g. RF during ADC
      where disallowed); referenced shape/event IDs all exist.
- [ ] **Dead-time / ring-down sanity** — RF ring-down and ADC dead-time present
      where the format/version requires (informational where scanner-specific).
- [ ] **Version / signature sanity** — `[VERSION]` recognized; `[SIGNATURE]`
      hash, if present, recomputes correctly (warn on mismatch, not fail).
- [ ] **Definitions sanity** — required definitions present; raster times
      positive; FOV present and positive.

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
