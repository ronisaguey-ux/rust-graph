//! Kotlin `file` soft keyword in calls, receivers, imports, and script heads.
//!
//! `file` is a soft keyword for `@file:...` use-site annotations, and an
//! ordinary identifier everywhere else: `file("x")`, `outputs.file(x)`,
//! `file.exists()`, and `import java.nio.file.*`.

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

fn call_list(facts: &Facts) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for symbol in &facts.symbols {
        for call in &symbol.calls {
            out.push((call.path.clone(), call.line));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn both(source: &str) -> (Vec<(String, u32)>, Vec<(String, u32)>) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (call_list(&lexer), call_list(&deep))
}

const ROWS: &[(&str, &str)] = &[
    ("top-level script file call", "file(\"node_modules\")\n"),
    ("dotted file call", "outputs.file(outputFile)\n"),
    ("nested file call", "layout.buildDirectory.file(\"x.jar\")\n"),
    ("file receiver call in function", "fun f(file: java.io.File): Boolean {\n    return file.exists()\n}\n"),
    ("file annotation and file call", "@file:JvmName(\"Test\")\nfun f() {\n    outputs.file(\"a\")\n}\n"),
];

#[test]
fn kotlin_file_calls_agree() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source) in ROWS {
        let (lexer, deep) = both(source);
        if lexer != deep {
            failures.push(format!("{label}:\n  lexer {lexer:?}\n  cst   {deep:?}"));
        }
    }
    assert!(failures.is_empty(), "kotlin file calls disagree:\n{}", failures.join("\n"));
}
