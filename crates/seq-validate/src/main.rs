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
//! ```
//!
//! `--profile` selects the scanner [`Profile`] for the hardware/safety checks
//! (Step 6); `--set field=value` overrides one of its limits (repeatable). With no
//! `--profile` and no limits embedded in the file's `[DEFINITIONS]`, the hardware
//! checks `skip`. The human report shows the prose message per check; `--verbose`
//! also appends each check's structured `measured`/`expected` data inline (always
//! present in `--json`). `--spec` is accepted but inert until the spec-assert
//! subsystem (Step 7) lands.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use seq_validate_core::{Profile, Report, Sequence, checks, profile, render};

/// Validate a Pulseq `.seq` file: report its metrics, integrity, and safety, and
/// optionally assert them against an expected-value spec.
#[derive(Parser, Debug)]
#[command(name = "seq-validate", version, about, long_about = None)]
struct Cli {
    /// The `.seq` file to validate.
    #[arg(value_name = "FILE.seq")]
    file: PathBuf,

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

    /// Expected-value spec for hard pass/fail (accepted; active from Step 7).
    #[arg(long, value_name = "SPEC.yaml")]
    spec: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let file_label = cli.file.display().to_string();

    if cli.spec.is_some() {
        eprintln!(
            "note: --spec is accepted but not yet active (added in Step 7); it is ignored for now."
        );
    }

    // Parse + run checks, or capture a harness/parse error. Either way we end up
    // with one Report so `--json` always emits the same schema.
    let report = match Sequence::from_file(&cli.file) {
        Ok(seq) => match resolve_profile(&cli, &seq) {
            Ok(profile) => {
                let results = checks::run_all(&checks::CheckCtx {
                    seq: &seq,
                    profile: profile.as_ref(),
                });
                Report::for_sequence(file_label, &seq, results)
            }
            Err(err) => Report::harness_error(file_label, err),
        },
        Err(err) => Report::harness_error(file_label, err.to_string()),
    };

    if cli.json {
        println!("{}", report.to_json());
    } else {
        // Color only on a real terminal, and honor the NO_COLOR convention.
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        print!("{}", render(&report, color, cli.verbose));
    }

    ExitCode::from(report.exit_code() as u8)
}

/// Resolve the scanner profile per `docs/06`: the `--profile` name (or embedded
/// `[DEFINITIONS]` limits) then any `--set field=value` overrides. An unknown
/// profile name, a malformed `--set`, or an override with no base profile is an
/// error (surfaced as a harness error → exit 2), never a silent wrong scanner.
fn resolve_profile(cli: &Cli, seq: &Sequence) -> Result<Option<Profile>, String> {
    let mut resolved = profile::resolve(cli.profile.as_deref(), seq)?;
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
