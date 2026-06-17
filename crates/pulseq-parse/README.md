# pulseq-parse

Parses [Pulseq](https://pulseq.github.io/) `.seq` files and lowers them into a
representation that mirrors what a scanner actually executes.

This crate is a **fork of `pulseq-rs`** (MIT, by Jonathan Endres), now owned and developed inside the pulseq-validator.

## Crate structure

The crate is organized into three layers, each lowered from the previous one:

- `raw` &mdash; the .seq file as parsed, almost 1:1. Sections, blocks, events,
  shapes and extensions are kept as separate tables indexed by their IDs.
  Targets Pulseq 1.5 (1.5.0 / 1.5.1); any other version is rejected with
  `UnsupportedVersion`. The 1.2&ndash;1.4 dialects this crate's ancestor handled
  were dropped &mdash; see [`NOTICE`](NOTICE).
- `model` &mdash; an idiomatic, validated representation. IDs are resolved to
  `Arc`-shared references, definitions are parsed, known extensions (labels,
  triggers, soft delays, rotations, RF shims) are recognized, and the
  required-section / event-duration invariants are enforced. (Formerly `seq`.)
- `interp` &mdash; the *interpreted* sequence: what the scanner would actually
  play out. The model&rarr;interp step applies FOV scaling and rotation, folds
  relative and offset RF/ADC freq+phase via the Larmor frequency, applies the
  rotation extension to gradients, resolves soft delays, computes per-ADC
  label snapshots, lifts `ONCE` / `PMC` / triggers into the block, and unifies
  the two possible RF shim sources. (Formerly `int`.)

Most users only need `model` (to inspect a file) or `interp` (to know what
plays on the scanner). `raw` is exposed for debugging and round-trip inspection.

In this repo, the validator usually consumes the higher-level IR in
`seq-validate-core` (which wraps `interp` and adds absolute block start times)
rather than calling this crate directly.

## Example: loading a sequence with `interp`

```rust
use std::collections::HashMap;
use pulseq_parse::{model, interp};

let seq = model::Sequence::from_file("example.seq")?;

let (int_seq, warnings) = interp::Sequence::from_seq(
    &seq,
    interp::Transform::default(),  // identity FOV (scale = 1, no rotation)
    42_577_468.8,               // 1H Larmor frequency @ 1 T [Hz]
    HashMap::new(),             // no soft-delay overrides
)?;

for w in warnings {
    eprintln!("warning: {w}");
}

for block in &int_seq.blocks {
    if let Some(adc) = &block.adc {
        println!(
            "ADC: {} samples, dwell {} s, lin={} par={}",
            adc.num, adc.dwell, adc.labels.lin, adc.labels.par,
        );
    }
}
# Ok::<(), pulseq_parse::Error>(())
```