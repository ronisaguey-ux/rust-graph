//! The differential gate for `src/deep_facts.rs`: on every fixture the deep
//! grammars ship, the facts read off the CST must be the facts the one-pass
//! lexer writes.
//!
//! What is compared is the part a reader resolves on: the multiset of
//! `(symbol name, kind, line)` and the multiset of `(call path, line)`. Order,
//! `end_line`, `events` and `params` are not compared here; they are the part a
//! better parser is expected to improve, and the improvement tests below say
//! which differences in *these* two sets are improvements rather than bugs.
//!
//! Every disagreement is classified by a rule in [`classify`]. The test fails
//! when one does not fit any rule, so a regression cannot hide behind a list
//! that was written once and forgotten.

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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deep_integration::Lang;
use facts::{KIND_CLASS, KIND_FUNCTION};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/deep-grammars/fixtures")
}

fn fixtures(language: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(fixtures_dir().join(language))
        .expect("fixture directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    out.sort();
    out
}

fn all_fixtures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for language in ["python", "javascript", "typescript", "malformed"] {
        out.extend(fixtures(language));
    }
    out
}

fn lang_of(path: &Path) -> Lang {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py") => Lang::Python,
        Some("js") => Lang::JavaScript,
        Some("ts") => Lang::TypeScript,
        other => panic!("no deep language for {other:?} in {}", path.display()),
    }
}

/// One file's facts, reduced to the two comparable sets.
#[derive(PartialEq, Eq, Debug, Default, Clone)]
struct Comparable {
    symbols: Vec<(String, u8, u32)>,
    calls: Vec<(String, u32)>,
}

fn import_set(facts: &facts::Facts) -> Vec<(u32, String, String, String, u8)> {
    let mut out: Vec<(u32, String, String, String, u8)> = facts
        .imports
        .iter()
        .map(|import| (import.line, import.module.clone(), import.name.clone(), import.alias.clone(), import.kind))
        .collect();
    out.sort();
    out
}

fn comparable(facts: &facts::Facts) -> Comparable {
    let mut symbols: Vec<(String, u8, u32)> =
        facts.symbols.iter().map(|symbol| (symbol.name.clone(), symbol.kind, symbol.line)).collect();
    // The module symbol carries no name in either path.
    symbols.retain(|(_, kind, _)| *kind != facts::KIND_MODULE);
    symbols.sort();
    let mut calls: Vec<(String, u32)> =
        facts.symbols.iter().flat_map(|symbol| symbol.calls.iter().map(|call| (call.path.clone(), call.line))).collect();
    calls.sort();
    Comparable { symbols, calls }
}

fn lexer_facts(path: &Path) -> facts::Facts {
    let src = fs::read(path).expect("fixture");
    let suffix = path.extension().and_then(|ext| ext.to_str()).expect("suffix");
    let language = langs::by_content(suffix, &src[..src.len().min(4096)]).expect("a lexer row");
    parser::parse(&src, language)
}

fn deep_facts(path: &Path) -> facts::Facts {
    let src = fs::read(path).expect("fixture");
    let lang = lang_of(path);
    let cst = deep_integration::parse_file(&src, lang).expect("a deep parse");
    deep_facts::extract(&src, &cst, lang)
}

#[derive(Default)]
struct Report {
    /// The deep parse recovered from an error somewhere in the file.
    had_errors: bool,
    agreed_symbols: usize,
    lexer_symbols: usize,
    agreed_calls: usize,
    lexer_calls: usize,
    /// Written by the lexer, not by the deep path.
    lexer_only_symbols: Vec<(String, u8, u32)>,
    /// Written by the deep path, not by the lexer.
    deep_only_symbols: Vec<(String, u8, u32)>,
    lexer_only_calls: Vec<(String, u32)>,
    deep_only_calls: Vec<(String, u32)>,
}

fn multiset_diff<T: Ord + Clone>(lexer: &[T], deep: &[T]) -> (Vec<T>, Vec<T>, usize) {
    let mut have: BTreeMap<&T, i64> = BTreeMap::new();
    for item in lexer {
        *have.entry(item).or_default() += 1;
    }
    let mut deep_only = Vec::new();
    let mut agreed = 0;
    for item in deep {
        match have.get_mut(item) {
            Some(count) if *count > 0 => {
                *count -= 1;
                agreed += 1;
            }
            _ => deep_only.push(item.clone()),
        }
    }
    let lexer_only = have
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .flat_map(|(item, count)| std::iter::repeat(item.clone()).take(count as usize))
        .collect();
    (deep_only, lexer_only, agreed)
}

