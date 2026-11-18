//! A Kotlin star import ends at the newline.
//!
//! `import kotlin.test.*` closes with a `*`, which reads as an operator, so a
//! reader that treats a trailing operator as a continuation folded the class
//! below it into the import statement. The class then lost its own symbol and
//! its supertype constructor was filed as a function — 30 rows in the kotlin
//! corpus, every one of them a `class X : Base() {` under a wildcard import.
//!
//! What is pinned is that both readers give the class a class symbol and give
//! the supertype none of its own.

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
use facts::{Facts, KIND_CLASS};

fn symbols(facts: &Facts) -> Vec<(String, u8)> {
    facts.symbols.iter().map(|symbol| (symbol.name.clone(), symbol.kind)).collect()
}

fn both(source: &str) -> (Vec<(String, u8)>, Vec<(String, u8)>) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (symbols(&lexer), symbols(&deep))
}

const IMPORTED: &str = "import kotlin.test.*\n\nclass PathTest : AbstractPathTest() {\n    private val x = 1\n}\n";

#[test]
fn kotlin_star_import_does_not_swallow_the_class_below_it() {
    let (lexer, deep) = both(IMPORTED);
    for (side, rows) in [("the lexer", &lexer), ("the cst", &deep)] {
        assert!(
            rows.iter().any(|(name, kind)| name == "PathTest" && *kind == KIND_CLASS),
            "{side} lost the class symbol: {rows:?}"
        );
        assert!(
            !rows.iter().any(|(name, _)| name == "AbstractPathTest"),
            "{side} gave the supertype a symbol of its own: {rows:?}"
        );
    }
}

const UNIMPORTED: &str = "class PathTest : AbstractPathTest() {\n    private val x = 1\n}\n";

#[test]
fn kotlin_class_without_an_import_above_it_reads_the_same() {
    let (lexer, deep) = both(UNIMPORTED);
    assert_eq!(
        lexer.iter().filter(|(_, kind)| *kind == KIND_CLASS).count(),
        1,
        "the lexer does not agree with itself: {lexer:?}"
    );
    assert!(
        deep.iter().any(|(name, kind)| name == "PathTest" && *kind == KIND_CLASS),
        "the cst lost the class symbol: {deep:?}"
    );
}
