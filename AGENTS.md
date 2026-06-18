# AGENTS.md

## The loop

1. **Always pass `--json`.** It emits one stable JSON document where `measured`
   and `expected` are always present (`null` when a check set neither). The human
   report only shows them with `--verbose`, so don't parse that.
2. **Branch on the exit code first** — it is the cheapest signal:
   `0` valid · `1` a check failed · `2` the file could not be processed. Only
   `status: "fail"` drives `1`; a `2` means parse/harness error and the document
   carries no results.
3. **Then route on `id` + `status`** for *what* to fix.
4. **Treat `skip` as "not applicable," never as a failure.** `warn` and `skip`
   never change the exit code.

## Usage

```console
$ seq-validate scan.seq --json                          # checks only, no hardware limits
$ seq-validate scan.seq --profile ge-premier --json     # + hardware/safety limits
$ seq-validate scan.seq --spec expected.yaml --json     # + hard pass/fail vs an expected spec
$ seq-validate scan.seq --profile ge-premier --set maxGrad=45 --json   # override one limit (repeatable)
$ seq-validate --list-profiles --json                    # enumerate the bundled scanner profiles and exit
$ seq-validate --emit-report-schema                      # print the report JSON Schema and exit
$ seq-validate --emit-spec-schema                        # print the spec JSON Schema and exit
```

`--profile <name>` supplies the hardware limits. Do **not** hardcode the profile
list: discover it at runtime with `seq-validate --list-profiles --json`, which
prints an array of `{name, vendor, description, aliases}`. The definitions live as
one YAML file each under `crates/seq-validate-core/profiles/`, so an agent can also
read them directly. With no `--profile`, no `scanner` in the spec, and no limits in
the file's `[DEFINITIONS]`, the `hardware.*` checks `skip` rather than assume a
scanner.

### Exit codes

| code | meaning |
|---|---|
| `0` | success — no check failed |
| `1` | at least one check failed (including an out-of-tolerance spec field) |
| `2` | harness/parse error — the file could not be processed |

## The JSON document

Top-level keys: `schema_version`, `file`, `error`, `sequence`, `results`,
`summary`. On a `2` (parse/harness error) `error` is the message and both
`sequence` and `results` are empty/null; otherwise `error` is `null` and
`results` carries one entry per check, each:

```json
{ "id": "...", "status": "pass|fail|warn|skip", "measured": {}, "expected": {}, "severity": "...", "message": "..." }
```

`message` is the human-readable explanation and is where the fix guidance lives. Read `message` for *how* to fix; route on `id` + `status` (above) for *what*.

The full contract is the JSON Schema at
[`crates/seq-validate-core/schema/report-v1.schema.json`](crates/seq-validate-core/schema/report-v1.schema.json),
also embedded in the binary (`seq-validate --emit-report-schema`).
`schema_version` pins the contract; any breaking change bumps it.

## Routing on `id` + `status`

Each `id` is `category.check`. Branch on the prefix to decide who handles it:

| prefix | meaning |
|---|---|
| `integrity.*` | the file is malformed / internally inconsistent |
| `metrics.*` | a derived imaging metric (TE, TR, flip, matrix, FOV, …) |
| `trajectory.*` | the measured k-space trajectory and geometry |
| `hardware.*` | a scanner limit (needs a profile; else `skip`) |
| `spec.*` | a field asserted against the supplied `--spec` (only in spec mode) |

`status` decides whether to act:

| status | meaning |
|---|---|
| `pass` | the check held |
| `fail` | the check was violated — **the only status that fails the run** |
| `warn` | suspicious but format-legal, or an approximate proxy |
| `skip` | not applicable / not measurable for this sequence |

## Spec mode (asserting your intended values)

`--spec expected.yaml` turns each provided field into a `spec.*` check and exits
nonzero if any is out of tolerance. The policy is **lenient** — only the fields
you supply are checked. A spec is YAML; the recognized fields:

```yaml
name: my-scan
scanner: ge-premier            # selects the hardware profile (an input, not asserted)
te_ms: 4.008
tr_ms: 400.048
flip_angle_deg: 80
n_slices: 44
matrix: [192, 192, 1]          # nominal (logical) sizes
fov_mm: [240, 240]
oversampling: [2, 1, 1]        # readout oversampling, divided out before comparison
```

Per-field tolerances default to sensible bands and can be set as `abs`, `rel`, or
`exact`. The full field list, units, vector shapes, and tolerance schema are the
spec JSON Schema at
[`crates/seq-validate-core/schema/spec-v1.schema.json`](crates/seq-validate-core/schema/spec-v1.schema.json)
(`seq-validate --emit-spec-schema` prints it), so the spec format is learnable
from the tool alone.

## Worked example: a failing `hardware.rf_b1`

Run with a scanner profile, the peak-B1 check fails, the process exits `1`, and
the offending result reads:

```json
{
  "id": "hardware.rf_b1",
  "status": "fail",
  "measured": { "peak_b1_ut": 10.49, "limit_ut": 5.0, "longest_rf_s": 0.002 },
  "expected": null,
  "severity": "error",
  "message": "peak B1 10.5 µT (block 1) exceeds B1max 5.0 µT"
}
```

How to read it: `measured.peak_b1_ut` (10.49 µT) is above `measured.limit_ut`
(5.0 µT) — for hardware checks both the observed value and the limit live inside
`measured`, and `expected` is `null` (it is populated by `spec.*` checks). The
`message` names the block (1) that peaks. The fix is upstream in the sequence,
not in the validator: lower the RF amplitude or lengthen the pulse for block 1
until `peak_b1_ut ≤ limit_ut`, then re-run. (If the limit itself is wrong, pick a
profile that matches the target scanner — `peak_b1_ut` is a property of the
sequence, `limit_ut` of the profile.)
