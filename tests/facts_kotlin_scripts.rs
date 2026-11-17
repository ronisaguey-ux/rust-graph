//! Kotlin scripts: a `.kts` line with an argument list is a call.
//!
//! `settings.gradle.kts` is often a single line — `include(":shared")` — and the
//! one-pass reader files it as the call `include`. The CST has no
//! `call_expression` for a script statement (the grammar keeps the head and its
//! arguments as two children), so nothing wrote a call and the guard fell back
//! on every settings file: `call include line 1` was the corpus's largest call
//! cluster (60 of 72 fallbacks on a .kts sample).
//!
//! A script statement's event is the call itself too. Written as an assignment
//! merge (`m:[n:include,0]`) it paired with no line the lexer wrote, which is
//! how the row survived after the call was added.
//!
//! What is pinned is that both readers name the same call on the same line: the
//! equality is what the guard decides a fallback on.

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

/// The lexer's record and the CST record for one script snippet.
fn both(source: &str) -> (Vec<(String, u32)>, Vec<(String, u32)>) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (call_list(&lexer), call_list(&deep))
}

const ROWS: &[(&str, &str)] = &[
    ("a single include", "include(\":shared\")\n"),
    ("an include on a later line", "\ninclude(\":p0\")\ninclude(\":p1\")\n"),
    ("a plugin apply with a named argument", "apply(plugin = \"kotlin\")\n"),
    ("a call with several arguments", "include(\":a\", \":b\")\n"),
];

#[test]
fn kotlin_script_line_files_the_same_call() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source) in ROWS {
        let (lexer, deep) = both(source);
        if lexer != deep {
            failures.push(format!("{label}:\n  lexer {lexer:?}\n  cst   {deep:?}"));
        }
    }
    assert!(failures.is_empty(), "kotlin script calls disagree:\n{}", failures.join("\n"));
}

#[test]
fn a_script_call_is_not_also_an_assignment_event() {
    // `include(":x")` writes one event: the call. An assignment merge beside it
    // would pair with nothing and put the file back on the lexer.
    let source = "include(\":shared\")\n";
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    let events: Vec<&str> = deep.symbols.iter().map(|symbol| symbol.events.as_str()).filter(|e| !e.is_empty()).collect();
    assert_eq!(events.len(), 1, "one event list for one statement: {events:?}");
    assert!(events[0].starts_with("[\"x\","), "the event is the call, not an assignment: {events:?}");
    assert!(events[0].contains("[\"c\","), "the value is the call: {events:?}");
}
