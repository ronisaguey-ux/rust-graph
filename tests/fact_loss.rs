//! The fact-loss reproduction: run the one-pass lexer and the CST walk over
//! the same real tree and print, per file, the symbols, calls and imports each
//! found, the difference, and — for every lost fact — which layer lost it.
//!
//! ```text
//! FACT_LOSS_DIR=/path/to/tree cargo test --features "deep parser" \
//!     --test fact_loss -- --ignored --nocapture
//! ```
//!
//! `FACT_LOSS_DIR` defaults to a tree derived from `CARGO_MANIFEST_DIR`, and the
//! test is `#[ignore]`d because it reads a tree outside this repository.
//!
//! The classification is mechanical, never a guess:
//!
//! - **grammar-interpolation**: the construct is inside a string the grammar
//!   keeps as one token (`f"{len(x)}"`, `` `${f(x)}` ``). Every string leaf is
//!   collected; a lost call whose callee is only inside one was never in the
//!   tree, so no mapping could have seen it.
//! - **grammar-unseen-token**: the callee's name is not a leaf anywhere.
//! - **mapping**: the CST has the construct and the walk emitted nothing for
//!   it — a node kind `deep_facts` does not map.
//! - **resolution**: the fact is there in a shape the resolver cannot tie — a
//!   receiver dropped from a path, or a variable whose class is not recorded
//!   (the resolver's `typed-variable` edges come from `Facts::vars`).
//! - **accepted**: a difference the one-pass lexer gets wrong and the CST gets
//!   right — a definition stamped with the line its statement ends on, a call
//!   whose receiver the lexer pasted on (`Path().resolve`), an `assert (a, b)
//!   == ...` read as a call to `assert`, an anonymous call named `.call`.
//!
//! A file with any loss is a file `auto` falls back on; that is the guard's
//! contract and this harness reports the fallback line it would write, with the
//! facts that triggered it (`guard_losses` mirrors `pipeline::loses`).

#![cfg(feature = "deep")]

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
#[path = "../src/deep/mod.rs"]
mod deep;
#[path = "../src/deep_integration.rs"]
mod deep_integration;
#[path = "../src/deep_facts.rs"]
mod deep_facts;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use deep_integration::Lang;

fn lang_of(path: &Path) -> Option<Lang> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py") => Some(Lang::Python),
        Some("js") => Some(Lang::JavaScript),
        Some("ts") => Some(Lang::TypeScript),
        Some("tsx") => Some(Lang::Tsx),
        _ => None,
    }
}

fn files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            files(&path, out);
        } else if kind.is_file() && lang_of(&path).is_some() {
            out.push(path);
        }
    }
}

fn lexer_facts(bytes: &[u8], suffix: &str) -> facts::Facts {
    let language = langs::by_content(suffix, &bytes[..bytes.len().min(4096)]).expect("a lexer row");
    parser::parse(bytes, language)
}

fn deep_facts(bytes: &[u8], lang: Lang) -> facts::Facts {
    let cst = deep_integration::parse_file(bytes, lang).expect("a deep parse");
    deep_facts::extract(bytes, &cst, lang)
}

/// Every leaf's text, the evidence a construct was tokenised at all.
fn leaf_texts(bytes: &[u8], lang: Lang) -> BTreeSet<String> {
    let cst = deep_integration::parse_file(bytes, lang).expect("a deep parse");
    let mut out = BTreeSet::new();
    for node in &cst.nodes {
        if node.first_child == deep_integration::NONE {
            out.insert(String::from_utf8_lossy(&bytes[node.start as usize..node.end as usize]).into_owned());
        }
    }
    out
}

/// The string leaves, as text. The grammar keeps an interpolated string
/// (`f"{len(x)}"`, `` `${f(x)}` ``) as one leaf, so a call inside one is not in
/// the tree at all — a grammar gap, not a mapping one.
fn string_leaves(bytes: &[u8], lang: Lang) -> Vec<String> {
    let cst = deep_integration::parse_file(bytes, lang).expect("a deep parse");
    let mut out = Vec::new();
    for node in &cst.nodes {
        if node.first_child != deep_integration::NONE {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes[node.start as usize..node.end as usize]).into_owned();
        let trimmed = text.trim_start_matches(|ch: char| ch.is_ascii_alphabetic());
        let quoted = trimmed.starts_with('"') || trimmed.starts_with('\'') || trimmed.starts_with('`');
        if quoted && (text.contains('{') || text.contains("${")) {
            out.push(text);
        }
    }
    out
}