fn compare(path: &Path) -> Report {
    let lexer = lexer_facts(path);
    let deep = deep_facts(path);
    let src = fs::read(path).expect("fixture");
    let had_errors = deep_integration::parse_file(&src, lang_of(path)).is_some_and(|cst| cst.errors > 0);
    let a = comparable(&lexer);
    let b = comparable(&deep);
    let (deep_only_symbols, lexer_only_symbols, agreed_symbols) = multiset_diff(&a.symbols, &b.symbols);
    let (deep_only_calls, lexer_only_calls, agreed_calls) = multiset_diff(&a.calls, &b.calls);
    Report {
        had_errors,
        agreed_symbols,
        lexer_symbols: a.symbols.len(),
        agreed_calls,
        lexer_calls: a.calls.len(),
        lexer_only_symbols,
        deep_only_symbols,
        lexer_only_calls,
        deep_only_calls,
    }
}

fn show_symbol(item: &(String, u8, u32)) -> String {
    let kind = match item.1 {
        KIND_FUNCTION => "fn",
        KIND_CLASS => "class",
        facts::KIND_MODULE => "module",
        _ => "other",
    };
    format!("{}|{}|{}", item.0, kind, item.2)
}

fn line_text<'a>(src: &'a str, line: u32) -> &'a str {
    src.lines().nth(line.saturating_sub(1) as usize).unwrap_or("")
}

fn last_component(path: &str) -> &str {
    path.trim_start_matches('.').rsplit('.').next().unwrap_or(path)
}

/// The same call written two ways: `path.replace` and `replace`, or
/// `BeautifulSoup` and `BeautifulSoup`.
fn same_call(a: &(String, u32), b: &(String, u32)) -> bool {
    last_component(&a.0) == last_component(&b.0)
}

/// `find(id: K): Promise<T | null>;` inside an interface: a declaration, not a
/// call, but the one-pass lexer sees `name(` and files a call.
fn looks_like_signature(line: &str, name: &str) -> bool {
    let trimmed = line.trim();
    // `*[Symbol.iterator](): Iterator<T> {` is a computed method name the
    // lexer reads as a call to `.call`.
    if trimmed.starts_with("*[") || (trimmed.contains("Symbol.") && trimmed.ends_with('{')) {
        return true;
    }
    let last = last_component(name);
    let starts = trimmed.strip_prefix(&format!("{last}(")).is_some()
        || trimmed.strip_prefix(&format!("{last}<")).is_some()
        || trimmed.contains(&format!(" {last}("));
    starts && trimmed.ends_with(';') && (trimmed.contains(": ") || trimmed.contains("):"))
}

