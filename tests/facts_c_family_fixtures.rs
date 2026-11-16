//! P6-12: one smoke file per C-family language, so a table edit cannot break a
//! language whose record no other test reads.
//!
//! `deep_facts_every_disagreement_is_explained` compares the CST record against
//! the lexer's, and that comparison holds only for Python, JavaScript, TypeScript
//! and Rust — the four languages `lang_for` answers for. The other nine grammars
//! are reachable only with `--deep`, their records are still lossy against the
//! lexer, and `auto` never selects them, so a corpus measurement of them is a
//! deliberate `lang_for` flip, not something the suite runs.
//!
//! That leaves their walk to no test at all: an arm added for one language that
//! also matches another's nodes changes these records silently — exactly the
//! failure that made main red for two hours on 2026-11-16, where an attribute
//! arm written for Swift also matched Python's nodes. The corpora did not move
//! that night; only the pinned fixtures caught it.
//!
//! So: every fixture in `crates/deep-grammars/fixtures/{c,cpp,csharp,go,java,
//! kotlin,objc,scala,swift}` is parsed and read here, under the walk's own rules,
//! with properties that hold whatever the reader's coverage is:
//!
//! 1. a valid fixture parses, does not come back degraded, and keeps the module
//!    symbol every record starts with;
//! 2. a file in its `broken/` or `malformed/` folder still parses — recovery is
//!    a result, not a failure — and reading it panics on nothing (P6-17's
//!    truncated headers, at fixture scale);
//! 3. reading any fixture twice gives the same records, so no arm carries state
//!    from one file into the next;
//! 4. no record names an identifier the source does not contain — a walk that
//!    invents a call is wrong in every language, not just the one it was written
//!    for.
//!
//! The counts are pinned, so a language that loses its fixtures cannot pass by
//! having nothing left to check.
#![cfg(all(feature = "deep", feature = "parser"))]

#[path = "../src/facts.rs"]
mod facts;
#[path = "../src/json.rs"]
mod json;
#[path = "../src/langs.rs"]
mod langs;
#[path = "../src/lexer.rs"]
mod lexer;
#[path = "../src/expr.rs"]
mod expr;
#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/index.rs"]
mod index;
#[path = "../src/resolve.rs"]
mod resolve;
#[path = "../src/sha256.rs"]
mod sha256;
#[path = "../src/deep/mod.rs"]
mod deep;
#[path = "../src/deep_facts/mod.rs"]
mod deep_facts;
#[path = "../src/pipeline.rs"]
mod pipeline;

use pipeline::gate::*;

use deep_integration::Lang;
use facts::Facts;
use std::path::{Path, PathBuf};

/// Folder under `crates/deep-grammars/fixtures`, its language, and the number of
/// valid and broken fixtures it holds.
const FAMILY: &[(&str, Lang, usize, usize)] = &[
    ("c", Lang::C, 8, 3),
    ("cpp", Lang::Cpp, 8, 3),
    ("csharp", Lang::CSharp, 8, 3),
    ("go", Lang::Go, 8, 3),
    ("java", Lang::Java, 8, 3),
    ("kotlin", Lang::Kotlin, 8, 3),
    ("objc", Lang::ObjC, 9, 3),
    ("scala", Lang::Scala, 8, 3),
    ("swift", Lang::Swift, 8, 3),
];

/// Regular files in `dir`, sorted. A missing directory is empty, not an error:
/// a language names its recovery folder either `broken/` or `malformed/`.
fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    out.sort();
    out
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/deep-grammars/fixtures")
}

/// Everything a table edit can move, in a form that compares and prints.
fn signature(facts: &Facts) -> String {
    let mut out = String::new();
    for symbol in &facts.symbols {
        out.push_str(&format!("S {} {} {}\n", symbol.key, symbol.name, symbol.kind));
        for call in &symbol.calls {
            out.push_str(&format!("C {} {}\n", symbol.key, call.path));
        }
        for base in &symbol.bases {
            out.push_str(&format!("B {} {}\n", symbol.key, base));
        }
    }
    for import in &facts.imports {
        out.push_str(&format!("I {} {} {} {}\n", import.module, import.name, import.alias, import.kind));
    }
    out
}