fn inside_interpolated(strings: &[String], name: &str) -> bool {
    let last = name.trim_start_matches('.').rsplit('.').next().unwrap_or(name);
    strings.iter().any(|text| text.contains(&format!("{last}(")) || text.contains(&format!("{last} ")))
}

fn call_set(f: &facts::Facts) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> =
        f.symbols.iter().flat_map(|s| s.calls.iter().map(|c| (c.path.clone(), c.line))).collect();
    out.sort();
    out
}

fn symbol_set(f: &facts::Facts) -> Vec<(String, u8, u32)> {
    let mut out: Vec<(String, u8, u32)> =
        f.symbols.iter().map(|s| (s.name.clone(), s.kind, s.line)).collect();
    out.retain(|(_, kind, _)| *kind != facts::KIND_MODULE);
    out.sort();
    out
}

fn import_set(f: &facts::Facts) -> Vec<(u32, String, String, String, u8)> {
    let mut out: Vec<(u32, String, String, String, u8)> = f
        .imports
        .iter()
        .map(|i| (i.line, i.module.clone(), i.name.clone(), i.alias.clone(), i.kind))
        .collect();
    out.sort();
    out
}

fn multiset_loss<T: Ord + Clone>(lexer: &[T], deep: &[T]) -> Vec<T> {
    let mut have: BTreeMap<&T, i64> = BTreeMap::new();
    for item in lexer {
        *have.entry(item).or_default() += 1;
    }
    for item in deep {
        if let Some(count) = have.get_mut(item) {
            *count -= 1;
        }
    }
    have.into_iter()
        .filter(|(_, count)| *count > 0)
        .flat_map(|(item, count)| std::iter::repeat(item.clone()).take(count as usize))
        .collect()
}

#[derive(Default, Clone, Copy)]
struct Counts {
    symbols: usize,
    calls: usize,
    imports: usize,
}

impl Counts {
    fn of(f: &facts::Facts) -> Counts {
        Counts { symbols: symbol_set(f).len(), calls: call_set(f).len(), imports: f.imports.len() }
    }
}

#[derive(Default)]
struct Tally {
    grammar: usize,
    mapping: usize,
    resolution: usize,
    /// Same symbol on a different line, the lexer's statement-end stamp
    /// against the CST's start line, and a call the lexer pasted its receiver
    /// onto (`Path().resolve`). Both are the lexer's error, already proven by
    /// `tests/deep_facts.rs`; they are counted, not called losses.
    accepted: usize,
    /// Files whose record `auto` would hand back to the lexer, and why the
    /// guard said so.
    guard_files: usize,
    guard_interpolation: usize,
    guard_phantom: usize,
    guard_event: usize,
    guard_other: usize,
}

impl Tally {
    fn add(&mut self, other: &Tally) {
        self.grammar += other.grammar;
        self.mapping += other.mapping;
        self.resolution += other.resolution;
        self.accepted += other.accepted;
        self.guard_files += other.guard_files;
        self.guard_interpolation += other.guard_interpolation;
        self.guard_phantom += other.guard_phantom;
        self.guard_event += other.guard_event;
        self.guard_other += other.guard_other;
    }
    fn total(&self) -> usize {
        self.grammar + self.mapping + self.resolution
    }
    fn charge(&mut self, class: &str) {
        match class {
            "grammar" | "grammar-interpolation" | "grammar-unseen-token" => self.grammar += 1,
            "mapping" => self.mapping += 1,
            "resolution" => self.resolution += 1,
            _ => self.accepted += 1,
        }
    }
}

fn last_of(path: &str) -> &str {
    path.trim_start_matches('.').rsplit('.').next().unwrap_or(path)
}

