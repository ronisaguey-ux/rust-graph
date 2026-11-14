//! The Rust reader's coverage table.
//!
//! Every form the plan claims the Rust reader covers is one row here, and one
//! row is one assertion: both readers read the file and the guard finds nothing
//! to fall back for. A form is a shape, not a keyword — `Point { x: 1, y }` is
//! covered when the lexer and the CST agree about which symbol the fields belong
//! to, not when the grammar merely has a node for a struct literal.
//!
//! The fixtures are the smallest source that exercises the form, so a failure
//! names the form instead of a file.
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
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// One row per claimed form.
const COVERAGE: &[(&str, &str)] = &[
    (
        "struct literal: named fields, shorthand and spread",
        "fn f(p: Point) {\n    let p = Point { x: 1, y };\n    let q = Point { z: 2, ..p };\n}\n",
    ),
    (
        "struct literal as a call argument",
        "fn f() {\n    let r = build(Point { x: 4 });\n}\n",
    ),
    (
        "match arms: block body, guard, call value and wildcard",
        "fn g(v: i32) -> i32 { v }\nfn f(x: i32) -> i32 {\n    match x {\n        0 => { let z = 1; z }\n        1 if x > 0 => g(x),\n        _ => 2,\n    }\n}\n",
    ),
    (
        "closures: annotated return type with a block, and an expression body",
        "fn f() {\n    let add = |x: i32| -> i32 { x + 1 };\n    let twice = |v: i32| v * 2;\n}\n",
    ),
    (
        "trait bounds on a parameter and a where clause",
        "fn f<T: Clone + Send>(t: T) -> T where T: Send { t }\n",
    ),
    (
        "dynamic types: dyn in a reference and behind Box",
        "fn g(x: &dyn Fn() -> i32, y: Box<dyn Send>) {}\n",
    ),
    (
        "let declarations: mut, annotated and tuple-pattern bindings",
        "fn f() {\n    let mut c = 3;\n    let d: i32 = 4;\n    let (a, b) = (1, 2);\n    let e = c + d + a + b;\n}\n",
    ),
];

/// Forms the reader does not cover yet, with the exact disagreement.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];

#[test]
fn every_claimed_rust_form_reads_the_same_on_both_paths() {
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
    assert!(failed.is_empty(), "the Rust reader falls back on {} form(s):\n{}", failed.len(), failed.join("\n"));
}

/// A recorded gap is asserted to disagree, so closing one without moving the row
/// up fails the suite. An empty gap list asserts nothing, which is the honest
/// state of the forms above.
#[test]
fn every_known_rust_gap_still_fails() {
    for (name, source, why) in KNOWN_GAPS {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        let verdict = pipeline::loses(&lexer, &deep);
        assert!(verdict.lost(), "{name} reads cleanly now — move it up to COVERAGE ({why})");
    }
}
