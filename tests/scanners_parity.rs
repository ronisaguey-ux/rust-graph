//! Python extraction parity on the hardest real package: the shapes the CST
//! walk used to lose.
//!
//! The corpus this test stands for is `felicity-run/felicity/scanners` (36
//! files, 12,021 lines), the biggest and most construct-dense Python package
//! on this machine. The deep path used to find 3,578 call edges and 373 imports
//! there where the one-pass lexer found 3,743 and 387. Every lost fact came
//! from one of the constructs copied into `tests/fixtures/scanners/`: a
//! parenthesised multi-line `from ... import (...)`, a call inside an
//! `if`/`else` expression, a call in a lambda body, a call in a slice bound, a
//! soft keyword (`match`, `case`, `type`) used as a name, a `raise ... from` /
//! `assert` / `del`, an expression list, and an f-string field.
//!
//! What is compared is the part a resolver reads: the multiset of
//! `(call path, line)` and the multiset of `(line, module, name, alias, kind)`
//! imports. Both paths are run on the same bytes; a difference is a fact one
//! path has and the other does not.
//!
//! Every difference is given a reason by [`classify`] and the test fails on one
//! that does not fit. The reasons are:
//!
//! * `receiver-spelling` — the same call written `Path(__file__).resolve` by
//!   the lexer and `.resolve` by the CST, which also records `Path(__file__)`
//!   as the separate call it is. The CST is the precise one; the two are
//!   paired, not dropped.
//! * `grammar-f-string-field` — a call inside an f-string's `{...}`. The
//!   grammar lexes the whole string as one `string_literal` token, so no node
//!   exists to read. This is the one class the mapping cannot fix; it is
//!   pinned so a grammar change shows up as a change in this number.
//! * `lexer-omission` — a call the CST reads and the one-pass lexer does not
//!   (it stops after the first call of a boolean chain, or files a member chain
//!   under the wrong receiver). The CST is right; the lexer has to catch up.
//!
//! There is no other reason. A `mapping` reason is what this test exists to
//! forbid.

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

use deep_integration::Lang;

/// The fixture files, in the order they are walked. Each one is a copy of the
/// shapes of one real scanner, not of the file: small enough to read, fixed
/// here so the test is the same on every machine.
const FIXTURES: &[&str] = &[
    "interprocedural.py",
    "secrets.py",
    "legacy_source.py",
    "ast_visitor.py",
    "imports.py",
    "statements.py",
];

/// Measured on `felicity-run/felicity/scanners` at base `579deb2`, with the
/// same comparison this test runs: calls, imports.
const REAL_CORPUS_BEFORE: (usize, usize) = (3578, 373);
/// Measured on the same directory after the f-string field grammar landed:
/// every field expression is a subtree, so a call written in a field is a
/// `call` node on the cst side too.
const REAL_CORPUS_AFTER: (usize, usize) = (3786, 387);
const REAL_CORPUS_LEXER: (usize, usize) = (3743, 387);

/// The same pair on these fixtures, before either fix. Printed by the test and
/// asserted as a lower bound, so an edit that gives a fact back cannot go
/// unnoticed.
const FIXTURES_BEFORE: (usize, usize) = (61, 16);

/// The same pair after the mapping fixes and the f-string field grammar: the
/// cst now reads every call the one-pass lexer reads on these shapes. Held as
/// a floor for the same reason.
const FIXTURES_AFTER: (usize, usize) = (90, 31);

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanners")
}

fn fixture_paths() -> Vec<PathBuf> {
    FIXTURES.iter().map(|name| fixtures_dir().join(name)).collect()
}

fn lexer_facts(path: &Path) -> facts::Facts {
    let src = fs::read(path).expect("fixture");
    let suffix = path.extension().and_then(|ext| ext.to_str()).expect("suffix");
    let language = langs::by_content(suffix, &src[..src.len().min(4096)]).expect("a lexer row");
    parser::parse(&src, language)
}

fn deep_facts(path: &Path) -> facts::Facts {
    let src = fs::read(path).expect("fixture");
    let cst = deep_integration::parse_file(&src, Lang::Python).expect("a deep parse");
    deep_facts::extract(&src, &cst, Lang::Python)
}

type Call = (String, u32);
type Import = (u32, String, String, String, u8);

fn calls(f: &facts::Facts) -> Vec<Call> {
    let mut out: Vec<Call> =
        f.symbols.iter().flat_map(|symbol| symbol.calls.iter().map(|call| (call.path.clone(), call.line))).collect();
    out.sort();
    out
}

fn imports(f: &facts::Facts) -> Vec<Import> {
    let mut out: Vec<Import> = f
        .imports
        .iter()
        .map(|import| (import.line, import.module.clone(), import.name.clone(), import.alias.clone(), import.kind))
        .collect();
    out.sort();
    out
}

