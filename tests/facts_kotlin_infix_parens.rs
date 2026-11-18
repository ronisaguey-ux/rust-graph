//! Infix operator words before a parenthesised right operand are not calls.
//!
//! `returns() implies (condition)`, `1 shl (a + b)`, `value shr (2 * shift)`
//! and `(a shl 12) xor (b shl 6)` are infix expressions whose right operand
//! opens with `(`. The CST records `infix_expression` and no call for the
//! operator word, whereas the one-pass reader saw `<ident> (` and emitted a
//! spurious call. Meanwhile, `"include" to listOf("*.jar")` and
//! `if (cond) foo(x)` are real calls that must still be recorded.

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

fn calls(facts: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    for symbol in &facts.symbols {
        for call in &symbol.calls {
            out.push(call.path.clone());
        }
    }
    out
}

fn both_calls(source: &str) -> (Vec<String>, Vec<String>) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (calls(&lexer), calls(&deep))
}

const INFIX_PARENS: &str = r#"
fun check(condition: Boolean, value: Int, a: Int, b: Int) {
    returns() implies (condition)
    val x = 1 shl (a + b)
    val y = value shr (2 * a)
    val z = (a shl 12) xor (b shl 6)
}
"#;

#[test]
fn kotlin_infix_with_parenthesised_rhs_is_not_a_call() {
    let (lexer, deep) = both_calls(INFIX_PARENS);
    for op in ["implies", "shl", "shr", "xor"] {
        assert!(!lexer.iter().any(|c| c == op), "lexer emitted spurious call {op}: {lexer:?}");
        assert!(!deep.iter().any(|c| c == op), "cst emitted spurious call {op}: {deep:?}");
    }
    assert!(lexer.iter().any(|c| c == "returns"), "lexer lost returns(): {lexer:?}");
    assert!(deep.iter().any(|c| c == "returns"), "cst lost returns(): {deep:?}");
}

const REAL_CALLS_NEAR_INFIX: &str = r#"
fun pairs(cond: Boolean) {
    implementation(mapOf("include" to listOf("*.jar")))
    if (cond) foo(1) else bar(2)
}
"#;

#[test]
fn kotlin_calls_after_infix_or_if_condition_are_still_calls() {
    let (l_calls, d_calls) = both_calls(REAL_CALLS_NEAR_INFIX);
    for expected in ["implementation", "mapOf", "listOf", "foo", "bar"] {
        assert!(l_calls.iter().any(|c| c == expected), "lexer lost {expected}: {l_calls:?}");
        assert!(d_calls.iter().any(|c| c == expected), "cst lost {expected}: {d_calls:?}");
    }
}
