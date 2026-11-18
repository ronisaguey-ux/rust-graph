//! Kotlin continuation lines: a line that opens with the operator belongs to
//! the statement above it.
//!
//! `_intercepted` / `?: (context[ContinuationInterceptor]?.intercept(this) ?: this)`
//! and `a === b || c is D` / `&& this.x == other.x` are one statement each. The
//! CST folds the continuation into the statement it continues and writes no
//! event; the one-pass reader treated the line as a statement of its own and
//! invented an `x` for it (24 rows, then 75 files once the whole family was
//! measured: `?:`, `&&`, `||` and a chained `.`).
//!
//! What is pinned is that neither record has an event on the continuation line,
//! and that the line above keeps its own.

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

const ROWS: &[(&str, &str, u32)] = &[
    (
        "an elvis continuation",
        "fun a(p: String?): String {\n    return p\n        ?: error(\"missing\")\n}\n",
        4,
    ),
    (
        "a boolean continuation",
        "fun b(a: Int, o: Int): Boolean {\n    return a === o || a < o\n        && a == o\n}\n",
        3,
    ),
    (
        "a chained call after a line break",
        "fun c(x: String?) = x\n    ?.trim()\n    .lowercase()\n",
        3,
    ),
];

#[test]
fn kotlin_continuation_line_states_nothing_of_its_own() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source, line) in ROWS {
        let (lexer, deep) = both(source);
        let want = format!("[\"x\",{line},");
        let want_a = format!("[\"a\",{line},");
        if lexer.contains(&want) || lexer.contains(&want_a) {
            failures.push(format!("{label}: the lexer filed the continuation line {line}: {lexer}"));
        }
        if deep.contains(&want) || deep.contains(&want_a) {
            failures.push(format!("{label}: the cst filed the continuation line {line}: {deep}"));
        }
    }
    assert!(failures.is_empty(), "kotlin continuation lines disagree:\n{}", failures.join("\n"));
}
