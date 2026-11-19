//! A Kotlin lambda arrow `->` is not a member-access operator.
//!
//! `{ t -> if (t == null) 0 else odd(t) }` separates parameter `t` from the
//! body with `->`. Treating `->` as a chain separator in `expr.rs` joined `t`
//! and `if` into a `t.if(...)` call (`c(n:t;...)`).

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

const SRC: &str = r#"
class C {
    val even: F<T?, Int> = F { t ->
        if (t == null) 0 else odd(t)
    }
}
"#;

#[test]
fn kotlin_lambda_arrow_does_not_join_parameter_to_body_call() {
    let language = langs::by_content("kt", SRC.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(SRC.as_bytes(), language);
    let calls: Vec<&str> = lexer
        .symbols
        .iter()
        .flat_map(|s| s.calls.iter().map(|c| c.path.as_str()))
        .collect();
    assert!(!calls.contains(&"t.if"), "lambda arrow joined t and if: {calls:?}");
    assert!(calls.contains(&"odd"), "expected odd call: {calls:?}");
}
