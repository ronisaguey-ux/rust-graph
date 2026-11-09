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

/// Report one file into `out`; returns whether it had a loss and the tally.
fn report_file(path: &Path, out: &mut String) -> (bool, Tally) {
    let bytes = fs::read(path).expect("file");
    let suffix = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let lang = lang_of(path).expect("language");
    let lexer = lexer_facts(&bytes, suffix);
    let deep = deep_facts(&bytes, lang);
    let leaves = leaf_texts(&bytes, lang);
    let strings = string_leaves(&bytes, lang);
    let a = Counts::of(&lexer);
    let b = Counts::of(&deep);
    let lost_symbols = multiset_loss(&symbol_set(&lexer), &symbol_set(&deep));
    let lost_calls = multiset_loss(&call_set(&lexer), &call_set(&deep));
    let lost_imports = multiset_loss(&import_set(&lexer), &import_set(&deep));
    let mut lexer_vars = lexer.vars.clone();
    let mut deep_vars = deep.vars.clone();
    lexer_vars.sort();
    deep_vars.sort();
    let lost_vars = multiset_loss(&lexer_vars, &deep_vars);
    let guard = guard_losses(&lexer, &deep);
    if lost_symbols.is_empty() && lost_calls.is_empty() && lost_imports.is_empty() && lost_vars.is_empty() && guard.is_empty() {
        return (false, Tally::default());
    }
    let deep_calls = call_set(&deep);
    let deep_symbols = symbol_set(&deep);
    let deep_modules: BTreeSet<String> = deep.imports.iter().map(|i| i.module.clone()).collect();
    let (deep_names, deep_aliases): (BTreeSet<&str>, BTreeSet<&str>) = (
        deep.imports.iter().map(|i| i.name.as_str()).collect(),
        deep.imports.iter().map(|i| i.alias.as_str()).collect(),
    );
    let mut tally = Tally::default();
    let mut lines = Vec::new();
    for (name, kind, line) in &lost_symbols {
        let class = if deep_symbols.iter().any(|(other, other_kind, _)| other == name && other_kind == kind) {
            "accepted"
        } else if deep_symbols.iter().any(|(other, _, _)| other == name) {
            "accepted"
        } else if leaves.contains(name) {
            "mapping"
        } else {
            "grammar-unseen-token"
        };
        tally.charge(class);
        lines.push(format!("  symbol {name} kind={kind} line={line}: {class}"));
    }
    for (name, line) in &lost_calls {
        let class = classify_call(name, *line, &deep_calls, &leaves, &strings);
        tally.charge(class);
        lines.push(format!("  call {name} line={line}: {class}"));
    }
    for (line, module, name, alias, _) in &lost_imports {
        let class = if deep_modules.contains(module) && (deep_names.contains(name.as_str()) || deep_aliases.contains(alias.as_str())) {
            "resolution"
        } else if deep_modules.contains(module) {
            "mapping"
        } else if leaves.contains(module.rsplit('.').next().unwrap_or(module)) || leaves.contains(module) {
            "mapping"
        } else {
            "grammar-unseen-token"
        };
        tally.charge(class);
        lines.push(format!("  import {module}|{name}|{alias} line={line}: {class}"));
    }
    // A variable with a known class is what ties `user.may()` to `User.may`:
    // the resolver's typed-variable edges come from this table.
    for (name, class) in &lost_vars {
        tally.charge("resolution");
        lines.push(format!("  var {name} = {class}: resolution"));
    }
    // The guard's own verdict, attributed.
    if !guard.is_empty() {
        tally.guard_files += 1;
        for cause in &guard {
            if let Some(rest) = cause.strip_prefix("call ") {
                let path = rest.split(" line=").next().unwrap_or(rest);
                let line: u32 = rest.rsplit("line=").next().and_then(|text| text.parse().ok()).unwrap_or(0);
                match classify_call(path, line, &deep_calls, &leaves, &strings) {
                    "grammar-interpolation" => tally.guard_interpolation += 1,
                    "accepted" => tally.guard_phantom += 1,
                    _ => tally.guard_other += 1,
                }
            } else if cause.starts_with("events ") {
                tally.guard_event += 1;
            } else {
                tally.guard_other += 1;
            }
        }
        let mut causes = guard.clone();
        causes.truncate(3);
        lines.push(format!("  guard: falls back on {} facts ({causes:?})", guard.len()));
    }
    out.push_str(&format!(
        "{}\n  lexer symbols {} calls {} imports {} | deep symbols {} calls {} imports {}\n  LOST symbols {} calls {} imports {} -> grammar {} mapping {} resolution {} (accepted {} contract differences)\n",
        path.display(),
        a.symbols,
        a.calls,
        a.imports,
        b.symbols,
        b.calls,
        b.imports,
        lost_symbols.len(),
        lost_calls.len(),
        lost_imports.len(),
        tally.grammar,
        tally.mapping,
        tally.resolution,
        tally.accepted,
    ));
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    (true, tally)
}

