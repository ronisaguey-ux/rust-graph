//! The default build stays free of third-party code, whatever the features add.

use std::fs;

#[test]
fn the_lockfile_holds_no_registry_crates() {
    let lock = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
    assert!(!lock.contains("source = "), "{lock}");
}

#[test]
fn the_grammar_crate_adds_no_dependencies() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/crates/rust-graph-deep/Cargo.toml");
    let manifest = fs::read_to_string(path).unwrap();
    let (_, dependencies) = manifest.split_once("[dependencies]").unwrap();
    // The macro compiles grammars with our own table generator, so it depends on
    // a workspace crate. What must never appear is third-party code: every
    // dependency has to be a path inside this repository.
    for line in dependencies.lines().map(str::trim).filter(|l| !l.is_empty()) {
        assert!(
            line.contains("path = "),
            "third-party dependency in the grammar crates: {line}\n{manifest}"
        );
    }
}
