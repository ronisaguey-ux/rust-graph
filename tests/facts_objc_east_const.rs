//! East-const declarations (`Type const name = value;`) in Objective-C emit
//! the same assignment event as `const Type name = value;`.

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
#import <Foundation/Foundation.h>
NSUInteger const kAFUploadStream3GSuggestedPacketSize = 1024 * 16;
@implementation Foo
@end
"#;

#[test]
fn objc_east_const_matches_lexer() {
    let language = langs::by_content("m", SRC.as_bytes()).expect("an Objective-C row");
    let lexer = parser::parse(SRC.as_bytes(), language);
    let cst = deep_integration::parse_file(SRC.as_bytes(), Lang::ObjC).expect("an Objective-C tree");
    let deep = deep_facts::extract(SRC.as_bytes(), &cst, Lang::ObjC);
    assert_eq!(cst.errors, 0);
    assert!(
        lexer.symbols[0].events.contains(r#"["a",3,["kAFUploadStream3GSuggestedPacketSize"]"#),
        "lexer events: {}",
        lexer.symbols[0].events
    );
    assert!(
        deep.symbols[0].events.contains(r#"["a",3,["kAFUploadStream3GSuggestedPacketSize"]"#),
        "deep events: {}",
        deep.symbols[0].events
    );
}