fn default_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("felicity-run")
}

fn walk_root(root: &Path) -> (Counts, Counts, Tally, usize, usize) {
    let mut paths = Vec::new();
    files(root, &mut paths);
    paths.sort();
    let (mut lexer_total, mut deep_total) = (Counts::default(), Counts::default());
    let mut tally = Tally::default();
    let (mut lossy, mut parsed) = (0usize, 0usize);
    let mut out = String::new();
    for path in &paths {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let suffix = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let Some(lang) = lang_of(path) else { continue };
        if langs::by_content(suffix, &bytes[..bytes.len().min(4096)]).is_none() {
            continue;
        }
        parsed += 1;
        let lexer = lexer_facts(&bytes, suffix);
        let deep = deep_facts(&bytes, lang);
        let a = Counts::of(&lexer);
        let b = Counts::of(&deep);
        lexer_total.symbols += a.symbols;
        lexer_total.calls += a.calls;
        lexer_total.imports += a.imports;
        deep_total.symbols += b.symbols;
        deep_total.calls += b.calls;
        deep_total.imports += b.imports;
        let mut chunk = String::new();
        let (had_loss, file_tally) = report_file(path, &mut chunk);
        if had_loss {
            lossy += 1;
            tally.add(&file_tally);
        }
        out.push_str(&chunk);
    }
    let verbose = std::env::var("FACT_LOSS_FILES").is_ok();
    if verbose {
        println!("{out}");
    } else {
        // Only the per-file summary lines, still per file.
        for line in out.lines() {
            if !line.starts_with("  ") {
                println!("{line}");
            }
        }
    }
    (lexer_total, deep_total, tally, lossy, parsed)
}

/// `FACT_LOSS_DUMP=<a file>`: print every call, import and symbol both paths
/// read off that file, so a single loss can be looked at directly.
#[test]
#[ignore = "reads a file named by FACT_LOSS_DUMP"]
fn fact_loss_dump_one_file() {
    let path = PathBuf::from(std::env::var_os("FACT_LOSS_DUMP").expect("FACT_LOSS_DUMP"));
    let bytes = fs::read(&path).expect("file");
    let suffix = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let lang = lang_of(&path).expect("language");
    let lexer = lexer_facts(&bytes, suffix);
    let deep = deep_facts(&bytes, lang);
    println!("== lexer calls ==");
    for (p, line) in call_set(&lexer) {
        println!("  {line:5} {p}");
    }
    println!("== deep calls ==");
    for (p, line) in call_set(&deep) {
        println!("  {line:5} {p}");
    }
    println!("== lexer symbols ==");
    for (n, k, line) in symbol_set(&lexer) {
        println!("  {line:5} kind={k} {n}");
    }
    println!("== deep symbols ==");
    for (n, k, line) in symbol_set(&deep) {
        println!("  {line:5} kind={k} {n}");
    }
    println!("== module events ==");
    for (label, facts) in [("lexer", &lexer), ("deep", &deep)] {
        println!("  {label}: {:?}", facts.symbols.first().map(|symbol| &symbol.events));
    }
    println!("== symbol events ==");
    for (label, facts) in [("lexer", &lexer), ("deep", &deep)] {
        for symbol in &facts.symbols {
            if !symbol.events.is_empty() {
                println!("  {label} {}: {}", symbol.key, symbol.events);
            }
        }
    }
    println!("== lexer imports ==");
    for row in import_set(&lexer) {
        println!("  {row:?}");
    }
    println!("== deep imports ==");
    for row in import_set(&deep) {
        println!("  {row:?}");
    }
}

