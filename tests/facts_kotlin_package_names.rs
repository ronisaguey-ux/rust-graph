//! Words from other languages' keyword lists are ordinary Kotlin names.
//!
//! The shared noise list carries Perl's `my`/`local`/`our` and JavaScript's
//! `let`. Kotlin uses `my` as a package segment and `let` as a stdlib call, so
//! skipping them read `package my.qa.noname` as `qa.noname` — 14 files of the
//! kotlin dump.
//!
//! What is pinned is the package's full dotted name.

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

fn lexer_events(source: &str) -> String {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    events(&parser::parse(source.as_bytes(), language))
}

fn deep_events(source: &str) -> String {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    events(&deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin))
}

#[test]
fn kotlin_package_name_keeps_its_first_segment() {
    let source = "package my.qa.noname\n\nclass A\n";
    let lexer = lexer_events(source);
    let deep = deep_events(source);
    assert!(lexer.contains("[\"n\",\"my.qa.noname\"]"), "the package lost its first segment: {lexer}");
    assert!(deep.contains("[\"n\",\"my.qa.noname\"]"), "the cst read another package: {deep}");
}