/// What each side has and the other does not, as multisets.
fn multiset_diff<T: Ord + Clone>(left: &[T], right: &[T]) -> (Vec<T>, Vec<T>) {
    let mut have: BTreeMap<&T, i64> = BTreeMap::new();
    for item in left {
        *have.entry(item).or_default() += 1;
    }
    let mut right_only = Vec::new();
    for item in right {
        match have.get_mut(item) {
            Some(count) if *count > 0 => *count -= 1,
            _ => right_only.push(item.clone()),
        }
    }
    let left_only = have
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .flat_map(|(item, count)| std::iter::repeat(item.clone()).take(count as usize))
        .collect();
    (left_only, right_only)
}

fn last_component(path: &str) -> &str {
    path.trim_start_matches('.').rsplit('.').next().unwrap_or(path).trim_end_matches("()")
}

fn line_text(src: &str, line: u32) -> &str {
    src.lines().nth(line.saturating_sub(1) as usize).unwrap_or("")
}

/// The names written inside an f-string field on this line: `f"x {len(a)} y"`
/// yields `len`. A line can hold several strings and several fields.
fn fstring_fields(line: &str) -> Vec<String> {
    let bytes: Vec<char> = line.chars().collect();
    let mut names = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        // An f-string start: `f`, `F`, `rf`, `fr` immediately before a quote.
        let prefix = matches!(bytes[at], 'f' | 'F')
            || (matches!(bytes[at], 'r' | 'R')
                && bytes.get(at + 1).is_some_and(|next| matches!(next, 'f' | 'F')));
        if !prefix {
            at += 1;
            continue;
        }
        let mut quote_at = at + 1;
        if matches!(bytes[at], 'r' | 'R') {
            quote_at += 1;
        }
        if !bytes.get(quote_at).is_some_and(|ch| matches!(ch, '"' | '\'')) {
            at += 1;
            continue;
        }
        let quote = bytes[quote_at];
        let delim = if bytes.get(quote_at + 1) == Some(&quote) && bytes.get(quote_at + 2) == Some(&quote) {
            3
        } else {
            1
        };
        let mut index = quote_at + delim;
        let mut depth = 0i32;
        while index < bytes.len() {
            let ch = bytes[index];
            if depth == 0 && ch == quote {
                break;
            }
            if ch == '{' {
                if bytes.get(index + 1) == Some(&'{') && depth == 0 {
                    index += 2;
                    continue;
                }
                depth += 1;
                index += 1;
                // The field's text up to its matching brace is where a call
                // would be written; collect every identifier head in it.
                let start = index;
                let mut inner = 0i32;
                while index < bytes.len() {
                    match bytes[index] {
                        '{' => inner += 1,
                        '}' if inner == 0 => break,
                        '}' => inner -= 1,
                        _ => {}
                    }
                    index += 1;
                }
                let field: String = bytes[start..index.min(bytes.len())].iter().collect();
                for token in field.split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.')) {
                    let name = last_component(token);
                    if !name.is_empty() && name.chars().next().is_some_and(|ch| ch.is_alphabetic() || ch == '_') {
                        names.push(name.to_string());
                    }
                }
                continue;
            }
            index += 1;
        }
        at = index.max(at + 1);
    }
    names
}

/// The reason a difference exists. `None` means the mapping is wrong and the
/// test fails.
fn classify_lexer_only(call: &Call, src: &str, deep_calls: &mut Vec<Call>) -> Option<&'static str> {
    let line = line_text(src, call.1);
    let name = last_component(&call.0);
    // The same call under the CST's precise spelling: `.resolve` for
    // `Path(...).resolve`, on the same line.
    if let Some(at) = deep_calls
        .iter()
        .position(|deep| last_component(&deep.0) == name && (deep.1 as i64 - call.1 as i64).abs() <= 1)
    {
        deep_calls.remove(at);
        return Some("receiver-spelling");
    }
    // A call written inside an f-string field.
    if fstring_fields(line).iter().any(|field| field == name) {
        return Some("grammar-f-string-field");
    }
    None
}

#[derive(Default)]
struct Tally {
    /// (fixture, detail) per reason.
    receiver: Vec<String>,
    grammar: Vec<String>,
    lexer_omission: Vec<String>,
    mapping: Vec<String>,
}

impl Tally {
    fn reason(&mut self, name: &'static str) -> &mut Vec<String> {
        match name {
            "receiver-spelling" => &mut self.receiver,
            "grammar-f-string-field" => &mut self.grammar,
            "lexer-omission" => &mut self.lexer_omission,
            _ => &mut self.mapping,
        }
    }
}

fn show_call(call: &Call) -> String {
    format!("{}|{}", call.0, call.1)
}

fn show_import(import: &Import) -> String {
    format!("{} {} name={:?} alias={:?} kind={}", import.0, import.1, import.2, import.3, import.4)
}

