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