/// The mapping gaps this harness found on a real tree, pinned on one source.
/// Each block is a construct where `deep_facts` used to drop a fact the lexer
/// kept; the assertion is exact equality on calls, imports and symbol names,
/// so a regression reappears here rather than in a corpus number.
#[test]
fn fact_loss_the_mapping_gaps_are_closed() {
    let source = br#"
import os
from pkg.mod import (
    alpha, beta as gamma, delta)

def outer(items, headers):
    assert check(items), "why"
    value = alpha(items) if beta(headers) else gamma(items)
    span = items[1:min(len(items), 6)]
    pair = (items := delta(items))
    handler = lambda key="": os.environ.get(key)
    table = {k.lower(): v for k, v in (headers or {}).items()}
    made = make(1, 2)(3) if False else handler("x")
    return span, pair, table, made
"#;
    let lang = Lang::Python;
    let lexer = lexer_facts(source, "py");
    let deep = deep_facts(source, lang);
    let lexer_calls: Vec<String> = call_set(&lexer).into_iter().map(|(p, l)| format!("{p}@{l}")).collect();
    let deep_calls: Vec<String> = call_set(&deep).into_iter().map(|(p, l)| format!("{p}@{l}")).collect();
    let missing: Vec<&String> = lexer_calls
        .iter()
        // `make(1, 2)(3)`: the one-pass lexer names an anonymous call `.call`.
        // There is no such name in the source, so the CST has nothing to read.
        .filter(|call| !call.starts_with(".call@"))
        .filter(|call| !deep_calls.contains(*call))
        .collect();
    assert!(missing.is_empty(), "calls missing from the CST record: {missing:?}\n  lexer: {lexer_calls:?}\n  deep:  {deep_calls:?}");
    assert_eq!(import_set(&lexer), import_set(&deep), "imports");

    let lexer_names: BTreeSet<(String, u8)> = symbol_set(&lexer).into_iter().map(|(n, k, _)| (n, k)).collect();
    let deep_names: BTreeSet<(String, u8)> = symbol_set(&deep).into_iter().map(|(n, k, _)| (n, k)).collect();
    let lost: Vec<&(String, u8)> = lexer_names.iter().filter(|symbol| !deep_names.contains(*symbol)).collect();
    assert!(lost.is_empty(), "symbols missing from the CST record: {lost:?}");

    // And each construct by name, so a failure says which one.
    let calls: BTreeSet<String> = deep_calls.iter().map(|call| call.split('@').next().unwrap_or(call).to_string()).collect();
    for expected in [
        "alpha",
        "beta",
        "gamma",
        "min",
        "len",
        "delta",
        "os.environ.get",
        "k.lower",
        "items",
        "make",
        "handler",
    ] {
        assert!(calls.iter().any(|call| call == expected || call.ends_with(&format!(".{expected}")) || call == &format!(".{expected}")), "no {expected} call in {calls:?}");
    }
    assert!(deep_names.contains(&("handler".to_string(), facts::KIND_FUNCTION)), "an assigned lambda is a definition: {deep_names:?}");
}

#[test]
#[ignore = "reads a tree outside this repository; set FACT_LOSS_DIR"]
fn fact_loss_over_a_real_tree() {    let root = std::env::var_os("FACT_LOSS_DIR").map(PathBuf::from).unwrap_or_else(default_root);
    assert!(root.is_dir(), "no tree at {}", root.display());
    let (lexer, deep, tally, lossy, parsed) = walk_root(&root);
    println!("\n== {} ==", root.display());
    println!("parsed {parsed} files, {lossy} with a loss");
    println!(
        "lexer: symbols {} calls {} imports {}",
        lexer.symbols, lexer.calls, lexer.imports
    );
    println!(
        "deep:  symbols {} calls {} imports {}   (delta {} / {} / {})",
        deep.symbols,
        deep.calls,
        deep.imports,
        deep.symbols as i64 - lexer.symbols as i64,
        deep.calls as i64 - lexer.calls as i64,
        deep.imports as i64 - lexer.imports as i64,
    );
    println!(
        "lost {} facts: grammar {} mapping {} resolution {} (accepted {} contract differences)",
        tally.total(),
        tally.grammar,
        tally.mapping,
        tally.resolution,
        tally.accepted,
    );
    println!(
        "guard: {} files would fall back (interpolation calls {}, lexer-phantom calls {}, events {}, other {})",
        tally.guard_files,
        tally.guard_interpolation,
        tally.guard_phantom,
        tally.guard_event,
        tally.guard_other,
    );
}
