//! Kotlin secondary-constructor delegation: `: this(...)` is a call to `self`.
//!
//! `constructor(x: Int) : this(x, 0)` delegates to another constructor of the
//! same class. The one-pass reader records `self` on the delegation's line,
//! whereas the CST walk had no arm for `constructor_delegation`, so every
//! secondary constructor delegating with `: this(...)` lost its `self` call
//! (and any calls inside its argument list).

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

fn has_call(facts: &Facts, target: &str, line: u32) -> bool {
    facts
        .symbols
        .iter()
        .flat_map(|symbol| symbol.calls.iter())
        .any(|call| call.path == target && call.line == line)
}

const ROWS: &[(&str, &str, &str, u32)] = &[
    (
        "single-line secondary constructor delegation",
        "class A(val x: Int, val y: Int) {\n    constructor(x: Int) : this(x, 0)\n}\n",
        "self",
        2,
    ),
    (
        "multi-line secondary constructor delegation",
        "class A(val x: Int, val y: Int) {\n    constructor(\n        x: Int,\n    ) : this(x, 0)\n}\n",
        "self",
        4,
    ),
    (
        "call inside delegation arguments",
        "class A(val x: Int, val y: Int) {\n    constructor(x: Int) : this(x, defaultY())\n}\n",
        "defaultY",
        2,
    ),
];

#[test]
fn kotlin_constructor_delegation_records_self_and_argument_calls() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source, target, line) in ROWS {
        let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
        let lexer = parser::parse(source.as_bytes(), language);
        let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
        let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
        if !has_call(&lexer, target, *line) {
            failures.push(format!("{label}: lexer missing call {target} at line {line}"));
        }
        if !has_call(&deep, target, *line) {
            failures.push(format!("{label}: deep missing call {target} at line {line}"));
        }
    }
    assert!(failures.is_empty(), "kotlin constructor delegation disagrees:\n{}", failures.join("\n"));
}
