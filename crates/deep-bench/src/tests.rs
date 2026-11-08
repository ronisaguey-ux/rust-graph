//! The harness's own tests: the manifest is verified, the statistics are exact,
//! a missing parser is refused with a reason, and the default build is provably
//! the one from before this field existed.

use super::bench::{stats, vm_hwm, Stats};
use super::manifest::Manifest;
use super::*;
use std::fs;
use std::path::{Path, PathBuf};

/// A throwaway directory with `files` names in it, all non-empty.
fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("deep-bench-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn manifest_text(files: &[PathBuf]) -> String {
    files.iter().map(|path| format!("{}\n", path.display())).collect()
}

fn write_manifest(root: &Path, text: &str) -> String {
    let path = root.join("corpus.list");
    fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn sha256_of_the_included_module_is_fips_correct() {
    assert_eq!(hex(&sha256::digest(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex(&sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn a_manifest_counts_and_sums_every_file() {
    let root = scratch("sums");
    let (a, b) = (root.join("a.rs"), root.join("b.rs"));
    fs::write(&a, b"fn main() {}\n").unwrap();
    fs::write(&b, b"let x = 1;\n").unwrap();
    let list = write_manifest(&root, &manifest_text(&[a, b]));

    let manifest = Manifest::load(&list).unwrap();
    assert_eq!(manifest.files.len(), 2);
    assert_eq!(manifest.bytes, 13 + 11);
    // The digest covers the manifest bytes, not the corpus.
    assert_eq!(manifest.digest, hex(&sha256::digest(fs::read(&list).unwrap().as_slice())));
    assert!(manifest.summary().contains("files: 2"));
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let root = scratch("comments");
    let a = root.join("a.rs");
    fs::write(&a, b"x").unwrap();
    let list = write_manifest(&root, &format!("# a corpus\n\n{}\n\n# done\n", a.display()));
    let manifest = Manifest::load(&list).unwrap();
    assert_eq!(manifest.files.len(), 1);
    assert_eq!(manifest.bytes, 1);
}

#[test]
fn a_missing_file_refuses_to_report() {
    let root = scratch("missing");
    let gone = root.join("gone.rs");
    let list = write_manifest(&root, &manifest_text(&[gone]));
    let error = Manifest::load(&list).unwrap_err();
    assert!(error.contains("refusing to report"), "{error}");
    assert!(error.contains("gone.rs"), "{error}");
}

#[test]
fn a_relative_path_refuses_to_report() {
    let root = scratch("relative");
    let list = write_manifest(&root, "src/main.rs\n");
    let error = Manifest::load(&list).unwrap_err();
    assert!(error.contains("not an absolute path"), "{error}");
}

#[test]
fn a_directory_is_not_a_corpus_file() {
    let root = scratch("directory");
    let list = write_manifest(&root, &format!("{}\n", root.display()));
    let error = Manifest::load(&list).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
}

#[test]
fn an_empty_manifest_is_refused() {
    let root = scratch("empty");
    let list = write_manifest(&root, "# nothing here\n");
    let error = Manifest::load(&list).unwrap_err();
    assert!(error.contains("names no files"), "{error}");
}

#[test]
fn a_missing_manifest_is_an_error_not_a_panic() {
    let error = Manifest::load("/nonexistent/corpus.list").unwrap_err();
    assert!(error.contains("No such file"), "{error}");
}

#[test]
fn stats_are_exact_for_odd_and_even_counts() {
    let odd = stats(vec![5.0, 1.0, 3.0]);
    assert_eq!(odd, Stats { median: 3.0, mean: 3.0, min: 1.0, max: 5.0 });
    let even = stats(vec![4.0, 1.0, 3.0, 2.0]);
    assert_eq!(even, Stats { median: 2.5, mean: 2.5, min: 1.0, max: 4.0 });
}

#[test]
fn the_gate_is_exactly_two_times() {
    // A ratio at the margin passes; just under it does not.
    assert!(4.0 / 2.0 >= MIN_SPEEDUP);
    assert!(4.1 / 2.0 > MIN_SPEEDUP);
    assert!(3.99 / 2.0 < MIN_SPEEDUP);
}

#[test]
fn the_command_line_is_reproducible() {
    assert_eq!(
        shell_join(&["rust-graph-deep-bench".into(), "compare".into(), "--list".into(), "/tmp/a b.list".into()]),
        "rust-graph-deep-bench compare --list '/tmp/a b.list'"
    );
    assert_eq!(shell_quote("it's"), "'it'\\''s'");
    // An empty argument must stay visible.
    assert_eq!(shell_quote(""), "''");
}

#[test]
fn a_missing_tree_sitter_driver_is_a_refusal_with_a_reason() {
    let error = check_program("/nonexistent/tsdrv", "--tree-sitter").unwrap_err();
    assert!(error.contains("does not exist"), "{error}");
    assert!(error.contains("Tree-sitter is not installed"), "{error}");
    assert!(error.contains("toolchain:"), "{error}");
}

#[test]
fn a_non_executable_program_is_refused() {
    let root = scratch("not-exec");
    let file = root.join("driver");
    fs::write(&file, b"#!/bin/sh\n").unwrap();
    let error = check_program(file.to_str().unwrap(), "--deep").unwrap_err();
    assert!(error.contains("not executable"), "{error}");
}

#[test]
fn an_unknown_option_is_never_ignored() {
    let error = Options::parse(&["--lists".into(), "x".into()]).unwrap_err();
    assert!(error.contains("unknown option"), "{error}");
    assert!(Options::parse(&["--rounds".into(), "0".into()]).is_err());
    assert!(Options::parse(&["--rounds".into(), "x".into()]).is_err());
}

#[test]
fn vm_hwm_reads_this_process() {
    let own = vm_hwm(std::process::id()).expect("our own VmHWM");
    assert!(own > 0);
    assert!(vm_hwm(u32::MAX).is_none());
}

#[test]
fn the_environment_lines_are_present() {
    assert!(!os_name().is_empty());
    assert!(!host_name().is_empty());
    assert!(!cpu_model().is_empty());
}

/// The repository root, for the files this crate lives beside.
fn repository_root() -> PathBuf {
    // The tests run twice: from the rust-graph package (the binary target is
    // declared there) and from this crate (`cargo test -p deep-bench`), so the
    // root is whichever directory holds `crates/deep-bench`.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|directory| directory.join("crates/deep-bench").is_dir())
        .expect("the repository root holds crates/deep-bench")
        .to_path_buf()
}

/// The hash check that says the default build did not change: `pre` was the tree
/// before any Field 5 file existed, `post` is the same tree with the harness and
/// the bridge in place, `fresh` is a build in an empty target directory. All
/// three must be one number per profile. `default-build.sha256` records the
/// commands.
#[test]
fn the_default_build_is_byte_identical_to_the_one_before_this_field() {
    let path = repository_root().join("crates/deep-bench/default-build.sha256");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let mut seen: Vec<(String, String, String, u64)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 4, "bad line in default-build.sha256: {line:?}");
        seen.push((fields[0].into(), fields[1].into(), fields[2].into(), fields[3].parse().unwrap()));
    }
    let get = |label: &str, profile: &str| {
        seen.iter()
            .find(|(l, p, _, _)| l == label && p == profile)
            .unwrap_or_else(|| panic!("default-build.sha256 has no {label} {profile} row"))
            .clone()
    };
    for profile in ["release", "debug"] {
        let (before, after) = (get("pre", profile), get("post", profile));
        assert_eq!(
            before.2, after.2,
            "{profile}: the default build changed since Field 5 was added — rerun the documented commands and rewrite the file only if the change is intended"
        );
        assert_eq!(before.3, after.3, "{profile}: the default binary changed size");
    }
    assert_eq!(get("fresh", "release").2, get("post", "release").2, "the release build is not reproducible in an empty target directory");
    assert_eq!(seen.len(), 5, "default-build.sha256 grew a row nothing explains");
}

/// The hash check only holds while nothing in the default build names the
/// bridge, and while the bridge's one door to the runtime is the gate. Both are
/// checked as exact text, so a gate that moves or loses its `cfg` fails here
/// rather than in a binary nobody diffs.
#[test]
fn the_default_build_does_not_reference_the_bridge() {
    let root = repository_root();
    let main = fs::read_to_string(root.join("src/main.rs")).expect("src/main.rs");
    for (number, line) in main.lines().enumerate() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        assert!(
            !line.contains("deep_integration"),
            "src/main.rs:{} names deep_integration, so the default build would include the bridge",
            number + 1
        );
        assert!(!(line == "mod deep;" || line == "pub mod deep;"), "src/main.rs:{} declares the deep runtime unconditionally", number + 1);
    }
    let bridge = fs::read_to_string(root.join("src/deep_integration.rs")).expect("src/deep_integration.rs");
    for gated in ["#[cfg(feature = \"deep\")]\nuse crate::deep;", "#[cfg(feature = \"deep\")]\nmod imp {"] {
        assert!(bridge.contains(gated), "src/deep_integration.rs no longer carries {gated:?}, so the runtime is not behind the feature");
    }
    assert!(
        bridge.contains("#[cfg(not(feature = \"deep\"))]\nmod imp {"),
        "src/deep_integration.rs needs the feature-off half that returns None"
    );
}
