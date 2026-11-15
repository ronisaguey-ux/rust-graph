//! P5-20: the arm-merge reproduction, as a test instead of a note.
//!
//! A match arm's block is a statement list of its own, and the two readers must
//! agree about where it starts: the arm's body merges its statements on the
//! arm's **first line**, so `1 => { a(); 2 }` is one event on that line carrying
//! the call and the constant — not a frame of its own, and not an event on the
//! line the block closes.
//!
//! The proof is the four-line reproduction the phase was measured on, and it is
//! asserted in both directions: the CST writes the merged value and the lexer
//! agrees, so the guard has nothing to fall back for. A reading that puts the
//! statements in their own frame fails the first assertion; one that stamps them
//! on the closing line fails the second.
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

use deep_integration::Lang;
use facts::Facts;
use pipeline::gate::*;

/// The reproduction: one arm, a block body of two statements.
const ARM: &str = "fn pick(v: u8) -> u8 {\n    match v {\n        1 => { a(); 2 }\n        _ => 0,\n    }\n}\n";

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// The events of the function that holds the match, as `(marker, line)`.
fn event_lines(facts: &Facts) -> Vec<(char, u32)> {
    let mut out = Vec::new();
    for symbol in &facts.symbols {
        let mut rest = symbol.events.as_str();
        while let Some(at) = rest.find("[\"") {
            rest = &rest[at + 2..];
            let Some(kind) = rest.chars().next() else { break };
            if !matches!(kind, 'a' | 'x' | 'r') {
                continue;
            }
            rest = &rest[1..];
            let Some(comma) = rest.find(',') else { break };
            let Some(end) = rest[comma + 1..].find(',') else { break };
            if let Ok(line) = rest[comma + 1..comma + 1 + end].trim().parse::<u32>() {
                out.push((kind, line));
            }
            rest = &rest[comma + 1 + end..];
        }
    }
    out
}

#[test]
fn the_arm_body_merges_its_statements_on_the_arms_first_line() {
    let deep = deep_facts(ARM);
    let lexer = lexer_facts(ARM);
    // Line 3 holds `1 => { a(); 2 }`: the arm's event is there, and the call is
    // inside its value rather than on a line of its own.
    let deep_events = event_lines(&deep);
    assert!(
        deep_events.iter().any(|(kind, line)| *kind == 'x' && *line == 3),
        "the CST wrote no event on the arm's line: {deep_events:?}"
    );
    assert!(
        deep.symbols.iter().any(|symbol| symbol.calls.iter().any(|call| call.line == 3)),
        "the arm's call was not recorded on its own line"
    );
    // Both readers agree, so a file that reads this way does not fall back.
    let verdict = pipeline::loses_in(&lexer, &deep, ARM);
    assert!(!verdict.lost(), "{verdict:?}");
    // And the two readers stamp the same lines. The CST writes the arm's value
    // and the call's own event, so it may write a line twice; what has to agree
    // is the set of lines, which is what a pairing is decided on.
    let lines = |facts: &Facts| -> std::collections::BTreeSet<u32> {
        event_lines(facts).into_iter().filter(|(kind, _)| *kind == 'x').map(|(_, line)| line).collect()
    };
    assert_eq!(lines(&lexer), lines(&deep), "the readers disagree about where the arm is");
}

#[test]
fn an_arm_body_is_not_a_frame_of_its_own() {
    // The losing reading gave the arm block its own frame, which is what moved
    // the corpus the wrong way. Only the function that holds the match may
    // appear as a non-module symbol.
    let deep = deep_facts(ARM);
    let named: Vec<(&str, u8)> = deep
        .symbols
        .iter()
        .filter(|symbol| symbol.kind != facts::KIND_MODULE)
        .map(|symbol| (symbol.name.as_str(), symbol.kind))
        .collect();
    assert_eq!(named, vec![("pick", facts::KIND_FUNCTION)], "an arm block opened a symbol");
}
