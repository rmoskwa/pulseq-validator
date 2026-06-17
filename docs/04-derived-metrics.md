# Step 4 — Derived imaging metrics + the correctness oracle

**Goal:** Measure the headline metrics from first principles — and stand up the
generated-corpus + Pulseq-oracle test harness that proves they're correct.

**Depends on:** Steps 1–3.

## Why

This is the product's spine ("what is this sequence?"). It is also where trust
is won or lost, so the oracle harness is built here and reused by every later
metric.

## Tasks — metrics

All measured in `crates/seq-validate-core/src/metrics.rs` (a single
`DerivedMetrics` check emitting one result per metric, sharing one
excitation/echo-train analysis pass), ported from the pulsepal harness
`param_check.py`/`seq_file.py`. Reported in SI seconds / degrees.

- [x] **TR** — interval between successive excitations of the same slice
      (grouped by RF frequency offset); whole-sequence duration when a slice is
      excited once.
- [x] **TE (effective)** — k-space-centre ADC sample time minus excitation RF
      centre (the central-ky echo → effective/contrast TE for echo trains,
      reduces to first echo for single-echo). The phase-encode area is measured
      in the **logical (pre-rotation) frame** — see reconciliation 1 below.
- [x] **Flip angle** — `360 × ∫ RF envelope` (small-tip), median over excitations.
- [x] **n_slices** — count of distinct excitation RF frequency offsets.
- [x] **Echo spacing** — median centre-to-centre echo interval (echo trains
      only; `skip` for single-echo).
- [x] **Scan time** — total block duration.
- [x] FOV/matrix live in Step 5 (dual-witness); here, only the non-geometry
      metrics above.

## Tasks — oracle harness (build alongside the first metric)

Lives in `corpus/` (see `corpus/README.md`); the Rust gate is
`tests/corpus_oracle.rs`.

- [x] Corpus generator: **MATLAB `mr`-toolbox** scripts
      (`corpus/matlab/generate_corpus.m`) producing 1.5.1 `.seq` files with
      **known input params** across families (GRE/SPGR, SE, EPI, single- vs
      multi-slice, 2D/3D, multi-gradient-echo). MATLAB (not PyPulseq) because the
      parser accepts **only Pulseq 1.5.x** and the `mr` toolbox emits 1.5.1.
- [x] Recover-the-inputs test: measured metrics ≈ the generation params
      (`<name>.params.json`) within tolerance.
- [x] **Independent oracle**: cross-check measured TE/TR/duration against
      `seq.testReport()` (`<name>.report.json`). A bundled `fnint.m` shim lets
      testReport's k-space integration run without the Curve Fitting Toolbox.
- [x] Commit the generated artifacts per sequence — `.seq`, params sidecar,
      `report.json`, and the full `testReport()` text — so CI runs the Rust
      validator against committed files. MATLAB is **not** required on CI.

## Acceptance criteria — met

- **Measured metrics on the example are sane and documented** — see below; pinned
  in `tests/metrics.rs::example_metrics_are_sane_and_pinned`.
- **For every corpus sequence, measured ≈ generated AND ≈ Pulseq self-report** —
  8 sequences, 38 recover-the-inputs + 21 self-report checks pass.
- **Echo-train fixtures confirm effective-TE picks the k-centre echo, not the
  first** — HASTE 108 ms (9th echo, ESP 12 ms), PROPELLER 84 ms (6th echo, ESP
  14 ms); `tests/metrics.rs::echo_train_fixtures_pick_the_mid_train_k_centre_echo`.

## Measured on the example (`fixtures/t1_spgr_axial_brain.seq`)

| metric | value | metric | value |
|--------|-------|--------|-------|
| TR | 0.400048 s | flip angle | 80.0° |
| effective TE | 0.004008 s | n_slices | 44 |
| scan time | 76.809216 s | echo spacing | — (single echo) |

These agree with the pulsepal harness `param_check.py` to floating-point dust.

## Reconciliations (where the two-sided oracle earned its keep)

1. **Effective TE in rotated sequences.** The interpreted IR applies block
   rotations, so on PROPELLER its physical `gy` mixes readout into the
   phase-encode axis and the naïve k-centre pick lands on the *first* echo
   (13.998 ms) instead of the central one (83.998 ms). The harness measures the
   phase-encode area in the **logical** frame; we match it by exposing the
   `model`-layer (pre-rotation) gradient areas on the IR
   (`Sequence::logical_grad_areas`) and accumulating ky there. For unrotated
   sequences the two frames coincide.
2. **Non-comparable oracle fields.** `testReport`'s TE for a fixed-ky
   multi-gradient-echo is an ambiguous middle echo (vs our first-echo
   convention), and its TR for single-shot-per-slice EPI is the slice interval
   (vs our per-slice TR). These are recorded as `null` in the corpus
   `report.json` (skipped, not papered over) — see `corpus/README.md`.

## References

- pulsepal harness `param_check.py` — the measurement algorithms (our code).
- Pulseq MATLAB `mr`-toolbox `testReport()` — the corpus generator and the
  independent oracle.

## Risks / notes

- Effective-TE definition differences (first vs k-centre) are exactly where the
  two-sided oracle earns its keep — reconcile any disagreement deliberately.