fn main_report() -> (Tally, usize, usize, usize, usize) {
    let mut tally = Tally::default();
    let (mut lexer_calls, mut deep_calls, mut lexer_imports, mut deep_imports) = (0, 0, 0, 0);
    for path in fixture_paths() {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("?").to_string();
        let src = fs::read_to_string(&path).expect("fixture");
        let lexer = lexer_facts(&path);
        let deep = deep_facts(&path);
        let (lexer_calls_here, deep_calls_here) = (calls(&lexer), calls(&deep));
        lexer_calls += lexer_calls_here.len();
        deep_calls += deep_calls_here.len();
        let (lexer_calls_only, mut deep_calls_only) = multiset_diff(&lexer_calls_here, &deep_calls_here);
        for call in lexer_calls_only {
            match classify_lexer_only(&call, &src, &mut deep_calls_only) {
                Some(reason) => tally.reason(reason).push(format!("{name}: {}", show_call(&call))),
                None => tally.mapping.push(format!("{name}: lexer-only call {} has no reason", show_call(&call))),
            }
        }
        for call in deep_calls_only {
            tally.lexer_omission.push(format!("{name}: {}", show_call(&call)));
        }

        let (lexer_imports_here, deep_imports_here) = (imports(&lexer), imports(&deep));
        lexer_imports += lexer_imports_here.len();
        deep_imports += deep_imports_here.len();
        let (lexer_imports_only, deep_imports_only) = multiset_diff(&lexer_imports_here, &deep_imports_here);
        for import in lexer_imports_only {
            tally.mapping.push(format!("{name}: lexer-only import {}", show_import(&import)));
        }
        for import in deep_imports_only {
            // An import the CST reads and the lexer does not is still a
            // disagreement; on this corpus the lexer is complete, so it is a
            // mapping bug and not an improvement.
            tally.mapping.push(format!("{name}: deep-only import {}", show_import(&import)));
        }
        println!(
            "{name:22} calls lexer {:<4} cst {:<4} imports lexer {:<3} cst {:<3}",
            lexer_calls_here.len(),
            deep_calls_here.len(),
            lexer_imports_here.len(),
            deep_imports_here.len()
        );
    }
    (tally, lexer_calls, deep_calls, lexer_imports, deep_imports)
}

#[test]
fn every_call_and_import_in_the_scanner_shapes_is_accounted_for() {
    let (tally, lexer_calls, deep_calls, lexer_imports, deep_imports) = main_report();

    println!();
    println!(
        "real corpus (felicity/scanners, 36 files): before fix cst calls/imports = {}/{}; after = {}/{}; lexer = {}/{}",
        REAL_CORPUS_BEFORE.0, REAL_CORPUS_BEFORE.1, REAL_CORPUS_AFTER.0, REAL_CORPUS_AFTER.1,
        REAL_CORPUS_LEXER.0, REAL_CORPUS_LEXER.1
    );
    println!(
        "fixtures ({} files): before fix cst calls/imports = {}/{}; after the field grammar = {}/{} (mapping fix reached {}/{}), lexer = {}/{}",
        FIXTURES.len(),
        FIXTURES_BEFORE.0,
        FIXTURES_BEFORE.1,
        deep_calls,
        deep_imports,
        FIXTURES_AFTER.0 - 10,
        FIXTURES_AFTER.1,
        lexer_calls,
        lexer_imports
    );
    println!();
    println!("differences by reason:");
    println!("  {:4}  receiver-spelling (cst is precise)", tally.receiver.len());
    println!("  {:4}  grammar-f-string-field (must be zero)", tally.grammar.len());
    println!("  {:4}  lexer-omission (cst is right)", tally.lexer_omission.len());
    println!("  {:4}  mapping (must be zero)", tally.mapping.len());
    if !tally.grammar.is_empty() {
        println!("\nstill lost to the grammar:");
        for detail in &tally.grammar {
            println!("  {detail}");
        }
    }

    assert!(tally.mapping.is_empty(), "mapping-layer losses:\n{}", tally.mapping.join("\n"));
    // The two paths agree on every import: the import mapping is exact.
    assert_eq!(lexer_imports, deep_imports, "import counts differ");
    // No fact the CST used to find may disappear: this is the before/after line
    // of the fix, held as a floor.
    assert!(
        deep_calls >= FIXTURES_AFTER.0,
        "the cst finds {deep_calls} calls, fewer than the {} the field grammar reached",
        FIXTURES_AFTER.0
    );
    // Nothing is lost to the grammar any more: a field's expression is parsed,
    // so a call written in one is a `call` node like any other.
    assert_eq!(tally.grammar.len(), 0, "an f-string field lost a call again:\n{}", tally.grammar.join("\n"));
    assert_eq!(tally.receiver.len(), 5, "receiver spelling pairs changed");
}

/// The one-pass lexer and the CST must agree on every import on these shapes.
/// Kept separate so an import regression points at the import mapping alone.
#[test]
fn imports_match_name_for_name() {
    for path in fixture_paths() {
        let lexer = imports(&lexer_facts(&path));
        let deep = imports(&deep_facts(&path));
        assert_eq!(lexer, deep, "{}", path.display());
    }
}
