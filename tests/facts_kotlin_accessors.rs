//! Kotlin accessors: a property getter is an assignment to the lexer.
//!
//! `override val size: Int` followed by `get() = keys.length` is read by the
//! one-pass reader as an `x` event on the getter's line, holding the body's
//! value. The walk wrote the body as the function's `r` value instead, so the
//! getter found no partner whenever the property stood before another member
//! (`get() = keys.length` in a class with more members lost its value; 24 rows
//! of the kotlin dump).
//!
//! What is pinned is that the getter's line carries its value, and that a
//! function's expression body still reads as the `r` value it did.

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

fn events(facts: &Facts) -> String {
    let mut out = String::new();
    for symbol in &facts.symbols {
        out.push_str(&symbol.events);
    }
    out
}

/// The lexer's record and the CST record for one snippet.
fn both(source: &str) -> (String, String) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (events(&lexer), events(&deep))
}

const ROWS: &[(&str, &str, u32)] = &[
    (
        "a getter before another member",
        "class A {\n    override val size: Int\n        get() = keys.length\n\n    fun other(): Int = 1\n}\n",
        3,
    ),
    (
        "a getter in a class that ends there",
        "class A {\n    override val size: Int\n        get() = keys.length\n}\n",
        3,
    ),
    (
        "a getter over a call",
        "class A {\n    val port: Int\n        get() = readPort()\n\n    fun other() {}\n}\n",
        3,
    ),
    (
        "a top-level extension property's getter",
        "public actual val Char.category: CharCategory\n    get() = CharCategory.valueOf(Character.getType(this))\n\nfun other() {}\n",
        2,
    ),
];

#[test]
fn kotlin_getter_files_its_value_on_its_own_line() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source, line) in ROWS {
        let (lexer, deep) = both(source);
        let want = format!("[\"x\",{line},");
        if !deep.contains(&want) {
            failures.push(format!("{label}: the cst has no `x` event on line {line}: {deep}"));
        }
        if !lexer.contains(&want) {
            failures.push(format!("{label}: the lexer has no `x` event on line {line}: {lexer}"));
        }
    }
    assert!(failures.is_empty(), "kotlin accessors disagree:\n{}", failures.join("\n"));
}
