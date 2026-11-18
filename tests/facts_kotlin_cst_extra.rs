//! Regression tests for Kotlin cst-extra event mismatches fixed in lane 8.

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

#[test]
fn kotlin_safe_call_and_multiline_expr_body_and_is_expr_agree() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "safe-call continuation",
            "fun a(x: String?): Int? = x\n    ?.length\n",
            "[\"x\",2,",
        ),
        (
            "leading = expression body",
            "fun b(x: Int)\n    = x + 1\n",
            "[\"x\",2,",
        ),
        (
            "colon newline before return type",
            "fun c(x: Int):\n    Int =\n    x + 1\n",
            "[\"x\",2,",
        ),
        (
            "leading + after }",
            "fun d(x: Int): String = when (x) {\n    1 -> \"a\"\n    else -> \"b\"\n} + \"c\"\n",
            "[\"x\",4,",
        ),
        (
            "leading as after }",
            "fun e(x: Any): String = when (x) {\n    1 -> a\n    else -> b\n} as String\n",
            "[\"x\",4,",
        ),
    ];

    for (label, source, forbidden) in cases {
        let (lexer, deep) = both(source);
        assert!(!lexer.contains(forbidden), "{label}: lexer emitted {forbidden} in {lexer}");
        assert!(!deep.contains(forbidden), "{label}: cst emitted {forbidden} in {deep}");
    }

    let (lexer, deep) = both("fun f(xs: List<Any>) = xs.filter {\n    it is String\n}\n");
    assert!(lexer.contains("[\"x\",2,") && deep.contains("[\"x\",2,"), "is_expression should emit x on line 2: lexer={lexer}, deep={deep}");
}
