//! Parse every `*.seq` under `fixtures/` through the full IR. Guards that the
//! pulseq-parse parser handles each bundled example (trapezoid + free gradients,
//! rotation/label extensions) end to end — and, crucially, that it **never
//! panics**.
//!
//! `sos-liver.seq` (stack-of-stars) was the lone holdout: its rotated readout
//! mixes gradient axes with different shapes/delays, which the interpreter once
//! rejected. The general rotation path (resample the axes onto a common time
//! grid, then mix) handles this, so every
//! bundled fixture now interprets cleanly.

use std::time::Instant;

use seq_validate_core::Sequence;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");

#[test]
fn all_fixtures_parse_cleanly() {
    let mut paths: Vec<_> = std::fs::read_dir(FIXTURE_DIR)
        .expect("fixtures/ dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().map(|x| x == "seq").unwrap_or(false))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .seq fixtures found");

    let mut problems = Vec::new();
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let t = Instant::now();

        match Sequence::from_file(path) {
            Ok(seq) => {
                eprintln!(
                    "OK   {name:32} v{} blocks={:6} dur={:9.3}s warnings={} ({:?})",
                    seq.version,
                    seq.blocks.len(),
                    seq.total_duration,
                    seq.warnings.len(),
                    t.elapsed(),
                );
                for w in &seq.warnings {
                    eprintln!("        warning: {w}");
                }
                if !seq.warnings.is_empty() {
                    problems.push(format!("{name}: unexpected interpreter warning(s)"));
                }
            }
            Err(e) => {
                eprintln!("ERR  {name:32} {e}");
                problems.push(format!("{name}: unexpected error: {e}"));
            }
        }
    }

    assert!(problems.is_empty(), "fixture problems: {problems:#?}");
}
