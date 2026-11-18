//! Swift implicit member expressions: `.utf8` in value position names its member.
//!
//! `self = .utf8` or `encoder.outputFormatting = .prettyPrinted` writes an
//! `implicit_member_expression` on the right-hand side. The CST walker already
//! named its member in callee and path positions, but `form` fell through to
//! the constant `0`, causing every assignment of an implicit enum/static
//! member to disagree with the one-pass lexer.

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

#[test]
fn swift_implicit_member_expression_names_member_in_value_position() {
    let source = "extension Encoding {\n    init?(iana: String) {\n        self = .utf8\n    }\n}\n";
    let language = langs::by_content("swift", source.as_bytes()).expect("a Swift row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Swift).expect("a Swift tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Swift);
    let want = "[\"n\",\"utf8\"]";
    assert!(events(&lexer).contains(want), "lexer missing {want}: {}", events(&lexer));
    assert!(events(&deep).contains(want), "deep missing {want}: {}", events(&deep));
}