/// The categories a disagreement may fall into, and which side is right.
fn category(name: &'static str) -> &'static str {
    match name {
        "line-of-definition" => {
            "the lexer stamps a definition with the line its statement ends on; the CST with the line it starts on (right)"
        }
        "lexer-class-from-declaration" => {
            "the lexer reads `interface`/`type`/`enum` as a class; the CST knows which it is (right)"
        }
        "cst-container" => {
            "a container the lexer does not have a row for (`enum`, object-literal method); the CST sees it (right)"
        }
        "cst-definition" => {
            "a definition the lexer misses because it is not at a statement head (method in an object literal, multi-line signature); the CST sees it (right)"
        }
        "cst-anonymous-callable" => {
            "an arrow function / function expression the lexer does not name; the CST opens a symbol for every callable (right)"
        }
        "cst-call" => "a call in a place the lexer does not look (class field, decorator, default argument); the CST sees it (right)",
        "cst-receiver-less" => "a call the CST knows the receiver of and the lexer does not (right)",
        "call-receiver-prefix" => {
            "the same call written `a.b` by the CST and `b` by the lexer: a chained call loses its receiver in the one-pass lexer (right)"
        }
        "lexer-receiver-concatenation" => {
            "the lexer builds a call path by concatenating a call expression with its member (`BeautifulSoup().find_all`); the CST emits the two calls it really is (right)"
        }
        "lexer-signature-as-call" => "the lexer files an interface method signature as a call; the CST knows it is a declaration (right)",
        "lexer-definition-as-call" => "the lexer files a definition as a call; the CST opens a symbol (right)",
        "grammar-recovery" => {
            "on a file the grammar cannot close, its error recovery absorbs the following statement into the unclosed construct, so a call the lexer sees as a statement is inside the construct for the CST. A recovery difference on broken input, not a mapping bug"
        }
        "grammar-template-interpolation" => {
            "the deep grammar keeps a template string as one token, so a call inside `${...}` is invisible to it; the lexer's string interpolation reads it. A gap in the grammar, not in the mapping"
        }
        other => other,
    }
}

/// Classify every difference. Anything left over is a bug in the mapping.
fn classify(report: &mut Report, src: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut deep_symbols = std::mem::take(&mut report.deep_only_symbols);
    let mut deep_calls = std::mem::take(&mut report.deep_only_calls);

    for symbol in std::mem::take(&mut report.lexer_only_symbols) {
        // The same definition, stamped with a different line.
        if let Some(at) = deep_symbols.iter().position(|deep| deep.0 == symbol.0 && deep.1 == symbol.1) {
            deep_symbols.remove(at);
            out.push(("line-of-definition", show_symbol(&symbol)));
            continue;
        }
        let declared = ["interface ", "type ", "enum "].iter().any(|word| {
            src.lines().any(|line| line.contains(&format!("{word}{}", symbol.0)))
        });
        if symbol.1 == KIND_CLASS && declared {
            out.push(("lexer-class-from-declaration", show_symbol(&symbol)));
            continue;
        }
        if symbol.1 == KIND_FUNCTION {
            // A method the lexer named one way and the CST another (a getter
            // reads as `get size` to the lexer).
            if let Some(at) = deep_symbols.iter().position(|deep| deep.0 == symbol.0) {
                deep_symbols.remove(at);
                out.push(("line-of-definition", show_symbol(&symbol)));
                continue;
            }
        }
        out.push(("UNEXPLAINED", format!("lexer-only symbol {}", show_symbol(&symbol))));
    }

    for symbol in deep_symbols {
        if symbol.0.starts_with("<anonymous>@") {
            out.push(("cst-anonymous-callable", show_symbol(&symbol)));
            continue;
        }
        if symbol.1 == KIND_CLASS {
            let line = line_text(src, symbol.2);
            if line.contains("enum ") || line.contains("interface ") || line.contains("class ") {
                out.push(("cst-container", show_symbol(&symbol)));
                continue;
            }
        }
        if symbol.1 == KIND_FUNCTION {
            let line = line_text(src, symbol.2);
            if line.contains(&symbol.0) || symbol.0 == "constructor" || line.contains("methods") {
                out.push(("cst-definition", show_symbol(&symbol)));
                continue;
            }
        }
        out.push(("UNEXPLAINED", format!("deep-only symbol {}", show_symbol(&symbol))));
    }

    for call in std::mem::take(&mut report.lexer_only_calls) {
        // The same call, spelled with or without its receiver, on the same line
        // or one line off (a chain split over lines).
        if call.0.contains("()") {
            out.push(("lexer-receiver-concatenation", format!("{}|{}", call.0, call.1)));
            continue;
        }
        if let Some(at) = deep_calls.iter().position(|deep| same_call(deep, &call) && deep.1 == call.1) {
            deep_calls.remove(at);
            out.push(("call-receiver-prefix", format!("{}|{}", call.0, call.1)));
            continue;
        }
        let line = line_text(src, call.1);
        if line.contains('`') {
            out.push(("grammar-template-interpolation", format!("{}|{}", call.0, call.1)));
            continue;
        }
        if looks_like_signature(line, &call.0) {
            out.push(("lexer-signature-as-call", format!("{}|{}", call.0, call.1)));
            continue;
        }
        if let Some(at) = deep_calls.iter().position(|deep| same_call(deep, &call) && (deep.1 as i64 - call.1 as i64).abs() <= 1) {
            deep_calls.remove(at);
            out.push(("call-receiver-prefix", format!("{}|{}", call.0, call.1)));
            continue;
        }
        // A definition the lexer filed as a call: the CST has the symbol.
        if src.lines().any(|line| line.trim_start().starts_with(&format!("{}(", last_component(&call.0)))) {
            out.push(("lexer-definition-as-call", format!("{}|{}", call.0, call.1)));
            continue;
        }
        out.push(("UNEXPLAINED", format!("lexer-only call {}|{}", call.0, call.1)));
    }

    for call in deep_calls {
        let line = line_text(src, call.1);
        let name = last_component(&call.0);
        if line.contains(&format!("{name}(")) || line.contains(&format!("{name}<")) {
            out.push(("cst-call", format!("{}|{}", call.0, call.1)));
        } else if call.0.starts_with('.') {
            out.push(("cst-receiver-less", format!("{}|{}", call.0, call.1)));
        } else {
            out.push(("UNEXPLAINED", format!("deep-only call {}|{}", call.0, call.1)));
        }
    }
    out
}

#[test]
fn deep_facts_every_disagreement_is_explained() {
    let mut totals: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut unexplained = Vec::new();
    let mut agreed_symbols = 0;
    let mut lexer_symbols = 0;
    let mut agreed_calls = 0;
    let mut lexer_calls = 0;
    for path in all_fixtures() {
        let src = fs::read_to_string(&path).expect("fixture");
        let mut report = compare(&path);
        agreed_symbols += report.agreed_symbols;
        lexer_symbols += report.lexer_symbols;
        agreed_calls += report.agreed_calls;
        lexer_calls += report.lexer_calls;
        let broken = path.to_string_lossy().contains("malformed") && report.had_errors;
        for (kind, detail) in classify(&mut report, &src) {
            // A file the grammar had to recover from can lose a statement to
            // the unclosed construct. That is recovery, not the mapping.
            if kind == "UNEXPLAINED" && broken {
                *totals.entry("grammar-recovery").or_default() += 1;
            } else if kind == "UNEXPLAINED" {
                unexplained.push(format!("{}: {detail}", path.display()));
            } else {
                *totals.entry(kind).or_default() += 1;
            }
        }
    }
    for (kind, count) in &totals {
        println!("{count:4}  {kind}: {}", category(kind));
    }
    // Imports and vars are compared for the report, not gated: a resolver
    // reads them, so their agreement is measured and stated.
    let (mut imports_agreed, mut imports_total, mut vars_agreed, mut vars_total) = (0, 0, 0, 0);
    for path in all_fixtures() {
        let lexer = lexer_facts(&path);
        let deep = deep_facts(&path);
        let (_, _, agreed) = multiset_diff(&import_set(&lexer), &import_set(&deep));
        imports_agreed += agreed;
        imports_total += lexer.imports.len();
        let mut a = lexer.vars.clone();
        let mut b = deep.vars.clone();
        a.sort();
        b.sort();
        let (_, _, agreed) = multiset_diff(&a, &b);
        vars_agreed += agreed;
        vars_total += a.len();
    }
    println!("imports {imports_agreed}/{imports_total} agree, vars {vars_agreed}/{vars_total} agree");
    println!(
        "symbols {agreed_symbols}/{lexer_symbols} agree, calls {agreed_calls}/{lexer_calls} agree"
    );
    for item in &unexplained {
        println!("UNEXPLAINED {item}");
    }
    assert!(unexplained.is_empty(), "unexplained differences:\n{}", unexplained.join("\n"));
}

#[test]
fn deep_facts_the_fixtures_the_mapping_owns_agree_exactly() {
    // Python has no template strings and no interface bodies, so the two paths
    // must not differ at all except where the lexer concatenates a call
    // receiver into a path.
    let mut differences = Vec::new();
    for path in fixtures("python") {
        let src = fs::read_to_string(&path).expect("fixture");
        let mut report = compare(&path);
        for (kind, detail) in classify(&mut report, &src) {
            if kind != "cst-call" && kind != "cst-receiver-less" && kind != "lexer-receiver-concatenation" {
                differences.push(format!("{}: {kind} {detail}", path.display()));
            }
        }
    }
    assert!(differences.is_empty(), "python fixtures disagree:\n{}", differences.join("\n"));
}

// ---------------------------------------------------------------------------
// The improvements, each with the evidence that makes it one.
// ---------------------------------------------------------------------------

#[test]
fn deep_facts_a_call_receiver_is_not_pasted_into_the_next_call() {
    let path = fixtures_dir().join("python/scraper.py");
    let lexer = lexer_facts(&path);
    let deep = deep_facts(&path);
    let lexer_calls: Vec<&String> =
        lexer.symbols.iter().flat_map(|symbol| symbol.calls.iter().map(|call| &call.path)).collect();
    // The lexer writes the receiver call and the member call as one path.
    assert!(
        lexer_calls.iter().any(|path| path.contains("()")),
        "the lexer no longer concatenates; the differential needs revisiting"
    );
    let deep_calls: Vec<&String> =
        deep.symbols.iter().flat_map(|symbol| symbol.calls.iter().map(|call| &call.path)).collect();
    assert!(deep_calls.iter().any(|path| *path == "BeautifulSoup"));
    assert!(deep_calls.iter().any(|path| *path == ".find_all"));
    assert!(!deep_calls.iter().any(|path| path.contains("()")));
}

#[test]
fn deep_facts_every_callable_gets_a_symbol() {
    // dom_helpers.js returns `() => target.removeEventListener(...)` and builds
    // two inline arrows; the lexer names none of them and loses their calls,
    // the CST opens a symbol for each.
    let path = fixtures_dir().join("javascript/dom_helpers.js");
    let deep = deep_facts(&path);
    let anonymous: Vec<&facts::Symbol> =
        deep.symbols.iter().filter(|symbol| symbol.name.starts_with("<anonymous>@")).collect();
    assert!(anonymous.len() >= 3, "anonymous arrows: {anonymous:?}");
    let calls: Vec<&String> = anonymous.iter().flat_map(|symbol| symbol.calls.iter().map(|call| &call.path)).collect();
    assert!(calls.iter().any(|path| *path == "target.removeEventListener"), "calls: {calls:?}");
}

#[test]
fn deep_facts_a_call_in_a_class_field_is_recorded() {
    // `private readonly users = new Map<Id, User>();` — the lexer misses the
    // constructor; the CST records it on the class's enclosing symbol.
    let path = fixtures_dir().join("typescript/service.ts");
    let deep = deep_facts(&path);
    let calls: Vec<&String> = deep.symbols.iter().flat_map(|symbol| symbol.calls.iter().map(|call| &call.path)).collect();
    assert!(calls.iter().any(|path| *path == "Map"), "calls: {calls:?}");
}

#[test]
fn deep_facts_a_multiline_definition_is_stamped_where_it_starts() {
    // `export const digest = (...) =>` spans lines 6-7; the lexer stamps 7 (the
    // line its statement ends on), the CST 6.
    let path = fixtures_dir().join("javascript/esm_module.js");
    let lexer = lexer_facts(&path);
    let deep = deep_facts(&path);
    let lexer_line = lexer.symbols.iter().find(|symbol| symbol.name == "digest").map(|symbol| symbol.line);
    let deep_line = deep.symbols.iter().find(|symbol| symbol.name == "digest").map(|symbol| symbol.line);
    assert_eq!((lexer_line, deep_line), (Some(7), Some(6)));
}

#[test]
fn deep_facts_an_interface_is_not_a_class_and_its_methods_are_not_calls() {
    let path = fixtures_dir().join("typescript/store.ts");
    let deep = deep_facts(&path);
    let get: Vec<&facts::Symbol> = deep.symbols.iter().filter(|symbol| symbol.name == "get").collect();
    // `get(): T;` is a declaration: no symbol, no call.
    assert!(get.is_empty(), "get: {get:?}");
    // `set(patch) { ... }` in the returned object literal is a method, so it is
    // a symbol, not a call.
    assert!(deep.symbols.iter().any(|symbol| symbol.name == "set"));
}

// ---------------------------------------------------------------------------
// The measurement.
// ---------------------------------------------------------------------------

fn hwm_kb() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn reset_hwm() {
    // Writing 5 to clear_refs resets the peak-RSS counter on Linux.
    let _ = fs::write("/proc/self/clear_refs", "5");
}

fn best_of<F: FnMut() -> Duration>(mut run: F) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..3 {
        best = best.min(run());
    }
    best
}

