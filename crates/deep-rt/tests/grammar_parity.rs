//! The shipped grammars, on the new runtime: one parser, byte for byte.
//!
//! `crates/deep-grammars` still compiles its runtime from `src/deep` by path. This
//! test takes the grammar *sources* unchanged (`python.rs`, `javascript.rs`) and
//! compiles them here against `deep-rt` instead, by making `crate::deep` — the
//! module every `grammar!` file imports — be this crate. Nothing in the grammar
//! files is edited: the same attribute, the same invocation, the same lexer, a
//! different runtime.
//!
//! Every fixture under `crates/deep-grammars/fixtures` is then parsed twice — once
//! by the old runtime, through the `deep-grammars` crate, and once by this one — and
//! the two must agree on all three of: the outcome, the named S-expression, and the
//! raw JSON tree (kinds, byte ranges, flags, token text). A fixture added for one
//! runtime is therefore a test for the other.
//!
//! The runtime port is a move, not a rewrite, so there is no allowance for a
//! difference anywhere. `UNIFICATION.md` records the differences that are expected
//! between the *engines* (descent versus GLR); this test is about the descent
//! engine's two homes.

#[allow(unused_imports)]
mod deep {
    //! What a `grammar!` grammar means by `crate::deep`, served by `deep-rt`.
    pub use deep_rt::{budget, lexer, pairs, parser, syntax, tree};
    pub use deep_rt::{Budget, Workspace};
}

#[path = "../../deep-grammars/src/python.rs"]
mod python;
#[path = "../../deep-grammars/src/javascript.rs"]
mod javascript;
#[path = "../../deep-grammars/src/typescript.rs"]
#[allow(unused_imports)]
mod typescript;

use std::fs;
use std::path::{Path, PathBuf};

use deep_rt::syntax::Language;
use deep_rt::{Budget, Workspace};

use javascript::{JAVASCRIPT, TSX, TYPESCRIPT};
use python::PYTHON;

/// The name a language is listed under, the old runtime's static and this
/// runtime's static, which compile from the same source file.
fn languages() -> Vec<(&'static str, &'static deep_grammars::deep::syntax::Language, &'static Language)> {
    vec![
        ("python", &deep_grammars::python::PYTHON, &PYTHON),
        ("javascript", &deep_grammars::javascript::JAVASCRIPT, &JAVASCRIPT),
        ("typescript", &deep_grammars::javascript::TYPESCRIPT, &TYPESCRIPT),
        ("tsx", &deep_grammars::javascript::TSX, &TSX),
    ]
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../deep-grammars/fixtures")
}

fn files(folder: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixtures().join(folder))
        .unwrap_or_else(|error| panic!("fixtures/{folder}: {error}"))
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

/// Everything a parse produced, in a form both runtimes can print: the named
/// S-expression, the raw JSON tree, the outcome's description, and every token by
/// kind name and byte range (trivia and layout tokens included).
type Rendered = (String, String, String, String);

/// `kind@start..end`, one per line: the lexer's own output, not just the tree.
fn token_dump(names: impl Iterator<Item = (&'static str, u32, u32)>) -> String {
    let mut out = String::new();
    for (kind, start, end) in names {
        out.push_str(kind);
        out.push('@');
        out.push_str(&start.to_string());
        out.push_str("..");
        out.push_str(&end.to_string());
        out.push('\n');
    }
    out
}

fn render_old(language: &deep_grammars::deep::syntax::Language, src: &[u8]) -> Rendered {
    let mut workspace = deep_grammars::deep::Workspace::default();
    let outcome = workspace.parse(src, language, &deep_grammars::deep::Budget::default()).unwrap();
    let mut json = String::new();
    workspace.tree.write_json(language, src, &mut json);
    let tokens = token_dump((0..workspace.tokens.len()).map(|at| (language.name(workspace.tokens.kind[at]), workspace.tokens.start(at), workspace.tokens.end[at])));
    (workspace.tree.sexp(language, false), json, outcome.describe(), tokens)
}

fn render_new(language: &Language, src: &[u8]) -> Rendered {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src, language, &Budget::default()).unwrap();
    let mut json = String::new();
    workspace.tree.write_json(language, src, &mut json);
    let tokens = token_dump((0..workspace.tokens.len()).map(|at| (language.name(workspace.tokens.kind[at]), workspace.tokens.start(at), workspace.tokens.end[at])));
    (workspace.tree.sexp(language, false), json, outcome.describe(), tokens)
}