/// Which layer lost a call.
fn classify_call(name: &str, line: u32, deep: &[(String, u32)], leaves: &BTreeSet<String>, strings: &[String]) -> &'static str {
    // `Path().resolve`: the one-pass lexer concatenates a receiver call into
    // the member call. The CST writes the two calls it is.
    if name.contains('(') {
        return "accepted";
    }
    // `assert (a, b) == (c, d)`: the lexer reads `assert(` as a call. The
    // keyword is not a callee in any language it lexes; a fact layer must not
    // invent one to agree with it.
    if matches!(name, "assert" | "return" | "yield" | "await" | "print" | "throw" | "new" | "del" | "raise") {
        return "accepted";
    }
    // `make(1, 2)(3)`: the one-pass lexer names an anonymous call `.call`.
    // There is no such name in the source for the CST to read.
    if name == ".call" {
        return "accepted";
    }
    let last = last_of(name);
    // A call inside an interpolated string is not in the tree at all: the
    // grammar keeps `f"{len(x)}"` as one token. Grammar gap, whatever else the
    // line holds.
    if inside_interpolated(strings, name) && !deep.iter().any(|(path, at)| path == name && *at == line) {
        return "grammar-interpolation";
    }
    let same_line: Vec<&(String, u32)> = deep.iter().filter(|(path, at)| *at == line && last_of(path) == last).collect();
    if let Some((path, _)) = same_line.first() {
        // The CST wrote `.group` where the source says `match.group`: the
        // receiver is in the tree and the walk dropped it, a mapping gap. A
        // bare `lower` against `.lower` is not: there is no receiver in the
        // source, the call's object is itself a call.
        return if path.starts_with('.') && !name.starts_with('.') && name != last { "mapping" } else { "accepted" };
    }
    if deep.iter().any(|(path, at)| last_of(path) == last && (*at as i64 - line as i64).abs() <= 1) {
        return "accepted";
    }
    if !leaves.contains(last) && !leaves.contains(name) {
        return "grammar-unseen-token";
    }
    if deep.iter().any(|(path, _)| last_of(path) == last) {
        return "resolution";
    }
    "mapping"
}

/// A mirror of `pipeline::loses`, the predicate `auto` uses to keep the
/// lexer's record. It is copied, not called, because the pipeline's copy is not
/// reachable from a test; the reproduction reports which check fires so a
/// fallback is attributed to the fact that caused it.
fn guard_losses(lexer: &facts::Facts, deep: &facts::Facts) -> Vec<String> {
    let mut out = Vec::new();
    for import in &lexer.imports {
        let same = deep.imports.iter().any(|row| {
            row.module == import.module && row.name == import.name && row.alias == import.alias && row.kind == import.kind
        });
        if !same {
            out.push(format!("import {}|{}|{}", import.module, import.name, import.alias));
        }
    }
    for symbol in lexer.symbols.iter().filter(|symbol| symbol.kind != facts::KIND_MODULE) {
        let same = deep.symbols.iter().any(|row| row.name == symbol.name && row.kind == symbol.kind);
        if !same {
            out.push(format!("symbol {} kind={}", symbol.name, symbol.kind));
        }
    }
    let name_of = |path: &str| path.rsplit('.').next().unwrap_or(path).to_string();
    for symbol in &lexer.symbols {
        for call in &symbol.calls {
            let name = name_of(&call.path);
            let near = |line: u32| (line as i64 - call.line as i64).abs() <= 1;
            let same = deep.symbols.iter().flat_map(|row| row.calls.iter()).any(|row| name_of(&row.path) == name && near(row.line));
            if !same {
                out.push(format!("call {} line={}", call.path, call.line));
            }
        }
    }
    let events = |facts: &facts::Facts, marker: &str| -> usize {
        facts.symbols.iter().map(|symbol| symbol.events.matches(marker).count()).sum()
    };
    for marker in ["[\"a\",", "[\"r\",", "[\"x\","] {
        let (lexer_events, deep_events) = (events(lexer, marker), events(deep, marker));
        if deep_events < lexer_events {
            out.push(format!("events {marker} {deep_events} < {lexer_events}"));
        }
    }
    out
}
