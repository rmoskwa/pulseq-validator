# pulseq-validator

A Rust-first, clean-room validator for Pulseq `*.seq` files. Supply a `.seq`
file, get a quantitative report of its imaging metrics (TE, TR, FOV, matrix,
contrast, k-space trajectory) plus hardware-safety and integrity checks —
optionally asserted against an expected-value spec for CI gating.

v1 is **static/analytic only** (no Bloch simulation). A physics simulator is a
deferred v2 goal.

## Status

Greenfield. See [`docs/`](docs/) for the design and the actionable build order.

- Design overview & decisions: [`docs/00-overview.md`](docs/00-overview.md)
- Build steps: [`docs/01`](docs/01-vendor-parser.md) … [`docs/07`](docs/07-spec-assert-mode.md)

## License

Permissive (MIT or Apache-2.0 — TBD before first public release). This project
is clean-room and **does not derive from MRzero/MRtwin** (AGPL-3.0 +
non-commercial EULA). Pulseq and pulseq-rs are MIT and may be vendored with
attribution.
