# pulseq-validator — for humans and AI agents

A Rust-first validator for [Pulseq](https://pulseq.github.io/) `*.seq` files.

Pass in a `.seq` file ---> return a quantitative report of the sequence's
imaging metrics (TE, TR, flip angle, FOV, matrix, k-space trajectory) together
with hardware-safety, vendor scanner compatability, and integrity checks.

Validation is **static and analytic**: every number comes
from the sequence's own gradients, RF, ADC, and timing, so a report is fast and
deterministic.

> [!NOTE] 
> Humans: Use pulseq-validator as a sanity check for what imaging metrics your sequence currently produces.

>[!NOTE] 
> AI Agents: Use pulseq-validator as a feedback and verification loop to incrementally self-correct on pulse sequence coding tasks.

## What is checked and validated

The validator runs four families of checks:

1. **Derived imaging metrics** — effective TE (the echo crossing k-space
   centre), TR, flip angle, slice count, echo spacing, in-plane resolution, and
   total scan time.
2. **K-space trajectory** — integrates `k = ∫G·dt` per axis to report extent,
   coverage, sampling uniformity, and a 2D-vs-3D classification. Validation is
   dimension-general: it follows permuted / non-Cartesian readouts and applies
   per-block rotation extensions.
3. **Hardware / safety limits** — gradient amplitude, slew rate, ADC dwell vs.
   raster, B1/duration, dead-time, and an approximate PNS estimate, compared
   against a selectable **scanner profile**.
4. **Sequence integrity** — raster alignment, block/timing consistency,
   transmit/receive overlaps, and version/signature sanity.

Geometry (FOV and matrix) is reported by **two independent witnesses**: a
parameter-algebra calculation that applies only when a Cartesian model holds, and
the trajectory measurement that applies generally. When the Cartesian model does
not hold the algebra reports `skip` (a first-class, non-failing result) and the
trajectory witness carries the geometry.

## How it works

```
.seq file
   │
   ▼
pulseq-parse ──► interpreted representation ──► seq-validate-core ──► Report ──► human / JSON
```

- **`crates/pulseq-parse`** parses the `.seq` file and lowers it to an
  *interpreted* representation — absolute block timing, applied rotations, and
  decompressed shapes — i.e. what the scanner would actually play out. It targets
  Pulseq **v1.5** (1.5.0 / 1.5.1); other versions are rejected.
- **`crates/seq-validate-core`** is the reusable engine library. It wraps the
  interpreted IR, runs the checks, and produces the result model and stable JSON
  report. Each check is a discrete unit registered in one place, so checks can be
  added or extracted without touching the others.
- **`crates/seq-validate`** is a thin CLI shell over the engine.

The result model is uniform: each check yields
`{ id, status, measured, expected?, severity, message }`, and only a `fail` drives
a nonzero exit code. Fix guidance, when a check has any, lives in `message` — there
is no separate hint/remediation field.

## Installing

### Prebuilt binary

Check recent [releases here.](https://github.com/rmoskwa/pulseq-validator/releases)
attaches a self-contained `seq-validate` executable for Linux (static musl),
macOS (Intel and Apple silicon), and Windows. Download the archive
for your platform, extract it, and put `seq-validate` on your `PATH` (or drop it somewhere locally)

### Build from source

The project is a standard Cargo workspace; building from source needs a recent
stable Rust toolchain ([rustup](https://rustup.rs/)).

```console
$ git clone <this-repo> && cd pulseq-validator
$ cargo build --release
```

The CLI binary lands at `target/release/seq-validate`. The examples below assume
it is on your `PATH`; otherwise run it through Cargo with
`cargo run --release -p seq-validate -- <args>`.

## Usage

```console
$ seq-validate scan.seq                       # human-readable report
$ seq-validate scan.seq --verbose             # + each check's measured/expected data
$ seq-validate scan.seq --json                # stable JSON (see the schema below)
$ seq-validate scan.seq --profile ge-premier  # + hardware/safety limits for a scanner
$ seq-validate scan.seq --spec expected.yaml  # + hard pass/fail vs an expected spec
```

The report groups results by category (integrity, metrics, trajectory, hardware,
and — when a spec is supplied — spec assertions). Each check has one of four
statuses:

| status | meaning                                                            |
|--------|--------------------------------------------------------------------|
| `pass` | the check held                                                     |
| `fail` | the check was violated — the only status that fails the run       |
| `warn` | suspicious but format-legal, or an approximate proxy               |
| `skip` | not applicable / not measurable for this sequence                  |

### Output modes

The default output is a colorized human report. `--verbose` appends each
check's structured `measured`/`expected` data inline.

`--json` emits a stable JSON document — the integration contract for Python or
web consumers, who need no bindings. It conforms to the JSON Schema at
[`crates/seq-validate-core/schema/report-v1.schema.json`](crates/seq-validate-core/schema/report-v1.schema.json). The schema is
embedded in the binary — `seq-validate --emit-report-schema` prints it and exits.

An AI agent or harness driving the validator programmatically should start from
[`AGENTS.md`](AGENTS.md), which states the JSON/exit-code loop and a worked example.

### Scanner profiles

`--profile <name>` selects a bundled scanner profile that supplies the hardware
limits. List the available profiles with `seq-validate --list-profiles` (add
`--json` for a machine-readable array). Each profile is one YAML file under
[`crates/seq-validate-core/profiles/`](crates/seq-validate-core/profiles/), embedded
into the binary at build time.

`--set FIELD=VALUE` overrides a single limit (repeatable), e.g.
`--set maxGrad=45`. With no `--profile`, no spec `scanner`, and no limits embedded
in the file's `[DEFINITIONS]`, the hardware checks `skip` — the wrong scanner is
never silently assumed.

### Expected-spec mode (CI gating)

`--spec <spec.yaml>` turns the validator into a pass/fail gate: each field the
spec provides becomes a `spec.*` check that asserts the measured value against the
expected one, and the run exits nonzero if any asserted field is out of tolerance.
The policy is **lenient** — only the fields you provide are checked.

A spec is YAML; see [`fixtures/t1_spgr_axial_brain.spec.yaml`](fixtures/t1_spgr_axial_brain.spec.yaml)
for a complete, passing example. Recognized fields:

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
`exact`.

The spec input has its own published JSON Schema at
[`crates/seq-validate-core/schema/spec-v1.schema.json`](crates/seq-validate-core/schema/spec-v1.schema.json)
(field types, units, the `[x, y, z]` vectors, the `tolerances` shape, and the
`none`/null opt-out). `seq-validate --emit-spec-schema` prints it the schema.

### Exit codes

| code | meaning                                                        |
|------|----------------------------------------------------------------|
| `0`  | success — no check failed                                      |
| `1`  | at least one check failed (including an out-of-tolerance spec) |
| `2`  | harness/parse error — the file could not be processed          |

## Scope

Validation is static/analytic only. Bloch/phantom simulation, GPU acceleration,
a formal plugin boundary, and sequence-family-specific pipelines (diffusion,
elastography, non-Cartesian) are currently not supported.

## Repository layout

```
crates/pulseq-parse        .seq parser + interpreted IR (fork of pulseq-rs)
crates/seq-validate-core   engine library: IR wrapper, checks, result model, JSON
crates/seq-validate        the seq-validate CLI
corpus/                    oracle corpus + MATLAB generator (provenance)
fixtures/                  example .seq files and an example spec
references/                the Pulseq v1.5.1 specification (PDF)
```

## License

Licensed under the [MIT License](LICENSE).
