//! The Python reader's coverage table.
//!
//! The mirror of `tests/ts_coverage.rs`: every construct the plan claims the
//! Python reader covers is one row, and one row is one assertion — both readers
//! read the smallest source that exercises the construct and the guard finds
//! nothing to fall back for.
//!
//! A construct that does not read cleanly yet is *asserted to fail*, not
//! skipped: `KNOWN_GAPS` names the exact disagreement, so closing one without
//! moving the row up fails the suite.
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

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("py", source.as_bytes()).expect("a Python row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Python).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Python)
}

/// One row per construct that reads cleanly today.
const COVERAGE: &[(&str, &str)] = &[
    (
        "match statement: subject and literal, sequence and mapping arms",
        "def f(x):\n    match x:\n        case 1:\n            return 'one'\n        case [a, b]:\n            return a\n        case {'k': v}:\n            return v\n    return None\n",
    ),
    (
        "match arm with a guard and a class pattern",
        "def f(x):\n    match x:\n        case Point(x=0, y=0):\n            return 'origin'\n        case Point(x=a) if a > 0:\n            return a\n    return None\n",
    ),
];

/// Constructs the reader does not cover yet, with the exact disagreement.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];

#[test]
fn every_claimed_python_construct_reads_the_same_on_both_paths() {
    let mut failed: Vec<String> = Vec::new();
    for (name, source) in COVERAGE {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        assert!(!lexer.symbols.is_empty(), "{name}: the lexer read no symbol");
        let verdict = pipeline::loses(&lexer, &deep);
        if verdict.lost() {
            failed.push(format!("{name}: {verdict:?}"));
        }
    }
    assert!(
        failed.is_empty(),
        "the Python reader falls back on {} construct(s):\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn the_known_python_gaps_are_still_gaps() {
    let mut closed: Vec<&str> = Vec::new();
    for (name, source, _) in KNOWN_GAPS {
        let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
        if !verdict.lost() {
            closed.push(name);
        }
    }
    assert!(closed.is_empty(), "these rows now read cleanly, move them to COVERAGE: {closed:?}");
}
