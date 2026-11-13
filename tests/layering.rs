//! The two readers are independent by construction, and this test is the check
//! that keeps it that way.
//!
//! The one-pass lexer (`lexer`, `expr`, `parser`, `facts`, `langs`) is built
//! without the `deep` feature and must not reach into `deep` or
//! `deep_integration`; the CST reader (`deep/*`) is built without the lexer's
//! modules and must not reach back. A shared table, a shared cache or a shared
//! node type would make a fallback comparison meaningless — each side would be
//! scoring the other's work — so the rule is a source-level one: the names do
//! not appear at all.
//!
//! This walks the tree instead of `include_str!` so a new module is covered the
//! day it is added, which is the failure mode a hand-written list has.

use std::fs;
use std::path::{Path, PathBuf};

const LEXER_NAMES: [&str; 4] = ["deep::", "deep_integration", "deep_facts", "crate::deep"];
const DEEP_NAMES: [&str; 7] = ["crate::lexer", "crate::expr", "crate::parser", "crate::facts", "crate::langs", "deep_facts::", "deep_integration::"];

fn rust_files(under: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(under) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

/// The code of one file, stripped of comments: a mention inside prose is not a
/// dependency, and the readers' docs are full of each other's names.
fn code_only(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn hits(text: &str, needles: &[&'static str]) -> Vec<&'static str> {
    let code = code_only(text);
    needles.iter().copied().filter(|needle| code.contains(needle)).collect()
}

#[test]
fn the_detector_sees_an_import() {
    assert_eq!(hits("use crate::deep::syntax;\n", &LEXER_NAMES), ["deep::", "crate::deep"]);
    assert_eq!(hits("use deep::tree::Node;\n", &LEXER_NAMES), ["deep::"]);
    assert!(hits("// deep:: is named in a comment only\n", &LEXER_NAMES).is_empty());
    assert!(hits("use crate::facts::Facts;\n", &DEEP_NAMES).iter().any(|hit| *hit == "crate::facts"));
}

#[test]
fn the_two_readers_do_not_reach_into_each_other() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = root.join("src");

    let mut lexer_side = Vec::new();
    for name in ["lexer.rs", "expr.rs", "parser.rs", "facts.rs", "langs.rs"] {
        lexer_side.push(src.join(name));
    }
    let mut deep_side = Vec::new();
    rust_files(&src.join("deep"), &mut deep_side);

    let mut failures = Vec::new();
    for path in &lexer_side {
        let text = fs::read_to_string(path).unwrap_or_default();
        for hit in hits(&text, &LEXER_NAMES) {
            failures.push(format!("{} names {hit}", path.display()));
        }
    }
    for path in &deep_side {
        let text = fs::read_to_string(path).unwrap_or_default();
        for hit in hits(&text, &DEEP_NAMES) {
            failures.push(format!("{} names {hit}", path.display()));
        }
    }

    assert!(failures.is_empty(), "the readers are no longer independent:\n{}", failures.join("\n"));
}
