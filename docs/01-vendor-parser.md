# Step 1 — Vendor the parser, extend to v1.5.1

**Goal:** A vendored, owned `.seq` parser that cleanly parses the example file
(`t1_spgr_axial_brain.seq`, Pulseq v1.5.1) into an interpreted IR.

**Depends on:** nothing (foundation).

## Why

Every downstream check sits on the IR. `pulseq-rs` (pulseq-frame) already has a
working parser with `raw` and interpreted (`int`) layers and decompressed shape
handling, but it predates v1.5.1. We vendor (not git-depend) so we own the
forward-porting and version it here.

## Tasks

- [ ] Confirm pulseq-rs's license permits vendoring; record it and add
      attribution (e.g. `vendor/pulseq-rs/NOTICE`).
- [ ] Copy pulseq-rs source into `vendor/pulseq-rs/` (or a workspace member);
      wire it into the Cargo workspace.
- [ ] Build and run its parse on the example file; catalogue every failure
      caused by v1.5.1 format differences.
- [ ] Implement v1.5.1 deltas vs. the prior version (diff the Pulseq spec /
      `vendor/pulseq` reference): `[VERSION]` handling, any new/changed section
      formats, shape encodings, extension definitions.
- [ ] Expose a stable internal IR type the rest of the crate consumes
      (interpreted layer: blocks with absolute start time, duration, optional
      RF/Gx/Gy/Gz/ADC, rotation/transform, decompressed shapes; plus
      `[DEFINITIONS]`).
- [ ] Keep the raw layer addressable for debugging / round-trip.

## Acceptance criteria

- The example `.seq` parses with no errors.
- Parse is **O(n)** in block count (no per-block re-scan of the whole table).
- A snapshot/round-trip test asserts the IR matches expected block count,
  definitions, and timing for the example file.
- Parse time on the example file (~50k lines) is well under a second.

## References

- `pulseq-rs` (pulseq-frame, MIT-assumed) — `raw`, `int`, `seq` modules.
- mr-zero-clone `src/seq_import.rs` — an example of consuming the `int` layer
  (reference only; AGPL — do not copy).
- pulsepal harness `seq_file.py` — a proven Python parser as a logic reference
  (our own code).
- Pulseq spec doc in `…/pulseq-ge-assistant/vendor/pulseq/doc`.

## Risks / notes

- pulseq-rs license must be permissive for vendoring under MIT/Apache — verify
  first.
- The harness flagged mr-zero's import as accidentally O(n²); that was the
  Python re-scan wrapper, not pulseq-rs — but re-confirm our IR stays O(n).
