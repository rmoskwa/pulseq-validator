# pulseq-validator

A Rust-first, clean-room validator for Pulseq `*.seq` files. Supply a `.seq`
file, get a quantitative report of its imaging metrics (TE, TR, FOV, matrix,
k-space trajectory) plus hardware-safety and integrity checks —
optionally asserted against an expected-value spec for CI gating.

v1 is **static/analytic only** (no Bloch simulation). A physics simulator is a
deferred v2 goal.

## Status

Early. The parser/IR (Step 1), the engine skeleton — result model, stable JSON
contract, and CLI shell (Step 2) — sequence integrity (Step 3: raster alignment,
timing/duration, event legality, version/signature, definitions), the derived
imaging metrics (Step 4: TR, effective TE, flip angle, n_slices, echo spacing,
scan time, with a MATLAB-generated oracle corpus), and the k-space trajectory
gate with dual-witness geometry (Step 5: `k = ∫G·dt` per-axis extent / coverage /
2D-vs-3D, plus FOV/matrix from both the Cartesian area-algebra and the trajectory,
reconciled) are in place; hardware and spec-assert checks land in Steps 6–7.

```console
$ seq-validate scan.seq            # human report
$ seq-validate scan.seq --json     # stable JSON (schema/report-v1.schema.json)
```

The CLI runs end-to-end today, grouping integrity, derived-metric, and trajectory
results by category; exit code is `0` on success, `1` on any check failure, `2` on
a parse/harness error. `--spec` / `--profile` are accepted but inactive until the
later steps.

See [`docs/`](docs/) for the design and the actionable build order.

- Design overview & decisions: [`docs/00-overview.md`](docs/00-overview.md)
- Build steps: [`docs/01`](docs/01-vendor-parser.md) … [`docs/07`](docs/07-spec-assert-mode.md)

## License

Permissive (MIT or Apache-2.0 — TBD before first public release). This project
is clean-room and **does not derive from MRzero/MRtwin** (AGPL-3.0 +
non-commercial EULA). The `.seq` parser in [`crates/pulseq-parse`](crates/pulseq-parse)
is a fork of the MIT-licensed `pulseq-rs`, now owned and developed here; MIT
attribution is retained in that crate's `LICENSE` and `NOTICE`.
