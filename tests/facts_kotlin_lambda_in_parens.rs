//! Kotlin lambdas nested inside `(...)` still end statements on newlines.
//!
//! Inside `expectAssertion({ msg -> msg!!\n assertTrue(...) })`, the outer `(`
//! must not suppress `LINE_END` inside `{ ... }`, otherwise `msg!!` and
//! `assertTrue(...)` merge into an `infix_expression` and lose the call.

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

const SRC: &str = r#"
fun assertNullMessage() {
    expectAssertion({ msg ->
        msg!!
        assertTrue(msg.contains("x"))
    }, { assertNull(null, "x") })
}
"#;

#[test]
fn kotlin_lambda_inside_parens_ends_statements_on_newlines() {
    let language = langs::by_content("kt", SRC.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(SRC.as_bytes(), language);
    let cst = deep_integration::parse_file(SRC.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(SRC.as_bytes(), &cst, Lang::Kotlin);
    assert!(
        lexer.symbols.iter().flat_map(|s| &s.calls).any(|c| c.path == "assertTrue" && c.line == 5),
        "lexer should see assertTrue on line 5: {:?}",
        lexer.symbols.iter().flat_map(|s| &s.calls).collect::<Vec<_>>()
    );
    assert!(
        deep.symbols.iter().flat_map(|s| &s.calls).any(|c| c.path == "assertTrue" && c.line == 5),
        "deep should see assertTrue on line 5: {:?}",
        deep.symbols.iter().flat_map(|s| &s.calls).collect::<Vec<_>>()
    );
}
