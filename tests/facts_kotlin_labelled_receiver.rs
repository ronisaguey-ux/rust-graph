//! A Kotlin labelled receiver names the scope, not a member.
//!
//! `this@asList.isEmpty()` is `self.isEmpty`: the label says which `this` is
//! meant. Left in the chain the label became part of the callee
//! (`asList.isEmpty`), which no reader of the CST names, and every member using
//! a labelled receiver lost its call (one row in the stdlib wasm builtins).

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

use facts::Facts;

fn events(source: &str) -> String {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let facts: Facts = parser::parse(source.as_bytes(), language);
    facts.symbols.iter().map(|symbol| symbol.events.as_str()).collect()
}

const LABELLED: &str = "class C {\n    fun isEmpty(): Boolean = this@C.isEmpty()\n}\n";

#[test]
fn a_labelled_receiver_calls_the_member_on_self() {
    let recorded = events(LABELLED);
    assert!(recorded.contains("\"self\""), "the receiver was lost: {recorded}");
    assert!(
        !recorded.contains("C.isEmpty"),
        "the label became part of the callee: {recorded}"
    );
}

const PLAIN: &str = "class C {\n    fun isEmpty(): Boolean = this.isEmpty()\n}\n";

#[test]
fn a_plain_receiver_still_calls_the_member_on_self() {
    let recorded = events(PLAIN);
    assert!(recorded.contains("\"self\""), "the receiver was lost: {recorded}");
}
