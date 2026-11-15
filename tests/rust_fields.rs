//! P5-16: the shapes this repository's own sources are written in.
//!
//! The workspace's `src` tree is the first corpus that has to parse clean: a
//! single fallback there means the product cannot read the language it is built
//! in. Each row below is a line that was falling back — a tuple field, a prefix
//! range as a call's argument, and a struct update whose base is a call.
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

use deep_integration::Lang;
use facts::Facts;
use pipeline::gate::*;

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// `(what the line is, source)`.
const ROWS: &[(&str, &str)] = &[
    (
        "a tuple field read through a chain",
        "fn f(want: &Want) {\n    let lexer_names = name_list(&want.1.targets);\n}\n",
    ),
    (
        "a prefix range as a call's argument",
        "fn f(stmt: &[u8], label: usize) {\n    stmt.drain(..label);\n}\n",
    ),
    (
        "a prefix range as a subscript's bound",
        "fn g(xs: &[u8]) -> &[u8] {\n    &xs[..2]\n}\n",
    ),
    (
        "a struct update whose base is a call",
        "fn h() -> RawResult {\n    let v = RawResult {\n        selector: Some(\"#hero\".into()),\n        ..RawResult::new(origin::VISUAL, \"low\".into(), format!(\"copy {n}\"))\n    };\n    v\n}\n",
    ),
    (
        "a tuple field as a call's receiver",
        "fn k(rows: &[(u8, u8)]) {\n    let first = rows.0.1;\n    use_it(first);\n}\n",
    ),
];

#[test]
fn the_workspaces_own_shapes_do_not_fall_back() {
    let mut problems = Vec::new();
    for (what, source) in ROWS {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        let verdict = pipeline::loses_in(&lexer, &deep, source);
        if verdict.lost() {
            problems.push(format!("{what}: {verdict:?}"));
        }
    }
    assert!(problems.is_empty(), "a shape this repository is written in still falls back:\n{}", problems.join("\n"));
}

#[test]
fn the_workspaces_own_tree_parses_without_a_fallback() {
    // The rule, not a sample: every `.rs` file this repository ships is read by
    // the CST with nothing left over. A new construct that only the lexer can
    // read fails here before it can reach a commit.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let mut checked = 0usize;
    let mut failed = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let Ok(source) = std::fs::read_to_string(&path) else { continue };
                checked += 1;
                let verdict = pipeline::loses_in(&lexer_facts(&source), &deep_facts(&source), &source);
                if verdict.lost() {
                    failed.push(format!("{}: {verdict:?}", path.display()));
                }
            }
        }
    }
    assert!(checked > 20, "the walk found {checked} files, which is not this tree");
    assert!(failed.is_empty(), "the workspace's own sources fall back:\n{}", failed.join("\n"));
}
