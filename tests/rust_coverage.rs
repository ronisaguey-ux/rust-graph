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
    // P5-10. Each of the four is the shape of a real arm, quoted from the corpus
    // file named in the label, reduced to the tokens that carry the pattern.
    (
        "match guard: `b'/' if c.peek_at(1) == b'/'` (000081_kotlin.rs:513)",
        "fn slash(c: Cursor) -> u8 {\n    match c.peek() {\n        b'/' if c.peek_at(1) == b'/' => 1,\n        _ => 0,\n    }\n}\n",
    ),
    (
        "or-pattern: `LPAREN | LBRACKET | LBRACE` (000081_kotlin.rs:804)",
        "fn field(t: Tok) -> usize {\n    match t {\n        LPAREN | LBRACKET | LBRACE => 1,\n        RPAREN | RBRACKET => 2,\n        _ => 0,\n    }\n}\n",
    ),
    (
        "binding at: `Some(q @ ('\"' | '\\''))` (001544_before_edit.rs:476) and `x @ 1..=5`",
        "fn quote(q: Option<char>) -> bool {\n    match q {\n        Some(q @ ('\"' | '\\'')) => true,\n        _ => false,\n    }\n}\nfn small(n: u32) -> &'static str {\n    match n {\n        x @ 1..=5 => \"small\",\n        _ => \"other\",\n    }\n}\n",
    ),
    (
        "range pattern: `0x00..=0x7f` and `0xc2..=0xdf` (000456_cursor.rs:134)",
        "fn lead(b0: u8) -> usize {\n    match b0 {\n        0x00..=0x7f => 1,\n        0xc2..=0xdf => 2,\n        0xe0..=0xef => 3,\n        _ => 0,\n    }\n}\n",
    ),
    // P5-11: a block keyword is one rule, so the statements inside are read
    // instead of merged into a single value that drops them.
    (
        "async block in value position",
        "fn d() -> u32 {\n    let b = async {\n        let x = 1;\n        work(x);\n        x + 1\n    };\n    0\n}\n",
    ),
    (
        "unsafe block in value position",
        "fn d(m: &Mutex<u32>) -> u32 {\n    let v = unsafe {\n        let p = m.get();\n        *p\n    };\n    v\n}\n",
    ),
    (
        "try block in value position",
        "fn d() -> Result<u32, E> {\n    let r = try {\n        let a = step(1)?;\n        finalize(a)?\n    };\n    r\n}\n",
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
        "impl for a generic type is named by its constructor",
        "impl TryFrom<CoreLog> for JsonString<LogEntry> {\n    fn try_from(v: CoreLog) -> Result<()> {\n        todo!()\n    }\n}\n",
    ),
    (
        "let-chain: a && let pattern binds and calls nothing",
        "fn f(w: &mut W) -> bool {\n    if w.len() == 3\n        && let Some(old) = w.pop_front()\n    {\n        let _ = old;\n    }\n    true\n}\n",
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
    (
        "an impl header with generics and a where clause",
        "impl<CT, RT> Handle<CT, RT>\nwhere\n    CT: Clone,\n{\n    fn get(&self) -> u8 {\n        0\n    }\n}\n",
    ),
    (
        "an impl whose type is a path",
        "impl tonic::client::X {\n    fn a(&self) -> u8 {\n        0\n    }\n}\n",
    ),
    (
        "a match arm with a range and an alternation",
        "fn f(c: u8) -> u8 {\n    match c {\n        b'a'..=b'z' | b'[' => 3,\n        _ => 0,\n    }\n}\n",
    ),
    (
        "a closure whose parameter is a reference pattern",
        "fn f(all: &mut Vec<u8>) {\n    all.sort_by_key(|&id| all[id as usize]);\n}\n",
    ),
    (
        "a let-else whose else block diverges",
        "fn f() {\n    let Some(x) = g(1)\n    else {\n        return;\n    };\n    let y = x;\n}\n",
    ),
    (
        "a member chain across lines through the ? operator",
        "fn f(client: H) {\n    client\n        .borrow()?\n        .core\n        .set(1);\n}\n",
    ),
    (
        "a closure whose parameter type is a function type (001452:386)",
        "fn f(stops: S) -> Fold {\n    let fold = |pick: fn(f64, f64) -> f64, start: f64| Rgba {\n        r: stops.iter().fold(start, pick),\n    };\n    fold\n}\n",
    ),
    (
        "a block that opens with a use names nothing (001538:189)",
        "fn f(b: &[u8]) -> String {\n    let d = {\n        use sha2::Digest;\n        let mut h = New::new();\n        h.update(b);\n        finish(h)\n    };\n    d\n}\n",
    ),
    (
        "char literals: an escaped scalar and a multi-byte one (001440:545)",
        "fn f() -> usize {\n    let marks = &['*', '\\u{2020}', '†', '§'];\n    marks.len()\n}\n",
    ),
    (
        "a class binding: the variant itself, a call chain, try and parentheses",
        "fn f(m: Msg) {\n    let v = Value::Object(m);\n    let text = Value::Object(m).to_string();\n    let u = Value::Object(m)?;\n    let w = (Value::Object(m));\n}\n",
    ),
    (
        "a statement where an item cannot go: a merge out of method bodies (001183_stochastics.rs:188)",
        "impl S {\n    fn reset(&mut self) { self.a.clear(); }\n    self.max_deque.clear();\n}\n",
    ),
    (
        "for over a struct pattern: the brace is the pattern's (000942_workflow_future.rs:399)",
        "fn f(jobs: Vec<Job>) {\n    for Job { variant } in jobs {\n        let _ = variant;\n    }\n}\n",
    ),
    (
        "a multi-line literal: a field on its own line after an if let (000930_worker.rs:488)",
        "fn f(ctx: C) -> S {\n    S {\n        a: 1,\n        b: match ctx.v() {\n            0 => 2,\n            _ => 3,\n        },\n        c: ctx.sticky(),\n    }\n}\n",
    ),
    (
        "a macro item where an item goes (000965_mod.rs:2721)",
        "impl S {\n    sched_to_start_impl!(scheduled_time);\n}\n",
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

/// P5-11: every block keyword routes through the same block handling, so a
/// statement inside a block is read as a statement — a new keyword cannot
/// silently merge the body into one value and drop what is in it.
#[test]
fn every_block_keyword_reads_the_statements_inside_it() {
    // The call inside each body is the canary: merged into one value it
    // disappears from the record, and nothing in the guard would say so.
    let rows: &[(&str, &str)] = &[
        ("async", "fn d() -> u32 {\n    let b = async {\n        inner(1);\n        2\n    };\n    0\n}\n"),
        ("unsafe", "fn d() -> u32 {\n    let b = unsafe {\n        inner(1);\n        2\n    };\n    0\n}\n"),
        ("try", "fn d() -> u32 {\n    let b = try {\n        inner(1)?;\n        2\n    };\n    0\n}\n"),
        ("const", "fn d() -> u32 {\n    let b = const { inner(1); 2 };\n    0\n}\n"),
        ("bare", "fn d() -> u32 {\n    let b = {\n        inner(1);\n        2\n    };\n    0\n}\n"),
    ];
    let mut problems = Vec::new();
    for (label, source) in rows {
        for (reader, facts) in [("lexer", lexer_facts(source)), ("cst", deep_facts(source))] {
            let calls: Vec<&str> = facts
                .symbols
                .iter()
                .flat_map(|symbol| symbol.calls.iter().map(|call| call.path.as_str()))
                .collect();
            let count = calls.iter().filter(|path| **path == "inner").count();
            if count != 1 {
                problems.push(format!("{label} [{reader}]: `inner` recorded {count} time(s), want 1: {calls:?}"));
            }
        }
    }
    assert!(problems.is_empty(), "a block keyword dropped a statement:\n{}", problems.join("\n"));
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
