//! Kotlin imports: a dotted path whose middle is a contextual keyword.
//!
//! Kotlin's grammar names 39 contextual keywords as names (`annotation`, `field`,
//! `value`, `internal`, `open`, …), and they appear inside package paths in real
//! code — `java.lang.annotation.Inherited` is the one the corpus is full of.
//! The walk read only `identifier` nodes, so that segment vanished and the record
//! said `java.lang.Inherited`: 3,882 rows of the kotlin corpus, the language's
//! single largest loss class, and the reason an annotated file fell back even
//! after its annotations parsed.
//!
//! Each row asserts the two readers file the same import, because that equality
//! is what the guard decides a fallback on.
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
use facts::{Facts, Import};

fn import_list(facts: &Facts) -> Vec<(String, String, String, u8)> {
    facts
        .imports
        .iter()
        .map(|import: &Import| (import.module.clone(), import.name.clone(), import.alias.clone(), import.kind))
        .collect()
}

/// The lexer's record and the CST record for one snippet.
fn both(source: &str) -> (Vec<(String, String, String, u8)>, Vec<(String, String, String, u8)>) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (import_list(&lexer), import_list(&deep))
}

const ROWS: &[(&str, &str)] = &[
    (
        "a contextual keyword inside the path",
        "import java.lang.annotation.Inherited\n\nclass A\n",
    ),
    (
        "two contextual keywords",
        "import com.example.internal.field.Value\n\nclass A\n",
    ),
    ("a plain path", "import org.jetbrains.kotlin.compiler.plugin\n\nclass A\n"),
    (
        "an alias after the path",
        "import org.jetbrains.kotlin.compiler.plugin.CompilerPluginRegistrar as Registrar\n\nclass A\n",
    ),
    ("a star import", "import kotlin.math.*\n\nclass A\n"),
    (
        "a star import whose path names a keyword",
        "import com.example.annotation.*\n\nclass A\n",
    ),
];

#[test]
fn kotlin_both_readers_file_the_same_import() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source) in ROWS {
        let (lexer, deep) = both(source);
        if lexer != deep {
            failures.push(format!("{label}:\n  lexer {lexer:?}\n  cst   {deep:?}"));
        }
    }
    assert!(failures.is_empty(), "kotlin imports disagree:\n{}", failures.join("\n"));
}
