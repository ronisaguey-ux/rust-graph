//! Kotlin generic invocation heads `f<T>(...)`: both readers must agree on
//! the call and skip type arguments instead of recording them as value names.

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

fn call_list(facts: &Facts) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for symbol in &facts.symbols {
        for call in &symbol.calls {
            out.push((call.path.clone(), call.line));
        }
    }
    out.sort();
    out
}

fn both(source: &str) -> (Vec<(String, u32)>, Vec<(String, u32)>) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (call_list(&lexer), call_list(&deep))
}

const ROWS: &[(&str, &str)] = &[
    (
        "script generic call with chained lambda",
        "tasks.withType<KotlinJvmCompile>().configureEach {\n    enabled = false\n}\n",
    ),
    (
        "script generic call with chained property",
        "the<org.example.Spec>().version\n",
    ),
    (
        "when entry generic call condition",
        "fun f(t: Any): Int = when (t) {\n    typeInfo<Map<String, Any?>>() -> 1\n    else -> 0\n}\n",
    ),
    (
        "generic call with variance projection",
        "fun f() {\n    extrasTypeOf<Box<out Int>>().signature\n}\n",
    ),
    (
        "getter generic call with chained method",
        "val p: Int\n    get() = variantImplementationFactory<Factory>().getInstance()\n",
    ),
];

#[test]
fn kotlin_generic_invocation_heads_agree() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source) in ROWS {
        let (lexer, deep) = both(source);
        if lexer != deep {
            failures.push(format!("{label}:\n  lexer {lexer:?}\n  cst   {deep:?}"));
        }
    }
    assert!(failures.is_empty(), "kotlin generic invocation calls disagree:\n{}", failures.join("\n"));
}
