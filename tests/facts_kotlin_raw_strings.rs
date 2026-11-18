//! Kotlin raw strings span lines, and nothing inside them is code.
//!
//! `const val CSS = """…"""` read as a single-line string ended at the first
//! newline, so the rest of the block was read as statements: `th {` became a
//! name and `border: 1px solid black;` a map, per line, for every file holding
//! a stylesheet or a SQL query in a raw string. 183 files of the kotlin dump
//! were down to this.
//!
//! What is pinned is that no reader files an event on a line inside the raw
//! string, and that the declaration holding it still gets its own.

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

fn both(source: &str) -> (String, String) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (events(&lexer), events(&deep))
}

const SHEET: &str = "const val CSS = \"\"\"\ntable {\n  border-collapse: collapse;\n}\nth {\n  background: #B0B0B0;\n}\ntd.location {\n  vertical-align: top;\n}\n\"\"\"\n\nfun after(): Int = 1\n";

#[test]
fn kotlin_raw_string_contents_are_not_code() {
    let (lexer, deep) = both(SHEET);
    let mut failures: Vec<String> = Vec::new();
    for line in 2..=9u32 {
        for marker in ["x", "a", "m", "c"] {
            let needle = format!("[\"{marker}\",{line},");
            if lexer.contains(&needle) {
                failures.push(format!("line {line}: the lexer filed {marker}: {lexer}"));
            }
            if deep.contains(&needle) {
                failures.push(format!("line {line}: the cst filed {marker}: {deep}"));
            }
        }
    }
    // The statement after the string is still read by both.
    assert!(lexer.contains("[\"r\",13,0]"), "the statement after the string was lost: {lexer}");
    assert!(deep.contains("[\"r\",13,0]"), "the statement after the string was lost: {deep}");
    assert!(failures.is_empty(), "kotlin raw strings are read as code:\n{}", failures.join("\n"));
}
