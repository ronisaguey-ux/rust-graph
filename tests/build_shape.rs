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
    assert!(dependencies.trim().is_empty(), "{manifest}");
}
