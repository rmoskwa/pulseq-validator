//! `seq-validate` — the CLI shell for the Pulseq `.seq` validator.
//!
//! Parses a `.seq` file into the interpreted IR, runs the registered checks
//! (none yet — Steps 3–6 add them), and emits a [`Report`] either human-readable
//! or as stable JSON. The exit code follows the policy on
//! [`Report::exit_code`]: `2` on a harness/parse error, `1` on any check `fail`,
//! else `0`.
//!
//! ```text
//! seq-validate <file.seq> [--json] [--profile <name>] [--spec <spec.yaml>]
//! ```
//!
//! `--profile` and `--spec` are accepted now (Step 2 wires arg parsing) but only
//! take effect once the hardware (Step 6) and spec-assert (Step 7) subsystems
//! land; supplying them prints a note until then.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use seq_validate_core::{Report, Sequence, checks, render};

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

    /// Scanner profile for hardware/safety limits (accepted; active from Step 6).
    #[arg(long, value_name = "NAME")]
    profile: Option<String>,

    /// Expected-value spec for hard pass/fail (accepted; active from Step 7).
    #[arg(long, value_name = "SPEC.yaml")]
    spec: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let file_label = cli.file.display().to_string();

    if cli.profile.is_some() || cli.spec.is_some() {
        eprintln!(
            "note: --profile/--spec are accepted but not yet active \
             (added in later build steps); they are ignored for now."
        );
    }

    // Parse + run checks, or capture a harness/parse error. Either way we end up
    // with one Report so `--json` always emits the same schema.
    let report = match Sequence::from_file(&cli.file) {
        Ok(seq) => {
            let results = checks::run_all(&checks::CheckCtx { seq: &seq });
            Report::for_sequence(file_label, &seq, results)
        }
        Err(err) => Report::harness_error(file_label, err.to_string()),
    };

    if cli.json {
        println!("{}", report.to_json());
    } else {
        // Color only on a real terminal, and honor the NO_COLOR convention.
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        print!("{}", render(&report, color));
    }

    ExitCode::from(report.exit_code() as u8)
}
