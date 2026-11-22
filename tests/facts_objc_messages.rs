//! Objective-C message sends: a message is a call with a receiver, which is
//! the shape the guard pairs, and a two-part selector is one call named after
//! both parts.
//!
//! `[obj insertObject:x atIndex:1]` used to be a tuple of names to the
//! one-pass reader and nothing at all to the CST reader, so every message line
//! was a row the guard could not pair. Both readers now write the same call,
//! and the rows below pin that shape.
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
#[path = "../src/deep_integration.rs"]
mod deep_integration;
#[path = "../src/deep_facts/mod.rs"]
mod deep_facts;

use deep_integration::Lang;
use facts::Facts;

const SOURCE: &str = "\
@interface Foo
- (void)go;
@end

void f(id obj, id x) {
  [obj insertObject:x atIndex:1];
  [obj go];
  id made = [[Foo alloc] init];
}
";

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("m", source.as_bytes()).expect("an Objective-C row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::ObjC).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::ObjC)
}

fn calls(facts: &Facts, key: &str) -> Vec<(String, u32)> {
    let symbol = facts
        .symbols
        .iter()
        .find(|symbol| symbol.key == key)
        .unwrap_or_else(|| panic!("no symbol {key} in {:?}", facts.symbols.iter().map(|s| &s.key).collect::<Vec<_>>()));
    symbol.calls.iter().map(|call| (call.path.clone(), call.line)).collect()
}

#[test]
fn a_two_part_selector_is_one_call_named_after_both_parts() {
    let facts = deep_facts(SOURCE);
    let calls = calls(&facts, "f");
    assert!(
        calls.iter().any(|(path, line)| path == "insertObject:atIndex:" && *line == 6),
        "the selector is one name, one colon per keyword part: {calls:?}"
    );
    // A message with no keyword part is the bare selector.
    assert!(calls.iter().any(|(path, line)| path == "go" && *line == 7), "{calls:?}");
    // `[[Foo alloc] init]`: the receiver is a message, so it is a call of its
    // own, and the outer message is named after its own selector.
    assert!(calls.iter().any(|(path, line)| path == "init" && *line == 8), "{calls:?}");
    assert!(calls.iter().any(|(path, line)| path == "alloc" && *line == 8), "{calls:?}");
}

#[test]
fn the_receiver_is_written_in_the_call() {
    let facts = deep_facts(SOURCE);
    let symbol = facts.symbols.iter().find(|symbol| symbol.key == "f").expect("the function");
    // `["c",id,["n","obj"],[…]]`: the receiver sits in the call's third slot,
    // which is what the guard pairs a message line on.
    assert!(
        symbol.events.contains("[\"c\",0,[\"n\",\"obj\"],"),
        "the receiver is not in the call: {}",
        symbol.events
    );
}

#[test]
fn both_readers_write_the_same_call() {
    let lexer = lexer_facts(SOURCE);
    let deep = deep_facts(SOURCE);
    assert_eq!(calls(&lexer, "f"), calls(&deep, "f"), "the two readers disagree on the calls");
    // The line itself has to agree too: same event, same value.
    let lexer_symbol = lexer.symbols.iter().find(|symbol| symbol.key == "f").expect("the function");
    let deep_symbol = deep.symbols.iter().find(|symbol| symbol.key == "f").expect("the function");
    assert_eq!(lexer_symbol.events, deep_symbol.events, "the two readers disagree on the events");
}
