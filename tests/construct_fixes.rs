//! P5-08: a construct two languages share is fixed in the code they share.
//!
//! `src/pipeline.rs` gives every fallback a machine code, and the code is the
//! same for the same disagreement whichever language hit it. The list of codes
//! that are live, and the module that decides each one, is recorded in
//! `tools/construct_fixes.json` beside `tools/baselines/constructs.json`. This
//! reads both and fails when a live code is unowned, when an owned code names a
//! module that only one language reads, or when the module it names is gone.
//!
//! The measurement itself is `tools/construct_report.py --measure`; a test that
//! swept four corpora would be minutes of work for an assertion about a table.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The modules both readers of every language go through. A shared construct
/// fixed anywhere else is a second fix waiting to disagree with the first.
const SHARED: &[&str] = &[
    "src/parser.rs",
    "src/expr.rs",
    "src/pipeline.rs",
    "src/deep_integration.rs",
    "src/deep_facts/mod.rs",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(name: &str) -> String {
    std::fs::read_to_string(root().join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// Every `<code>: {<module>, <why>}` pair in `tools/construct_fixes.json`.
///
/// The file is a flat object of two-string records and the test has no JSON
/// dependency, so it is read by hand: a line ending in `: {` opens a code, and
/// the `module` line inside it names the module.
fn fixes() -> BTreeMap<String, String> {
    let text = read("tools/construct_fixes.json");
    let mut out = BTreeMap::new();
    let mut code = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('"') && line.ends_with(": {") {
            code = line.trim_end_matches(": {").trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("\"module\":") {
            let module = rest.trim().trim_end_matches(',').trim_matches('"').to_string();
            out.insert(std::mem::take(&mut code), module);
        }
    }
    out
}

/// `code -> rust count` for every code the Rust corpus still reports, plus the
/// other languages that report it, from `tools/baselines/constructs.json`.
fn codes() -> BTreeMap<String, (usize, Vec<String>)> {
    let text = read("tools/baselines/constructs.json");
    let mut out: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('"') && line.ends_with(": {") {
            current = Some(line.trim_end_matches(": {").trim_matches('"').to_string());
            continue;
        }
        let Some(code) = current.clone() else {
            continue;
        };
        // A count line is `"lang": N,` — read the four languages and ignore the
        // `samples` array, which is nested and has no counts in it.
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        let Ok(count) = value.trim().trim_end_matches(',').parse::<usize>() else {
            continue;
        };
        let slot = out.entry(code).or_insert((0, Vec::new()));
        if key == "rust" {
            slot.0 = count;
        } else if matches!(key, "js" | "ts" | "python") && count > 0 {
            slot.1.push(key.to_string());
        }
    }
    out.retain(|_, (rust, _)| *rust > 0);
    out
}

#[test]
fn every_live_construct_is_owned_by_the_module_that_decides_it() {
    let fixes = fixes();
    let live = codes();
    assert!(!live.is_empty(), "the construct record read as empty");
    let mut problems = Vec::new();
    for (code, (rust, others)) in &live {
        let Some(module) = fixes.get(code) else {
            problems.push(format!("{code}: {rust} rust file(s) and no entry in tools/construct_fixes.json"));
            continue;
        };
        if !root().join(module).is_file() {
            problems.push(format!("{code}: recorded as fixed in {module}, which does not exist"));
        }
        if !others.is_empty() && !SHARED.contains(&module.as_str()) {
            problems.push(format!(
                "{code}: shared with {} but fixed in {module}, which only one language reads",
                others.join(", ")
            ));
        }
    }
    assert!(problems.is_empty(), "the construct table is not honest:\n{}", problems.join("\n"));
}

#[test]
fn an_owned_construct_names_a_module_that_exists() {
    // A code that no language reports any more is a finish line, not a claim, so
    // it may stay in the table; the module it names must still be a real file,
    // otherwise the entry has quietly rotted into a note about nothing.
    for (code, module) in &fixes() {
        assert!(
            root().join(module).is_file(),
            "{code}: recorded as fixed in {module}, which does not exist"
        );
    }
}
