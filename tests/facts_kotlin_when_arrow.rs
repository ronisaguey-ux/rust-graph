//! A Kotlin `when` arrow `->` separates the branch condition from its body.
//!
//! `'/' -> append("\\|")` has condition `'/'` and body `append("\\|")`.
//! Merging the condition and body across `->` in `expr.rs` produced
//! `m:[0,c(0;[0])]` where the CST's branch statement emits `c(0;[0])`.

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

const SRC: &str = r#"package p
fun String.f(): String =
    if (a()) b()
    else buildString(length) {
        for (ch in this@f) {
            when (ch) {
                '/' -> append("\\|")
                '.' -> append("\\,")
                else -> append(ch)
            }
        }
    }
"#;

#[test]
fn kotlin_when_arrow_does_not_merge_condition_with_body() {
    let language = langs::by_content("kt", SRC.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(SRC.as_bytes(), language);
    for symbol in &lexer.symbols {
        assert!(
            !symbol.events.contains(r#"["m",[0,["c","#),
            "when arrow merged condition constant with body call: {}",
            symbol.events
        );
    }
    let module = lexer
        .symbols
        .iter()
        .find(|s| s.name.is_empty())
        .expect("module symbol");
    assert!(
        module.events.contains(r#"["x",7,["c","#) && module.events.contains(r#"["x",8,["c","#),
        "expected call events for when branch bodies: {}",
        module.events
    );
}
