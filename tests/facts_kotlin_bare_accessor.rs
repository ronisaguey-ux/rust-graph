//! Kotlin bare accessors: `private set` is a statement of its own.
//!
//! `override var size: Int = 0` followed by `private set` is read by the
//! one-pass reader as an `x` event on the accessor's line whose value is the
//! modifier and the keyword merged (`m:[n:private,n:set]`). The grammar
//! required an accessor to carry a body, so a bare one turned the property into
//! a recovery and the walk wrote nothing for the line: ArrayDeque.kt,
//! SlidingWindow.kt and SequenceTest.kt each lost a statement on it (21 files of
//! the kotlin dump, 20 rows).
//!
//! What is pinned is that the accessor's own line carries that value in both
//! records, and that an accessor with a body still files its body.

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

/// The lexer's record and the CST record for one snippet.
fn both(source: &str) -> (String, String) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (events(&lexer), events(&deep))
}

const ROWS: &[(&str, &str, u32)] = &[
    (
        "an override's bare setter before another member",
        "class A {\n    override var size: Int = 0\n        private set\n\n    fun other(): Int = 1\n}\n",
        3,
    ),
    (
        "a bare setter in a class that ends there",
        "class A {\n    var total: Int = 0\n        private set\n}\n",
        3,
    ),
    (
        "a top-level bare setter",
        "var count: Int = 0\n    private set\n\nfun other() {}\n",
        2,
    ),
];

#[test]
fn kotlin_bare_accessor_files_its_modifiers_on_its_own_line() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source, line) in ROWS {
        let (lexer, deep) = both(source);
        let want = format!("[\"x\",{line},[\"m\",[[\"n\",\"private\"],[\"n\",\"set\"]]]]");
        if !deep.contains(&want) {
            failures.push(format!("{label}: the cst has no bare accessor on line {line}: {deep}"));
        }
        if !lexer.contains(&want) {
            failures.push(format!("{label}: the lexer has no bare accessor on line {line}: {lexer}"));
        }
    }
    assert!(failures.is_empty(), "kotlin bare accessors disagree:\n{}", failures.join("\n"));
}
