//! An infix call after a closed lambda block continues the expression above.
//!
//! `fn("isEmpty()") { … } builder { … }` is one expression: the CST folds the
//! `builder { … }` tail into it and writes no value, while the one-pass reader
//! opened a statement at the name and filed `x n:builder` — 8 rows in one
//! stdlib template file, plus `} order {` in a Dokka base class.
//!
//! What is pinned is that neither reader files a value for the tail, and that
//! a plain call statement on its own still gets one.

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

const TAIL: &str = "fun f() {\n    val x = fn(\"isEmpty()\") {\n        one()\n    } builder {\n        two()\n    }\n}\n";

#[test]
fn kotlin_infix_tail_after_a_block_states_nothing() {
    let (lexer, deep) = both(TAIL);
    // The tail is on line 4; the CST folds the name into the value it writes on
    // the line the expression starts (2), so the check is on the tail's line.
    for (side, events) in [("the lexer", &lexer), ("the cst", &deep)] {
        assert!(!events.contains("[\"x\",4,"), "{side} filed the tail as a value: {events}");
        assert!(!events.contains("[\"a\",4,"), "{side} bound the tail: {events}");
    }
}

const STANDALONE: &str = "fun g() {\n    run {\n        one()\n    }\n}\n";

#[test]
fn kotlin_standalone_call_still_states_its_value() {
    let (lexer, deep) = both(STANDALONE);
    assert!(lexer.contains("run"), "the call was lost: {lexer}");
    assert!(deep.contains("run"), "the cst lost the call too: {deep}");
}