#[test]
fn every_c_family_fixture_parses_and_reads_the_same_twice() {
    let mut failures: Vec<String> = Vec::new();
    for (dir, lang, valid, broken) in FAMILY {
        let root = fixtures_root().join(dir);
        let paths = files(&root);
        if paths.len() != *valid {
            failures.push(format!("{dir}: {} fixtures, the test pins {valid}", paths.len()));
        }
        let recovery: Vec<PathBuf> = ["broken", "malformed"]
            .iter()
            .flat_map(|name| files(&root.join(name)))
            .collect();
        if recovery.len() != *broken {
            failures.push(format!("{dir}: {} recovery fixtures, the test pins {broken}", recovery.len()));
        }
        // A valid fixture is the reader's claim about well-formed input: it must
        // parse, must not be mostly error nodes, and must own a module symbol.
        for path in paths {
            let name = format!("{dir}/{}", path.file_name().and_then(|name| name.to_str()).unwrap_or("?"));
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(format!("{name}: {error}"));
                    continue;
                }
            };
            let Some(cst) = deep_integration::parse_file(&bytes, *lang) else {
                failures.push(format!("{name}: no deep parse for {lang:?}"));
                continue;
            };
            if cst.degraded() {
                failures.push(format!("{name}: {} errors on {} nodes", cst.errors, cst.nodes.len()));
            }
            let first = signature(&deep_facts::extract(&bytes, &cst, *lang));
            let second = signature(&deep_facts::extract(&bytes, &cst, *lang));
            if first != second {
                failures.push(format!("{name}: two reads disagree"));
            }
            if !first.contains("S <module> ") {
                failures.push(format!("{name}: no module symbol"));
            }
        }
        // A truncated or half-written file is the C family's normal input. The
        // walk may recover badly, but it may not refuse the file or panic on it.
        for path in recovery {
            let name = format!("{dir}/{}", path.to_string_lossy().rsplit('/').next().unwrap_or("?"));
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(format!("{name}: {error}"));
                    continue;
                }
            };
            let Some(cst) = deep_integration::parse_file(&bytes, *lang) else {
                failures.push(format!("{name}: no deep parse for {lang:?}"));
                continue;
            };
            let first = signature(&deep_facts::extract(&bytes, &cst, *lang));
            let second = signature(&deep_facts::extract(&bytes, &cst, *lang));
            if first != second {
                failures.push(format!("{name}: two reads disagree"));
            }
        }
    }
    assert!(failures.is_empty(), "c-family fixtures:\n{}", failures.join("\n"));
}

/// The last segment of a call path is the name the source writes; a member call
/// keeps its receiver, so the last segment is the checkable part.
fn last_segment(path: &str) -> &str {
    path.rsplit(['.', ':']).next().unwrap_or(path)
}

/// The module symbol has no name, and a recovered node may carry a placeholder,
/// so only identifiers are checked.
fn checkable(name: &str) -> bool {
    name.chars().next().is_some_and(|first| first.is_alphabetic() || first == '_')
}

#[test]
fn no_c_family_fact_names_a_thing_the_source_does_not_contain() {
    let mut failures: Vec<String> = Vec::new();
    for (dir, lang, _, _) in FAMILY {
        for path in files(&fixtures_root().join(dir)) {
            let name = format!("{dir}/{}", path.file_name().and_then(|name| name.to_str()).unwrap_or("?"));
            let bytes = std::fs::read(&path).expect("fixture bytes");
            let source = String::from_utf8_lossy(&bytes);
            let Some(cst) = deep_integration::parse_file(&bytes, *lang) else { continue };
            let facts = deep_facts::extract(&bytes, &cst, *lang);
            let mut invented: Vec<String> = Vec::new();
            for symbol in &facts.symbols {
                if checkable(&symbol.name) && !source.contains(&symbol.name) {
                    invented.push(format!("symbol {}", symbol.name));
                }
                for call in &symbol.calls {
                    let segment = last_segment(&call.path);
                    if checkable(segment) && !source.contains(segment) {
                        invented.push(format!("call {} ({})", call.path, symbol.key));
                    }
                }
            }
            if !invented.is_empty() {
                failures.push(format!("{name}: {}", invented.join(", ")));
            }
        }
    }
    assert!(failures.is_empty(), "c-family fixtures invent names:\n{}", failures.join("\n"));
}
