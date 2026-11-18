//! Kotlin annotations: a qualified name is the whole path.
//!
//! `@kotlin.jvm.JvmName("flatMapSequence")` is written in the stdlib's generated
//! sources, and the one-pass reader files it as the call `kotlin.jvm.JvmName`
//! with the receiver `kotlin.jvm` in the event. The walk took the first
//! identifier child as the name, so the call was `kotlin` and the guard counted
//! the line as a lost call (18 rows of the kotlin dump, plus the same shape in
//! `@SinceKotlin`, `@OptIn` and the Gradle plugin's `@PathSensitive`).
//!
//! What is pinned is that both readers name the same call on the same line.

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

/// The lexer's record and the CST record for one snippet.
fn both(source: &str) -> (Vec<(String, u32)>, Vec<(String, u32)>) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (call_list(&lexer), call_list(&deep))
}

const ROWS: &[(&str, &str)] = &[
    (
        "a qualified annotation",
        "@kotlin.jvm.JvmName(\"flatMapSequence\")\nfun a() {}\n",
    ),
    (
        "a plain annotation",
        "@JvmName(\"x\")\nfun a() {}\n",
    ),
    (
        "a qualified annotation with a marker",
        "@kotlin.jvm.JvmName(\"y\")\nclass A\n",
    ),
    (
        "a qualified annotation with no arguments",
        "@kotlin.Suppress\nfun a() {}\n",
    ),
    (
        "annotation with collection literal and named arg",
        "@JdkVersions(versions = [JavaVersion.VERSION_11], compatibleWithGradle = false)\nfun a() {}\n",
    ),
    (
        "file annotation with empty args",
        "@file:kotlin.jvm.JvmMultifileClass()\npackage p\n",
    ),
    (
        "bracketed file annotation list",
        "@file:[JvmName(\"MCF\") JvmMultifileClass]\npackage p\n",
    ),
];

#[test]
fn kotlin_annotation_files_the_same_call() {
    // Every call the one-pass reader files must be in the CST's record: a
    // missing call is what the guard reports, while an extra one is one of the
    // annotation shapes the guard accepts by name (`keyword_call`).
    let mut failures: Vec<String> = Vec::new();
    for (label, source) in ROWS {
        let (lexer, deep) = both(source);
        for call in &lexer {
            if !deep.contains(call) {
                failures.push(format!("{label}: {call:?} is missing from the cst {deep:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "kotlin annotation calls disagree:\n{}", failures.join("\n"));
}
/// `@SinceKotlin("1.1") public actual typealias Error = java.lang.Error`: the
/// annotation in front of a declaration is not a name, so the line declares and
/// assigns nothing. Read as an assignment it filed `a Error = java.lang.Error`,
/// a row the CST never writes (57 rows and 9 files of the kotlin dump).
const DECLARATIONS: &[&str] = &[
    "@SinceKotlin(\"1.1\") public actual typealias Error = java.lang.Error\n",
    "@Deprecated(\"d\") public actual typealias E = java.lang.Error\n",
];

#[test]
fn kotlin_annotated_declaration_is_not_an_assignment() {
    let mut failures: Vec<String> = Vec::new();
    for source in DECLARATIONS {
        let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
        let lexer = parser::parse(source.as_bytes(), language);
        let mut events = String::new();
        for symbol in &lexer.symbols {
            events.push_str(&symbol.events);
        }
        if events.contains("[\"a\",") {
            failures.push(format!("{source:?}: the lexer filed an assignment for a declaration: {events}"));
        }
    }
    assert!(failures.is_empty(), "kotlin annotated declarations disagree:\n{}", failures.join("\n"));
}
