//! Swift declarations without `=`: `public let x: T`, `public var y: T`, `deinit`.
//!
//! A stored or computed property without an initialiser, `deinit`, and
//! `associatedtype` write no event in the CST. The one-pass lexer previously
//! only skipped statements opening with `@`, so every modifier-prefixed
//! property declaration (`public let rawValue: String`, `public var count: Int`)
//! invented an `x` event with no CST counterpart.

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
fn swift_declarations_without_initializer_emit_no_x_event() {
    let source = "struct M {\n    public let rawValue: String\n    public private(set) var count: Int\n    deinit {\n    }\n}\n";
    let language = langs::by_content("swift", source.as_bytes()).expect("a Swift row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Swift).expect("a Swift tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Swift);
    for line in [2, 3, 4] {
        let marker = format!("[\"x\",{line},");
        assert!(!events(&lexer).contains(&marker), "lexer invented event on line {line}: {}", events(&lexer));
        assert!(!events(&deep).contains(&marker), "deep invented event on line {line}: {}", events(&deep));
    }
}
