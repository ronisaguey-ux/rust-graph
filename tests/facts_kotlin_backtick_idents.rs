//! Backticked Kotlin identifiers stay single tokens across both readers.
//!
//! Kotlin quotes keywords, hyphenated names, and arbitrary characters with
//! backticks: `` `in` ``, `` `java-library` ``, `` `for` ``, and
//! `` `implementation(npm(decamelize))` ``. Both the single-pass reader and
//! the CST reader must keep the quoted identifier as one name.

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

fn events(facts: &Facts) -> String {
    let mut out = String::new();
    for symbol in &facts.symbols {
        out.push_str(&symbol.events);
    }
    out
}

fn parse_both(suffix: &str, source: &str) -> (Facts, Facts) {
    let language = langs::by_content(suffix, source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (lexer, deep)
}

#[test]
fn kotlin_backticked_package_and_import() {
    let source = concat!(
        "package testProject.`new-mpp-cocoapods-template`.src.iosMain.kotlin\n",
        "import `producer-cinterop-library`.ProducerCLibraryStruct\n",
        "class A\n",
    );
    let (lexer, deep) = parse_both("kt", source);
    let lex_ev = events(&lexer);
    let deep_ev = events(&deep);
    assert!(
        lex_ev.contains("testProject.`new-mpp-cocoapods-template`.src.iosMain.kotlin"),
        "lexer: {lex_ev}"
    );
    assert!(
        deep_ev.contains("testProject.`new-mpp-cocoapods-template`.src.iosMain.kotlin"),
        "deep: {deep_ev}"
    );
    assert_eq!(lexer.imports.len(), 1, "lexer imports: {:?}", lexer.imports);
    assert_eq!(deep.imports.len(), 1, "deep imports: {:?}", deep.imports);
    assert_eq!(lexer.imports[0].module, "`producer-cinterop-library`.ProducerCLibraryStruct");
    assert_eq!(deep.imports[0].module, "`producer-cinterop-library`.ProducerCLibraryStruct");
}

#[test]
fn kotlin_backticked_member_and_script_block() {
    let source = concat!(
        "plugins {\n",
        "    `kotlin-dsl`\n",
        "}\n",
        "coreLibrariesVersion = libs.versions.kotlin.`for`.gradle.plugins.compilation.get()\n",
    );
    let (lexer, deep) = parse_both("kts", source);
    let lex_ev = events(&lexer);
    let deep_ev = events(&deep);
    assert!(lex_ev.contains("[\"n\",\"`kotlin-dsl`\"]"), "lexer: {lex_ev}");
    assert!(deep_ev.contains("[\"n\",\"`kotlin-dsl`\"]"), "deep: {deep_ev}");
    assert!(
        lex_ev.contains("libs.versions.kotlin.`for`.gradle.plugins.compilation"),
        "lexer: {lex_ev}"
    );
    assert!(
        deep_ev.contains("libs.versions.kotlin.`for`.gradle.plugins.compilation"),
        "deep: {deep_ev}"
    );
}
