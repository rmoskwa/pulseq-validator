//! Embeds the bundled scanner profiles (`profiles/*.yaml`) into the binary.
//!
//! Profiles are data files so the catalog grows by dropping a YAML into
//! `profiles/` (no code edit). This script globs that directory at build time and
//! generates a `(file stem, contents)` table of `include_str!`s, so the files are
//! embedded and change-tracked exactly like a hand-written `include_str!`.

#![allow(clippy::expect_used)] // a build script should fail loudly on a broken tree

use std::path::Path;
use std::{env, fs};

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let dir = Path::new(&manifest).join("profiles");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("profiles/ directory exists")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    files.sort();

    let mut out = String::from("static EMBEDDED: &[(&str, &str)] = &[\n");
    for path in &files {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("profile file name is UTF-8");
        // Re-run if a profile's contents change (the directory mtime alone misses
        // edits to existing files).
        println!("cargo:rerun-if-changed={}", path.display());
        out.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            stem,
            path.display().to_string()
        ));
    }
    out.push_str("];\n");

    let dest = Path::new(&env::var("OUT_DIR").expect("OUT_DIR set by cargo")).join("profiles.rs");
    fs::write(&dest, out).expect("write generated profiles.rs");
}