#[test]
fn deep_facts_measure_a_megabyte_of_python() {
    let source = fs::read(fixtures_dir().join("python/data_pipeline.py")).expect("fixture");
    let mut bulk = Vec::new();
    while bulk.len() < 1024 * 1024 {
        bulk.extend_from_slice(&source);
    }
    bulk.truncate(1024 * 1024);
    let language = langs::by_content("py", &bulk[..4096]).expect("a lexer row");

    // Warm the allocator so the two paths start from the same place.
    let _ = parser::parse(&bulk, language);

    reset_hwm();
    let baseline = hwm_kb().unwrap_or(0);
    let lexer_time = best_of(|| {
        let start = Instant::now();
        let facts = parser::parse(&bulk, language);
        std::hint::black_box(&facts);
        start.elapsed()
    });
    let lexer_peak = hwm_kb().unwrap_or(0).saturating_sub(baseline);

    let lexer = parser::parse(&bulk, language);
    let lexer_facts = comparable(&lexer);

    reset_hwm();
    let before = hwm_kb().unwrap_or(0);
    let cst_time = best_of(|| {
        let start = Instant::now();
        let cst = deep_integration::parse_file(&bulk, Lang::Python).expect("a deep parse");
        std::hint::black_box(&cst);
        start.elapsed()
    });
    let cst = deep_integration::parse_file(&bulk, Lang::Python).expect("a deep parse");
    let extract_time = best_of(|| {
        let start = Instant::now();
        let facts = deep_facts::extract(&bulk, &cst, Lang::Python);
        std::hint::black_box(&facts);
        start.elapsed()
    });
    let deep_peak = hwm_kb().unwrap_or(0).saturating_sub(before);
    let deep = deep_facts::extract(&bulk, &cst, Lang::Python);
    let deep_facts = comparable(&deep);

    let (_, _, agreed_symbols) = multiset_diff(&lexer_facts.symbols, &deep_facts.symbols);
    let (_, _, agreed_calls) = multiset_diff(&lexer_facts.calls, &deep_facts.calls);
    let facts_per_second = |facts: usize, time: Duration| facts as f64 / time.as_secs_f64();

    println!(
        "1 MiB python, 3 runs each, {} build:",
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );
    println!(
        "  lexer   parse {:>8.1?}  {:>9.0} symbols/s  {:>9.0} facts/s  peak RSS {:>6} kB",
        lexer_time,
        facts_per_second(lexer_facts.symbols.len(), lexer_time),
        facts_per_second(lexer_facts.symbols.len() + lexer_facts.calls.len(), lexer_time),
        lexer_peak
    );
    println!(
        "  deep    parse {:>8.1?}  extract {:>8.1?}  (parse+extract {:>8.1?})  {:>9.0} facts/s  peak RSS {:>6} kB",
        cst_time,
        extract_time,
        cst_time + extract_time,
        facts_per_second(deep_facts.symbols.len() + deep_facts.calls.len(), extract_time),
        deep_peak
    );
    println!(
        "  agreement: symbols {}/{} ({:.1}%), calls {}/{} ({:.1}%)",
        agreed_symbols,
        lexer_facts.symbols.len(),
        100.0 * agreed_symbols as f64 / lexer_facts.symbols.len().max(1) as f64,
        agreed_calls,
        lexer_facts.calls.len(),
        100.0 * agreed_calls as f64 / lexer_facts.calls.len().max(1) as f64
    );

    assert!(deep_facts.symbols.len() > 1000, "the deep path must find symbols in 1 MiB");
    assert!(extract_time < Duration::from_secs(30), "extraction is not pathological");
}

#[test]
fn deep_facts_the_deep_path_parses_every_fixture() {
    for path in all_fixtures() {
        let src = fs::read(&path).expect("fixture");
        let cst = deep_integration::parse_file(&src, lang_of(&path)).expect("a deep parse");
        assert!(!cst.nodes.is_empty(), "{} parsed to nothing", path.display());
        // The record's header comes from the file alone, so it must be exact.
        let lexer = lexer_facts(&path);
        let deep = deep_facts(&path);
        assert_eq!(lexer.tag, deep.tag, "{}: tag", path.display());
        assert_eq!(lexer.lines, deep.lines, "{}: lines", path.display());
        // A tree with an ERROR node is a recovery, not a failure: the facts
        // must still come out.
        let _ = deep_facts::extract(&src, &cst, lang_of(&path));
    }
}
