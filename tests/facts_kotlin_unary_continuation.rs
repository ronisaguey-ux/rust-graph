//! A Kotlin line that opens with an operator continues the expression above.
//!
//! `-jvmMain` newline `-nativeMain { … }` in a kts block is one binary
//! expression: the CST stamps the name on the line it is written (4) and writes
//! no call, while a reader that ended the statement at the newline opened a
//! statement at the name and filed `c(0;[0])` for its brace — 20 rows in the
//! kotlin corpus, every one of them a `-name {` line in a kts block.
//!
//! What is pinned is that neither reader files a call for that name, and that a
//! name on its own line still states its own value.

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
    let language = langs::by_content("kts", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (events(&lexer), events(&deep))
}

const CONTINUED: &str = "commonMain {\n    -concurrentMain {\n        -jvmMain\n        -nativeMain {\n            println(1)\n        }\n    }\n}\n";

#[test]
fn kotlin_operator_opens_a_continuation_line() {
    let (lexer, deep) = both(CONTINUED);
    for (side, events) in [("the lexer", &lexer), ("the cst", &deep)] {
        assert!(
            !events.contains("[\"x\",4,"),
            "{side} started a statement on the continuation line: {events}"
        );
        assert!(
            events.contains("nativeMain"),
            "{side} lost the name: {events}"
        );
    }
    assert!(
        lexer.contains("jvmMain"),
        "the lexer lost the first name: {lexer}"
    );
}

const STANDALONE: &str = "fun g() {\n    -x\n}\n";

#[test]
fn kotlin_unary_statement_still_states_its_name() {
    let (lexer, deep) = both(STANDALONE);
    assert!(lexer.contains("x"), "the name was lost: {lexer}");
    assert!(deep.contains("x"), "the cst lost the name too: {deep}");
}

const AFTER_BRACE: &str = "commonMain {\n    -appleMain {\n        -macosMain\n    }\n    -linuxMain {\n        -linuxArm64Main\n    }\n}\n";

#[test]
fn kotlin_unary_after_closing_brace_starts_a_new_statement() {
    let (lexer, deep) = both(AFTER_BRACE);
    for (side, events) in [("the lexer", &lexer), ("the cst", &deep)] {
        assert!(
            events.contains("[\"x\",5,") && events.contains("linuxMain"),
            "{side} did not start a new statement on line 5: {events}"
        );
    }
}
