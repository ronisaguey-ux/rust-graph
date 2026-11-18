//! Indented Objective-C `#import` and `#include` directives still emit imports.

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

const SRC: &str = r#"
#if 1
#    import "SentrySampling.h"
#    include "Local.h"
#endif

@implementation Example
@end
"#;

#[test]
fn objc_indented_import_matches_lexer() {
    let language = langs::by_content("m", SRC.as_bytes()).expect("an Objective-C row");
    let lexer = parser::parse(SRC.as_bytes(), language);
    let cst = deep_integration::parse_file(SRC.as_bytes(), Lang::ObjC).expect("an Objective-C tree");
    let deep = deep_facts::extract(SRC.as_bytes(), &cst, Lang::ObjC);
    let lex_mods: Vec<&str> = lexer.imports.iter().map(|i| i.module.as_str()).collect();
    let deep_mods: Vec<&str> = deep.imports.iter().map(|i| i.module.as_str()).collect();
    assert_eq!(lex_mods, vec!["SentrySampling.h", "Local.h"]);
    assert_eq!(deep_mods, lex_mods);
}
