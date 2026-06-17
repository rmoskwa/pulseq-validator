# Oracle corpus

Small Pulseq **1.5.1** sequences with **known generation parameters**, used to
prove the validator's derived metrics (`docs/04-derived-metrics.md`) are correct.
Each sequence is checked two ways — *recover-the-inputs* and *Pulseq self-report*
— by the Rust test `crates/seq-validate-core/tests/corpus_oracle.rs`, which runs
on the committed artifacts. **MATLAB is not needed to run the tests**; it is only
needed to (re)generate the corpus.

## Layout — `data/<name>.*`

| file | what it is |
|------|------------|
| `<name>.seq` | the generated sequence (Pulseq 1.5.1) |
| `<name>.params.json` | the **known inputs** handed to the generator (the recover-the-inputs ground truth) |
| `<name>.report.json` | the scalars `seq.testReport()` measures **independently** from k-space (TE / TR / duration) — the second oracle |
| `<name>.testreport.txt` | the full `testReport()` text, committed for provenance |

A `null` in `params.json` / `report.json` means "not applicable / not comparable
for this family" and is skipped by the test (see the reconciliations below).

## Families

| name | what it exercises |
|------|-------------------|
| `gre2d_1slice` | single-slice spoiled GRE — baseline TE/TR/flip, single echo |
| `gre2d_3slice` | multi-slice GRE — `n_slices` from distinct RF frequency offsets |
| `spgr2d_1slice` | RF-spoiled GRE — different flip/TR/TE contrast point |
| `gre3d_8part` | 3-D GRE (non-selective block pulse, gz partition encode) — one slab ⇒ `n_slices` 1 |
| `mgre2d_1slice` | multi-gradient-echo (fixed ky) — echo spacing; first (k-centre) echo TE |
| `epi2d_1slice` | single-shot EPI — echo train: central-ky effective TE + echo spacing |
| `epi2d_3slice` | multi-slice single-shot EPI |
| `se2d_1slice` | spin echo — a `use=refocusing` 180° pulse excluded from the excitation count |

The bundled fixtures (`fixtures/`) cover the rotated / long-train families this
corpus omits (FSE PROPELLER, HASTE, EPI-RS), cross-checked against the pulsepal
harness in `tests/metrics.rs`.

## Two deliberate reconciliations

The two-sided oracle exists to surface definition mismatches; two are recorded as
`null` (not-comparable) in the sidecars rather than papered over:

- **`mgre2d` TE.** A multi-gradient-echo at fixed ky has several echoes that all
  cross k-space centre. Our effective TE is the **first** (shortest-TE) echo;
  `testReport` picks an ambiguous middle echo (its own "TODO: detect multiple
  TEs"). So `report.json.te_s` is `null`; the generated `te_s` (the first echo)
  is the ground truth.
- **EPI TR.** With one excitation per slice, `testReport`'s TR is the slice
  interval, while our per-slice TR falls back to the whole-scan duration. Not the
  same quantity, so `report.json.tr_s` is `null` for the EPI families.

## Regenerating

Requires MATLAB with the Pulseq `mr`-toolbox on the path. The bundled
`matlab/fnint.m` shim lets `testReport`'s k-space integration run **without** the
Curve Fitting Toolbox (it implements the antiderivative of a piecewise polynomial
that the toolbox's Octave path otherwise needs).

```bash
# point at your mr-toolbox checkout (or addpath() it yourself before running)
export PULSEQ_MATLAB=/path/to/pulseq/matlab
matlab -batch "run('corpus/matlab/generate_corpus.m')"
```

The generator is fault-tolerant per family and prints a one-line summary per
sequence. Re-running overwrites `data/`.
