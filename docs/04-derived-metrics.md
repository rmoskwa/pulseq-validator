# Step 4 — Derived imaging metrics + the correctness oracle

**Goal:** Measure the headline metrics from first principles — and stand up the
generated-corpus + Pulseq-oracle test harness that proves they're correct.

**Depends on:** Steps 1–3.

## Why

This is the product's spine ("what is this sequence?"). It is also where trust
is won or lost, so the oracle harness is built here and reused by every later
metric.

## Tasks — metrics

- [ ] **TR** — interval between successive excitations of the same slice.
- [ ] **TE (effective)** — k-space-centre ADC sample time minus excitation RF
      centre (the central-ky echo → correct effective/contrast TE for echo
      trains, reduces to first echo for single-echo).
- [ ] **Flip angle** — `360 × ∫ RF envelope` (small-tip).
- [ ] **n_slices** — count of distinct excitation RF frequency offsets.
- [ ] **Echo spacing** — median centre-to-centre echo interval (echo trains
      only; `skip` for single-echo).
- [ ] **Scan time** — from `TotalDuration` / block timing.
- [ ] FOV/matrix live in Step 5 (dual-witness); here, only the non-geometry
      metrics above.

## Tasks — oracle harness (build alongside the first metric)

- [ ] Corpus generator: **MATLAB `mr`-toolbox** scripts producing `.seq` files
      with **known input params** across families (GRE/SPGR, SE/TSE, EPI, single-
      vs multi-slice, 2D/3D). MATLAB (not PyPulseq) because the parser accepts
      **only Pulseq 1.5.x** (`raw/mod.rs`) and the `mr` toolbox emits 1.5.1, while
      the current PyPulseq release still writes 1.4.x.
- [ ] Recover-the-inputs test: assert the validator's measured metrics match the
      generation params within tolerance.
- [ ] **Independent oracle**: cross-check measured TE/TR/timing against MATLAB
      `seq.testReport()` / built-in calculators on the same sequence object.
- [ ] Commit the generated artifacts per sequence — the `.seq`, a params sidecar
      (the known inputs), and the `testReport()` golden output — so CI runs the
      Rust validator against committed files on every change. MATLAB is **not**
      required on the CI runner; it runs locally only to (re)generate the corpus.

## Acceptance criteria

- Measured metrics on the example file are sane and documented.
- For every corpus sequence, measured ≈ generated **and** measured ≈ Pulseq
  self-report (within per-metric tolerance).
- Echo-train fixtures confirm effective-TE picks the k-centre echo, not the
  first.

## References

- pulsepal harness `param_check.py` — the measurement algorithms (our code).
- Pulseq MATLAB `mr`-toolbox `testReport()` — the corpus generator and the
  independent oracle.

## Risks / notes

- Effective-TE definition differences (first vs k-centre) are exactly where the
  two-sided oracle earns its keep — reconcile any disagreement deliberately.