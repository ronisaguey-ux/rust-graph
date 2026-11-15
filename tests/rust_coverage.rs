//! The Rust reader's coverage table.
//!
//! Every form the plan claims the Rust reader covers is one row here, and one
//! row is one assertion: both readers read the file and the guard finds nothing
//! to fall back for. A form is a shape, not a keyword — `Point { x: 1, y }` is
//! covered when the lexer and the CST agree about which symbol the fields belong
//! to, not when the grammar merely has a node for a struct literal.
//!
//! The fixtures are the smallest source that exercises the form, so a failure
//! names the form instead of a file.
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

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// One row per claimed form.
const COVERAGE: &[(&str, &str)] = &[
    (
        "struct literal: named fields, shorthand and spread",
        "fn f(p: Point) {\n    let p = Point { x: 1, y };\n    let q = Point { z: 2, ..p };\n}\n",
    ),
    (
        "struct literal as a call argument",
        "fn f() {\n    let r = build(Point { x: 4 });\n}\n",
    ),
    (
        "match arms: block body, guard, call value and wildcard",
        "fn g(v: i32) -> i32 { v }\nfn f(x: i32) -> i32 {\n    match x {\n        0 => { let z = 1; z }\n        1 if x > 0 => g(x),\n        _ => 2,\n    }\n}\n",
    ),
    (
        "closures: annotated return type with a block, and an expression body",
        "fn f() {\n    let add = |x: i32| -> i32 { x + 1 };\n    let twice = |v: i32| v * 2;\n}\n",
    ),
    (
        "trait bounds on a parameter and a where clause",
        "fn f<T: Clone + Send>(t: T) -> T where T: Send { t }\n",
    ),
    (
        "dynamic types: dyn in a reference and behind Box",
        "fn g(x: &dyn Fn() -> i32, y: Box<dyn Send>) {}\n",
    ),
    (
        "use lists: nested braces with a glob and a self item",
        "use crate::deep::{facts, sink::{self, write}};\nuse std::collections::*;\n",
    ),
    (
        "use lists: super, crate and self paths with an alias",
        "use super::parent::{self, Child};\nuse self::local::helper as h;\nuse crate::inner::Thing;\n",
    ),
    (
        "use lists: pub(crate), pub(super) and pub(in path)",
        "pub(crate) use crate::inner::Thing;\npub(super) use crate::other::*;\npub(in crate::x) use c::d;\n",
    ),
    (
        "a method in an impl with a return type and a doc comment",
        "struct S {\n    v: u64,\n}\n\nimpl S {\n    /// Adds `other`; true if that changed anything.\n    pub fn union(&mut self, other: &S) -> bool {\n        let mut changed = false;\n        changed |= other.v != self.v;\n        changed\n    }\n}\n",
    ),
    (
        "function types: dyn Fn in a parameter and a Box<dyn Fn>",
        "fn g(x: &dyn Fn(i32) -> i32, y: Box<dyn Fn() -> i32>) {}\n",
    ),
    (
        "function types: impl Fn as a parameter bound",
        "fn h(f: impl Fn(i32) -> i32) -> i32 { f(1) }\n",
    ),
    (
        "an if let pattern with tuple and enum constructors",
        "fn f(def: u8, act: u8) {\n    if let (Some(d), Action::Reduce(p)) = (def, act) {\n        let q = d;\n        let r = p;\n        let s = q + r;\n    }\n}\n",
    ),
    (
        "a pub(crate) field: the restriction is not a value",
        "pub struct Counter {\n    pub(crate) meter: u64,\n    pub(super) scale: u8,\n}\n",
    ),
    (
        "an extern block with C declarations",
        "extern \"C\" {\n    fn ioctl(fd: i32, request: u64, ...) -> i32;\n    fn close(fd: i32) -> i32;\n}\n",
    ),
    (
        "let declarations: mut, annotated and tuple-pattern bindings",
        "fn f() {\n    let mut c = 3;\n    let d: i32 = 4;\n    let (a, b) = (1, 2);\n    let e = c + d + a + b;\n}\n",
    ),
];

/// Forms the reader does not cover yet, with the exact disagreement.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];

#[test]
fn every_claimed_rust_form_reads_the_same_on_both_paths() {
    let mut failed: Vec<String> = Vec::new();
    for (name, source) in COVERAGE {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        assert!(!lexer.symbols.is_empty(), "{name}: the lexer read no symbol");
        let verdict = pipeline::loses(&lexer, &deep);
        if verdict.lost() {
            failed.push(format!("{name}: {verdict:?}"));
        }
    }
    assert!(failed.is_empty(), "the Rust reader falls back on {} form(s):\n{}", failed.len(), failed.join("\n"));
}

/// P5-06: a use list parses to the import shape the guard expects, tuple for
/// tuple — the module path, the name, the alias and whether it is a glob. The
/// guard only says "no loss"; this says the two readers agree on what was
/// imported, which is what a resolver consumes.
#[test]
fn use_lists_parse_to_the_import_shape_both_readers_agree_on() {
    let source = "use crate::deep::{facts, sink::{self, write}};\nuse std::collections::*;\nuse super::parent::{self, Child};\npub(crate) use crate::inner::Thing;\npub(super) use crate::other::*;\nuse self::local::helper as h;\n";
    fn shape(facts: &Facts) -> Vec<(String, String, String, bool)> {
        let mut out: Vec<(String, String, String, bool)> = facts
            .imports
            .iter()
            .map(|import| (import.module.clone(), import.name.clone(), import.alias.clone(), import.kind == facts::IMPORT_STAR))
            .collect();
        out.sort();
        out
    }
    let lexer = shape(&lexer_facts(source));
    let deep = shape(&deep_facts(source));
    assert_eq!(lexer, deep, "the two readers import different things");
    for expected in [
        ("crate.deep".to_string(), "facts".to_string(), "facts".to_string(), false),
        ("crate.deep.sink".to_string(), "self".to_string(), "self".to_string(), false),
        ("crate.deep.sink".to_string(), "write".to_string(), "write".to_string(), false),
        ("std.collections".to_string(), String::new(), String::new(), true),
        ("super.parent".to_string(), "Child".to_string(), "Child".to_string(), false),
        ("crate.inner.Thing".to_string(), String::new(), "Thing".to_string(), false),
        ("crate.other".to_string(), String::new(), String::new(), true),
        ("self.local.helper".to_string(), String::new(), "h".to_string(), false),
    ] {
        assert!(deep.contains(&expected), "missing import {expected:?} in {deep:?}");
    }
}

/// A recorded gap is asserted to disagree, so closing one without moving the row
/// up fails the suite. An empty gap list asserts nothing, which is the honest
/// state of the forms above.
#[test]
fn every_known_rust_gap_still_fails() {
    for (name, source, why) in KNOWN_GAPS {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        let verdict = pipeline::loses(&lexer, &deep);
        assert!(verdict.lost(), "{name} reads cleanly now — move it up to COVERAGE ({why})");
    }
}
