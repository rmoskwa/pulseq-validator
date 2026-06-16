# Step 1 — Fork the parser, own it

**Goal:** A forked, owned `.seq` parser crate that cleanly parses the example
file (`t1_spgr_axial_brain.seq`, Pulseq v1.5.1) into an interpreted IR.

**Depends on:** nothing (foundation).

**Status:** ✅ Done. The parser lives at `crates/pulseq-parse` and the IR at
`crates/seq-validate-core`.

## Why

Every downstream check sits on the IR. `pulseq-rs` (pulseq-frame) already has a
working parser with `raw` and interpreted (`int`) layers and decompressed shape
handling — and, as it turned out, the pinned commit (`865a890`) **already
supports Pulseq 1.2 through 1.5.1**, so no forward-porting was needed (the
earlier assumption that it predated v1.5.1 was wrong). We **fork** it (not a git
dependency, and not a tracked vendor copy): the code is copied in once, renamed
to `pulseq-parse`, and developed independently from here. We own it; upstream is
a starting point, not something we sync against. MIT attribution is retained in
the crate's `LICENSE` + `NOTICE`.

## Tasks

- [x] Confirm pulseq-rs's license permits this; record it and add attribution.
      → MIT (Copyright (c) 2024 Jonathan Endres). Attribution in
      `crates/pulseq-parse/{LICENSE,NOTICE}`.
- [x] Copy pulseq-rs source into a workspace member; wire it into the Cargo
      workspace. → `crates/pulseq-parse` (lean copy: parser library only — the
      upstream HTML viewer, its `bin`/feature/optional-deps, upstream tests, and
      `assets/` were dropped).
- [x] Build and run its parse on the example file; catalogue v1.5.1 failures.
      → None. The example parses with **0 errors / 0 warnings**; the commit
      already handles v1.5.1, so the "implement v1.5.1 deltas" task was moot.
- [x] Expose a stable internal IR the rest of the crate consumes (interpreted
      layer: blocks with **absolute start time**, duration, optional
      RF/Gx/Gy/Gz/ADC, rotation/transform, decompressed shapes; plus
      `[DEFINITIONS]` and `[VERSION]`). → `seq_validate_core::ir::Sequence`,
      which wraps the `interp` layer and adds the file provenance the lower
      layers drop: `[DEFINITIONS]` and `[VERSION]`. Absolute block start times
      and total duration live on the `interp` layer itself
      (`interp::Sequence::{block_starts, duration}`, an O(n) cumulative sum),
      since they are a pure function of what the scanner plays.
- [x] Keep the raw layer addressable for debugging / round-trip. →
      `seq_validate_core::raw_sections()`.
- [x] (Added) Make the interpreter **never panic** on input. Upstream
      `transform_grad` called `unimplemented!()` for a rotation across gradient
      axes with different shapes/delays (stack-of-stars). Converted to a typed
      recoverable error (`InterpreterError::UnsupportedRotation { block_id }`);
      the real resampling math is deferred to Step 5. First entry in the fork's
      divergence log (`crates/pulseq-parse/NOTICE`).

## Acceptance criteria — met

- The example `.seq` parses with no errors. ✅ (0 err / 0 warn, 50 688 blocks,
  total duration 76.809216 s.)
- Parse is **O(n)** in block count. ✅ (cumulative-sum timing; a per-block
  tripwire guards against accidental quadratic behaviour.)
- A snapshot/round-trip test asserts block count, definitions, and timing for
  the example file. ✅ (`crates/seq-validate-core/tests/parse_example.rs`.)
- Parse time on the example file (~50k lines) is well under a second. ✅
  (~30–60 ms release; perf assertion is build-aware for the slow debug path.)

A second test (`tests/parse_all_fixtures.rs`) guards that **every** bundled
fixture parses or degrades gracefully (never panics); `sos-liver.seq` is the
lone expected-unsupported case until Step 5 implements rotation resampling.

## References

- `pulseq-rs` (pulseq-frame, **MIT** — confirmed) — `raw`, `int`, `seq` modules;
  the fork point is commit `865a890`. (In our fork we renamed `int`→`interp`
  and `seq`→`model`; the upstream names are kept here since that is what you
  diff against. See `crates/pulseq-parse/NOTICE`.)
- mr-zero-clone `src/seq_import.rs` — an example of consuming the `int` layer
  (upstream's name for what we call `interp`; reference only — AGPL, do not copy).
- pulsepal harness `seq_file.py` — a proven Python parser as a logic reference
  (our own code).
- Pulseq v1.5.1 spec — kept at `docs/refs/pulseq-spec-1.5.1.pdf`.

## Notes

- The harness flagged mr-zero's import as accidentally O(n²); that was the
  Python re-scan wrapper, not pulseq-rs — our IR is confirmed O(n).