/// One file, both runtimes, all three renderings equal.
fn compare_one(path: &Path, name: &str) {
    let (label, old, new) = languages()
        .into_iter()
        .find(|(label, _, _)| *label == name)
        .unwrap_or_else(|| panic!("no language {name}"));
    let src = fs::read(path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    assert!(!src.is_empty(), "{path:?}: an empty fixture proves nothing");
    let (want_tree, want_json, want_outcome, want_tokens) = render_old(old, &src);
    let (got_tree, got_json, got_outcome, got_tokens) = render_new(new, &src);
    let shown = || {
        let text = String::from_utf8_lossy(&src);
        let head = text.lines().take(3).collect::<Vec<_>>().join("\n");
        format!("{label} {}: {head}", path.file_name().unwrap().to_string_lossy())
    };
    assert_eq!(got_outcome, want_outcome, "outcome differs in {}\nold: {}\nnew: {}", shown(), want_outcome, got_outcome);
    assert_eq!(got_tree, want_tree, "named tree differs in {}\nold: {}\nnew: {}", shown(), want_tree, got_tree);
    assert_eq!(got_json, want_json, "raw tree differs in {}\nold: {}\nnew: {}", shown(), want_json, got_json);
    assert_eq!(got_tokens, want_tokens, "tokens differ in {}\nold:\n{}\nnew:\n{}", shown(), want_tokens, got_tokens);
}

fn compare_folder(folder: &str, name: &str) {
    let paths = files(folder);
    assert!(!paths.is_empty(), "fixtures/{folder} is empty");
    for path in paths {
        compare_one(&path, name);
    }
}

#[test]
fn python_fixtures_are_byte_identical() {
    compare_folder("python", "python");
}

#[test]
fn javascript_fixtures_are_byte_identical() {
    compare_folder("javascript", "javascript");
}

#[test]
fn typescript_fixtures_are_byte_identical() {
    compare_folder("typescript", "typescript");
}

#[test]
fn malformed_fixtures_are_byte_identical() {
    // This set mixes the three languages by extension.
    for path in files("malformed") {
        let name = path.to_string_lossy();
        let language = if name.ends_with(".py") {
            "python"
        } else if name.ends_with(".ts") {
            "typescript"
        } else {
            "javascript"
        };
        compare_one(&path, language);
    }
}

#[test]
fn the_included_grammars_are_the_shipped_ones() {
    // Same size, same root, same layout settings: these really are the crate's
    // sources, compiled once per runtime.
    assert_eq!(PYTHON.kinds.len(), deep_grammars::python::PYTHON.kinds.len());
    assert_eq!((PYTHON.name, PYTHON.name(PYTHON.root)), ("python", "module"));
    assert!(PYTHON.layout.is_some());
    assert_eq!(JAVASCRIPT.kinds.len(), deep_grammars::javascript::JAVASCRIPT.kinds.len());
    assert_eq!((JAVASCRIPT.name, JAVASCRIPT.name(JAVASCRIPT.root)), ("javascript", "program"));
    assert_eq!(TYPESCRIPT.kinds.len(), JAVASCRIPT.kinds.len());
    // None of them carries tables, so all four still parse by descent.
    assert!(PYTHON.tables.is_none() && JAVASCRIPT.tables.is_none());
    assert!(TSX.tables.is_none() && TYPESCRIPT.tables.is_none());
}
