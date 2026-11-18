//! A Kotlin block comment nests, and an enum entry body is a type body.
//!
//! `/** … /* … */ … */` closes at the last `*/`. Stopping at the first one
//! handed the rest of the KDoc to the statement reader, so the code fence
//! (`public fun uiTest() { /* … */ }`) and every line after it were read as
//! code and the annotation following the KDoc carried the fence's line.
//!
//! `enum class Kind { A { … } }`: an entry's body holds members, and the entry
//! itself (`A {`) states no value. Read as a statement it wrote `x n:A`, which
//! the CST never writes.
//!
//! What is pinned is that the annotation is filed on its own line and that no
//! entry line carries an event.

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

const FENCE: &str = "/**\n * Usage:\n * ```\n * class A {\n *     public fun uiTest() { /* ... */ }\n * }\n * ```\n */\n@Target(AnnotationTarget.FUNCTION)\nclass B\n";

#[test]
fn kotlin_block_comment_closes_at_its_last_marker() {
    let (lexer, deep) = both(FENCE);
    // The fence sits inside the comment: lines 4 to 7 state nothing.
    for line in 4..=7u32 {
        for marker in ["x", "a", "c"] {
            assert!(
                !lexer.contains(&format!("[\"{marker}\",{line},")),
                "the fence line {line} was read as code: {lexer}"
            );
        }
    }
    // The annotation is a fact on its own line, 9.
    assert!(lexer.contains("[\"x\",9,") || lexer.contains("[\"a\",9,"), "the annotation lost its line: {lexer}");
    assert!(!lexer.contains("[\"x\",7,"), "the annotation carried the fence's line: {lexer}");
    assert!(!deep.is_empty(), "the cst read nothing at all");
}

const ENTRIES: &str = "enum class Kind {\n    A {\n        override fun value(): Int = 1\n    },\n    B {\n        override fun value(): Int = 2\n    }\n}\n";

#[test]
fn kotlin_enum_entry_body_states_nothing_of_its_own() {
    let (lexer, deep) = both(ENTRIES);
    for line in [2u32, 5] {
        assert!(
            !lexer.contains(&format!("[\"x\",{line},")) && !lexer.contains(&format!("[\"a\",{line},")),
            "the entry on line {line} was read as a value: {lexer}"
        );
        assert!(
            !deep.contains(&format!("[\"x\",{line},")) && !deep.contains(&format!("[\"a\",{line},")),
            "the cst filed the entry on line {line}: {deep}"
        );
    }
}
