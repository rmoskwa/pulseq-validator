//! The check catalog (`checks::catalog`) must document every result `id` the
//! registry can emit — that lockstep is what keeps `seq-validate --list-checks`
//! honest, since the aggregate checks emit ids that do not match their trait-level
//! `id()`. The descriptions are generated from the registry, not a hand-kept copy.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;

use seq_validate_core::checks::{self, run_all};
use seq_validate_core::{CheckCtx, Profile, Sequence};

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/t1_spgr_axial_brain.seq"
);

#[test]
fn every_emitted_id_is_documented_in_the_catalog() {
    let seq = Sequence::from_file(EXAMPLE).expect("bundled example must parse");
    // A profile is needed to exercise every `hardware.*` id: without one only
    // `hardware.profile` is emitted, so the run must carry a profile to cover the
    // whole check space.
    let profile = Profile::by_name("ge-premier").expect("bundled profile");
    let results = run_all(&CheckCtx {
        seq: &seq,
        profile: Some(&profile),
    });

    let catalog = checks::catalog();
    let documented: BTreeSet<&str> = catalog.iter().map(|d| d.id.as_str()).collect();
    for r in &results {
        assert!(
            documented.contains(r.id.as_str()),
            "result id `{}` is emitted but missing from the catalog",
            r.id
        );
    }
}

#[test]
fn catalog_entries_are_unique_and_described() {
    let catalog = checks::catalog();
    assert!(
        !catalog.is_empty(),
        "the catalog is generated from the registry, which is non-empty"
    );
    let mut seen = BTreeSet::new();
    for d in &catalog {
        assert!(
            !d.summary.trim().is_empty(),
            "catalog entry `{}` has an empty summary (a check forgot to override summary()/docs())",
            d.id
        );
        assert!(
            seen.insert(d.id.as_str()),
            "duplicate catalog id `{}`",
            d.id
        );
    }
}
