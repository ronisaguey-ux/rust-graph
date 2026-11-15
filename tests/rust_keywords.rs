//! P5-15: the guard carries no rust-only excuse for a keyword name.
//!
//! `covers_value` used to keep a per-language keyword list and, for Rust, set a
//! name aside when no variable could take it. That list was the rust-only
//! tolerance: it existed because the lexer's value used to carry `loop`,
//! `match`, `while`, `try` and `move` as names. The value merge is fixed, so the
//! excuse is removed — and these rows are the proof it is gone rather than
//! merely unused: each statement is compared with the guard that no longer has
//! it, and any keyword that came back would be reported as a lost name.
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

use deep_integration::Lang;
use facts::{Facts, Symbol};
use pipeline::gate::*;

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// The words the guard used to excuse for Rust, and nothing else.
const EXCUSED: &[&str] = &["fn", "if", "for", "while", "return", "match", "loop", "let", "unsafe"];

/// `(what the statement is, source)`.
const ROWS: &[(&str, &str)] = &[
    (
        "a `loop` as an assignment's value",
        "fn f() {\n    let x = loop {\n        break 1;\n    };\n}\n",
    ),
    (
        "a `match` returning from a function",
        "fn g(v: u8) -> u8 {\n    match v {\n        1 => 2,\n        _ => 0,\n    }\n}\n",
    ),
    (
        "a `while` loop with a mutating body",
        "fn h(mut i: u8) {\n    while i > 0 {\n        i -= 1;\n    }\n}\n",
    ),
    (
        "an `unsafe` block opened for one call",
        "fn k() {\n    let v = unsafe { read_volatile(&RAW) };\n}\n",
    ),
    (
        "a `move` closure assigned to a binding",
        "fn m() {\n    let f = move || compute();\n}\n",
    ),
    (
        "a `for` loop over a range",
        "fn n() {\n    for i in 0..10 {\n        use_it(i);\n    }\n}\n",
    ),
];

/// Every name atom a symbol's values carry.
///
/// The events are one JSON array per symbol and a name is written `["n","x"]`,
/// so the atoms are read straight out of the text rather than re-parsed.
fn names(symbol: &Symbol) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = symbol.events.as_str();
    while let Some(at) = rest.find("[\"n\",\"") {
        rest = &rest[at + 7..];
        let Some(end) = rest.find('"') else { break };
        out.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    out
}

#[test]
fn no_rust_statement_needs_a_keyword_name_excused() {
    let mut problems = Vec::new();
    for (what, source) in ROWS {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        let verdict = pipeline::loses_in(&lexer, &deep, source);
        if verdict.lost() {
            problems.push(format!("{what}: {verdict:?}"));
        }
        for symbol in lexer.symbols.iter().chain(deep.symbols.iter()) {
            for name in names(symbol) {
                let leaf = name.rsplit('.').next().unwrap_or(&name);
                if EXCUSED.contains(&leaf) {
                    problems.push(format!("{what}: `{leaf}` was recorded as a value name"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "a statement still relies on the keyword excuse:\n{}", problems.join("\n"));
}
