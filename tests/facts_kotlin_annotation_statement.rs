//! A Kotlin statement that is only annotations states no value.
//!
//! `@Suppress("EXTENSION_SHADOWED_BY_MEMBER")` on its own line is an annotation
//! with arguments. The bare form already wrote nothing; with the parentheses and
//! the string left out of the matcher the statement fell through to the
//! expression reader, which filed the annotation as a CALL named `Suppress` the
//! CST never has (stdlib `Maps.kt`, `Comparisons.kt`).

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

const ANNOTATED: &str = "@Suppress(\"EXTENSION_SHADOWED_BY_MEMBER\")\n@kotlin.internal.InlineOnly\nfun f(x: Int): Int = x\n";

#[test]
fn an_annotation_with_arguments_is_not_a_call() {
    let recorded = events(ANNOTATED);
    assert!(
        !recorded.contains("Suppress"),
        "the annotation became a call: {recorded}"
    );
    assert!(
        recorded.contains("[\"r\""),
        "the declaration's value was lost: {recorded}"
    );
}

const BARE: &str = "@Test\nfun f(x: Int): Int = x\n";

#[test]
fn a_bare_annotation_is_not_a_call_either() {
    let recorded = events(BARE);
    assert!(!recorded.contains("Test"), "the annotation became a value: {recorded}");
}
