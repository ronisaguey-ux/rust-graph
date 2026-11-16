//! Go: `go` and `defer` launch and postpone a call; they are not callees.
//!
//! The lexer reads the statement model of Go by hand, and two words in it are
//! easy to get wrong in opposite directions: `go serve(addr)` must record the
//! call `serve` and no call `go`, and `defer f.Close()` must record
//! `f.Close` and no call `defer`. The CST keeps both as `go_statement` /
//! `defer_statement` over a call expression, so the reader must write the same
//! record the lexer writes — keyword events included, because a resolver reads
//! them as the edges they are.
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
    let language = langs::by_content("go", source.as_bytes()).expect("a Go row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Go).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Go)
}

/// One row per shape the two words take. Each source is the smallest file that
/// writes the statement.
const COVERAGE: &[(&str, &str)] = &[
    (
        "a go statement with a plain call",
        "package p\n\nfunc start(addr string) {\n\tgo serve(addr)\n}\n",
    ),
    (
        "a defer statement with a method call",
        "package p\n\nfunc withFile(path string) {\n\tf, err := open(path)\n\tdefer f.Close()\n\t_ = err\n}\n",
    ),
    (
        "a go statement over a function literal",
        "package p\n\nfunc spawn(jobs int) {\n\tgo func() {\n\t\twork(jobs)\n\t}()\n}\n",
    ),
    (
        "a defer statement over a function literal",
        "package p\n\nfunc guarded() {\n\tdefer func() {\n\t\trecover()\n\t}()\n}\n",
    ),
    (
        "go and defer in the same function",
        "package p\n\nfunc serveAll(addrs []string, w io.Writer) {\n\tdefer w.Flush()\n\tfor _, addr := range addrs {\n\t\tgo serve(addr)\n\t}\n}\n",
    ),
];

/// Shapes that still disagree.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];

#[test]
fn every_go_and_defer_shape_writes_the_same_record() {
    let mut failed: Vec<String> = Vec::new();
    for (name, source) in COVERAGE {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        let verdict = pipeline::loses(&lexer, &deep);
        if verdict.lost() {
            failed.push(format!("{name}: {verdict:?}"));
        }
    }
    assert!(failed.is_empty(), "go/defer disagrees with the lexer:\n{}", failed.join("\n"));
}

#[test]
fn the_known_go_gaps_are_still_gaps() {
    for (name, source, disagreement) in KNOWN_GAPS {
        let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
        assert!(verdict.lost(), "{name} now reads cleanly — move it into COVERAGE: {disagreement}");
    }
}
