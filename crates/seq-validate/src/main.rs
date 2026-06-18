//! `seq-validate` — the CLI shell for the Pulseq `.seq` validator.
//!
//! Parses a `.seq` file into the interpreted IR, runs the registered checks
//! (integrity, derived metrics, trajectory geometry, and hardware/safety), and
//! emits a [`Report`] either human-readable or as stable JSON. The exit code
//! follows the policy on
//! [`Report::exit_code`]: `2` on a harness/parse error, `1` on any check `fail`,
//! else `0`.
//!
//! ```text
//! seq-validate <file.seq> [--json] [-v|--verbose] [--profile <name>] [--set field=value]... [--spec <spec.yaml>]
//! seq-validate --emit-spec-schema | --emit-report-schema
//! ```
//!
//! `--emit-spec-schema` / `--emit-report-schema` print the embedded JSON Schema
//! for the `--spec` input / the `--json` report output and exit 0, so a harness
//! can learn either contract from the binary alone (no `.seq` file required).
//!
//! `--profile` selects the scanner [`Profile`] for the hardware/safety checks;
//! `--set field=value` overrides one of its limits (repeatable). With no
//! `--profile` and no limits embedded in the file's `[DEFINITIONS]`, the hardware
//! checks `skip`. The human report shows the prose message per check; `--verbose`
//! also appends each check's structured `measured`/`expected` data inline (always
//! present in `--json`). `--spec <spec.yaml>` asserts the measured metrics
//! against an expected-value spec: each provided field becomes a `spec.*` check and
//! the run exits nonzero if any asserted field is out of tolerance.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use seq_validate_core::{Measurements, Profile, Report, Sequence, Spec, checks, profile, render};

/// Validate a Pulseq `.seq` file: report its metrics, integrity, and safety, and
/// optionally assert them against an expected-value spec.
#[derive(Parser, Debug)]
#[command(name = "seq-validate", version, about, long_about = None)]
struct Cli {
    /// The `.seq` file to validate. Optional only when emitting a schema.
    #[arg(
        value_name = "FILE.seq",
        required_unless_present_any = ["emit_spec_schema", "emit_report_schema"]
    )]
    file: Option<PathBuf>,

    /// Emit the report as stable JSON instead of the human-readable form.
    #[arg(long)]
    json: bool,

    /// In the human report, also show each check's measured/expected data inline.
    #[arg(short, long)]
    verbose: bool,

    /// Scanner profile for hardware/safety limits (e.g. `ge-premier`, `generic-3t`).
    #[arg(long, value_name = "NAME")]
    profile: Option<String>,

    /// Override one profile limit, e.g. `--set maxGrad=45` (repeatable).
    #[arg(long = "set", value_name = "FIELD=VALUE")]
    set: Vec<String>,

    /// Expected-value spec for hard pass/fail: assert measured metrics against it.
    #[arg(long, value_name = "SPEC.yaml")]
    spec: Option<PathBuf>,

    /// Print the `--spec` input JSON Schema (schema/spec-v1.schema.json) and exit 0.
    #[arg(long)]
    emit_spec_schema: bool,

    /// Print the `--json` report JSON Schema (schema/report-v1.schema.json) and exit 0.
    #[arg(long)]
    emit_report_schema: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Schema-introspection flags: print the embedded JSON Schema and exit 0, so a
    // harness can learn the input/output contract from the binary alone. These
    // take no `.seq` file (enforced by `required_unless_present_any` on `file`).
    if cli.emit_spec_schema {
        print!("{}", seq_validate_core::SPEC_SCHEMA);
        return ExitCode::SUCCESS;
    }
    if cli.emit_report_schema {
        print!("{}", seq_validate_core::REPORT_SCHEMA);
        return ExitCode::SUCCESS;
    }

    // clap guarantees `file` is present here (required unless an --emit flag);
    // model the `None` arm explicitly to stay clear of unwrap/expect.
    let Some(file) = cli.file.as_deref() else {
        eprintln!("error: the <FILE.seq> argument is required");
        return ExitCode::from(2);
    };
    let file_label = file.display().to_string();

    // Build one Report whatever happens (parse error, profile/spec error, or a
    // full run) so `--json` always emits the same schema.
    let report = build_report(&cli, file, file_label);

    if cli.json {
        println!("{}", report.to_json());
    } else {
        // Color only on a real terminal, and honor the NO_COLOR convention.
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        print!("{}", render(&report, color, cli.verbose));
    }

    ExitCode::from(report.exit_code() as u8)
}

/// Parse the sequence, the optional spec, resolve the profile, run the checks, and
/// (when a spec is given) append the `spec.*` assertions. Any step that fails
/// becomes a harness-error [`Report`] (exit 2) so the JSON schema is uniform.
fn build_report(cli: &Cli, file: &Path, file_label: String) -> Report {
    let seq = match Sequence::from_file(file) {
        Ok(seq) => seq,
        Err(err) => return Report::harness_error(file_label, err.to_string()),
    };

    // Load the spec first: its `scanner` field can select the profile. Parsing
    // also yields non-fatal diagnostics (the `spec.unrecognized_fields` warning).
    let (spec, spec_warnings) = match cli.spec.as_deref() {
        Some(path) => match Spec::from_yaml_file(path) {
            Ok((spec, warnings)) => (Some(spec), warnings),
            Err(err) => return Report::harness_error(file_label, format!("spec: {err}")),
        },
        None => (None, Vec::new()),
    };

    let profile = match resolve_profile(cli, &seq, spec.as_ref().and_then(|s| s.scanner.as_deref()))
    {
        Ok(profile) => profile,
        Err(err) => return Report::harness_error(file_label, err),
    };

    let mut results = checks::run_all(&checks::CheckCtx {
        seq: &seq,
        profile: profile.as_ref(),
    });
    // Spec-parse diagnostics lead the Spec section (they precede the assertions);
    // `Measurements::from_results` only reads `metrics.*`/`trajectory.*`, so the
    // warning's presence here does not affect the assertions.
    results.extend(spec_warnings);
    if let Some(spec) = &spec {
        // Spec assertions reuse the measured values from the file-only checks,
        // read through the typed `Measurements` surface.
        let assertions = spec.assert(&Measurements::from_results(&results));
        results.extend(assertions);
    }
    Report::for_sequence(file_label, &seq, results)
}

/// Resolve the scanner profile: the `--profile` name, else the
/// spec's `scanner` field, else embedded `[DEFINITIONS]` limits — then any `--set
/// field=value` overrides. An unknown profile name, a malformed `--set`, or an
/// override with no base profile is an error (surfaced as a harness error → exit
/// 2), never a silent wrong scanner.
fn resolve_profile(
    cli: &Cli,
    seq: &Sequence,
    spec_scanner: Option<&str>,
) -> Result<Option<Profile>, String> {
    // An explicit `--profile` wins over the spec's `scanner`; either is an explicit
    // selection, so an unknown name is an error rather than a silent fallback.
    let name = cli.profile.as_deref().or(spec_scanner);
    let mut resolved = profile::resolve(name, seq)?;
    for entry in &cli.set {
        let (field, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("malformed --set {entry:?}; expected FIELD=VALUE"))?;
        let value: f64 = value
            .trim()
            .parse()
            .map_err(|_| format!("--set {field}: {value:?} is not a number"))?;
        let p = resolved.as_mut().ok_or_else(|| {
            format!("--set {field} given but no profile to override; pass --profile <name>")
        })?;
        p.apply_override(field.trim(), value)?;
    }
    Ok(resolved)
}
