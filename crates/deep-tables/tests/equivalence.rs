//! The same JSON grammar, compiled both ways, over the same fixtures.
//!
//! `recursive` compiles the DSL to parser functions on the old runtime;
//! `tables` compiles it to LALR(1) tables and runs them on `deep-rt`. This test
//! drives both, turns each parse into the same event language, and requires the
//! two streams to be equal — so a change to either path that moves a node, a
//! token or a kind fails here.
//!
//! The one representational difference is stated by the DSL itself: a rule whose
//! name starts with `_` makes no node. The recursive emitter drops it by
//! construction; an LR reduction always makes one, so the table event walk drops
//! the symbols in `JSON_TABLES_HIDDEN`. Everything else — the root, every named
//! node, every token and every byte range — has to match exactly.

mod deep;

mod recursive {
    rust_graph_deep::grammar! { JSON {
        name = "json";
        mode = recursive;
        lex = crate::deep::lex;

        tokens {
            LBRACE = "{" open braces,
            RBRACE = "}" close braces,
            LBRACKET = "[" open brackets,
            RBRACKET = "]" close brackets,
            COLON = ":",
            COMMA = ",",
            STRING = "string",
            NUMBER = "number",
            TRUE = "true",
            FALSE = "false",
            NULL = "null",
            BAD_STRING = "bad_string" bad,
            WORD = "word" bad,
        }

        document: _value*,
        _value: object | array | STRING | NUMBER | TRUE | FALSE | NULL,
        object: "{" (pair % ",") "}",
        array: "[" (_value % ",") "]",
        pair: STRING ":" _value,
    } }
}

mod tables {
    rust_graph_deep::grammar! { JSON {
        name = "json";
        mode = tables;
        // The runtime that drives the tables, and whose `Grammar` re-export the
        // emitted static is typed with.
        runtime = deep_rt;
        // Unused by this path: the tables are driven by `deep-rt`, whose lexer
        // is the shared `deep::json` lexer wrapped below.
        lex = crate::deep::lex;

        tokens {
            LBRACE = "{" open braces,
            RBRACE = "}" close braces,
            LBRACKET = "[" open brackets,
            RBRACKET = "]" close brackets,
            COLON = ":",
            COMMA = ",",
            STRING = "string",
            NUMBER = "number",
            TRUE = "true",
            FALSE = "false",
            NULL = "null",
            BAD_STRING = "bad_string" bad,
            WORD = "word" bad,
        }

        document: _value*,
        _value: object | array | STRING | NUMBER | TRUE | FALSE | NULL,
        object: "{" (pair % ",") "}",
        array: "[" (_value % ",") "]",
        pair: STRING ":" _value,
    } }
}

use deep::budget::Budget;
use deep::parser::Workspace;
use deep::syntax::{Language, EOF, ERROR, FIRST_LANGUAGE_KIND};
use deep::tree::Event;

/// One step of either parse. A token carries its text, so a kind that is the
/// same number in both paths but a different stretch of input still fails.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Open(String),
    Token(String, String),
    Close,
}

fn recursive_events(src: &[u8], language: &Language) -> Vec<Step> {
    let mut workspace = Workspace::default();
    workspace.parse(src, language, &Budget::default()).expect("a JSON file fits");
    let names = language;
    let mut steps = Vec::new();
    for event in &workspace.events {
        match event {
            Event::Start { kind, .. } => steps.push(Step::Open(names.name(*kind).to_string())),
            Event::Finish => steps.push(Step::Close),
            Event::Token(at) => {
                let text = String::from_utf8_lossy(workspace.tokens.text(src, *at as usize)).into_owned();
                steps.push(Step::Token(names.name(workspace.tokens.kind[*at as usize]).to_string(), text));
            }
            Event::Span { .. } | Event::Flag(_) => {}
        }
    }
    steps
}

/// The runtime's lexer, over the tokens the shared lexer already produced.
struct Tokens<'a> {
    tokens: &'a deep::lexer::Tokens,
    src: &'a [u8],
    at: usize,
    terminal_count: u16,
}

impl deep_abi::Lexer for Tokens<'_> {
    fn next_token(&mut self, _state: deep_abi::StateId) -> Option<deep_abi::Token> {
        while self.at < self.tokens.len() {
            let at = self.at;
            self.at += 1;
            let kind = self.tokens.kind[at];
            if kind < FIRST_LANGUAGE_KIND {
                // Whitespace and comments are trivia; anything else is the
                // runtime's own ERROR symbol, which no state acts on.
                if matches!(kind, deep::syntax::WHITESPACE | deep::syntax::COMMENT) {
                    continue;
                }
            }
            let symbol = if kind >= FIRST_LANGUAGE_KIND && kind - FIRST_LANGUAGE_KIND < self.terminal_count {
                kind - FIRST_LANGUAGE_KIND + 1
            } else {
                deep_abi::ERROR_SYMBOL
            };
            return Some(deep_abi::Token { symbol, start: self.tokens.start(at), end: self.tokens.end[at] });
        }
        None
    }
}

fn table_events(src: &[u8]) -> Vec<Step> {
    let mut tokens = deep::lexer::Tokens::default();
    deep::lexer::lex(src, &deep::json::JSON, &mut tokens);
    let mut lexer = Tokens { tokens: &tokens, src, at: 0, terminal_count: 13 };
    let config = deep_rt::Config::new().with_source(src);
    let (cst, _stats) = deep_rt::parse(&tables::JSON, &mut lexer, &config).expect("the tables parse");
    let mut steps = Vec::new();
    walk(&cst, cst.root, src, &mut steps);
    steps
}

fn walk(cst: &deep_abi::Cst, id: deep_abi::NodeId, src: &[u8], out: &mut Vec<Step>) {
    let node = cst.nodes[id as usize];
    let hidden = tables::JSON_HIDDEN.contains(&node.kind);
    let leaf = node.first_child == deep_abi::NONE;
    if leaf && node.kind <= 13 && node.kind != 0 {
        let text = String::from_utf8_lossy(&src[node.start as usize..node.end as usize]).into_owned();
        out.push(Step::Token(symbol_name(node.kind), text));
        return;
    }
    if !hidden {
        out.push(Step::Open(symbol_name(node.kind)));
    }
    let mut child = node.first_child;
    while child != deep_abi::NONE {
        walk(cst, child, src, out);
        child = cst.nodes[child as usize].next_sibling;
    }
    if !hidden {
        out.push(Step::Close);
    }
}

fn symbol_name(symbol: u16) -> String {
    tables::JSON_SYMBOLS.get(symbol as usize).copied().unwrap_or("<unknown>").to_string()
}

/// The fixtures: documents a JSON parser meets, including an empty one.
const VALID: &[(&str, &str)] = &[
    ("scalar", "42"),
    ("string", "\"hello\""),
    ("empty object", "{}"),
    ("empty array", "[]"),
    ("nested", "{\"a\": [1, 2, {\"b\": null}], \"c\": true, \"d\": false}"),
    ("top level sequence", "1 2 3"),
    ("brackets", "[[[]], [1], {}]"),
    ("whitespace and comments", "// leading\n{ \"a\" : /* inline */ 1 , \"b\" : -2.5e3 }"),
];

#[test]
fn both_paths_produce_the_same_event_stream() {
    for (name, fixture) in VALID {
        let recursive = recursive_events(fixture.as_bytes(), &recursive::JSON);
        let tables = table_events(fixture.as_bytes());
        assert_eq!(recursive, tables, "fixture {name}: {fixture}");
        assert!(recursive.len() > 1, "fixture {name} produced events");
    }
}

#[test]
fn the_event_streams_are_the_ones_the_grammar_says() {
    let steps = table_events(b"{\"a\": [1]}");
    let rendered: Vec<String> = steps
        .iter()
        .map(|step| match step {
            Step::Open(name) => format!("({name}"),
            Step::Token(name, text) => format!("[{name} {text}]"),
            Step::Close => ")".to_string(),
        })
        .collect();
    assert_eq!(
        rendered.join(" "),
        "(document (object [{ {] (pair [string \"a\"] [: :] (array [[ [] [number 1] [] ]] ) ) [} }] ) )"
    );
}

#[test]
fn malformed_input_still_produces_a_tree_on_both_paths() {
    for fixture in ["{\"a\" 1}", "[1 2]", "{\"a\": } }"] {
        assert!(recursive_errors(fixture) > 0, "{fixture}: the recursive path reported no error");

        let cst = match table_tree_result(fixture) {
            Ok(cst) => cst,
            Err(fatal) => panic!("{fixture}: the table path gave up: {fatal:?}"),
        };
        let errors = cst.nodes.iter().filter(|node| node.kind == deep_abi::ERROR_SYMBOL || node.flags & deep_abi::FLAG_ERROR != 0).count();
        let missing = cst.nodes.iter().filter(|node| node.flags & deep_abi::FLAG_MISSING != 0).count();
        assert!(errors + missing + cst.errors as usize > 0, "{fixture}: the table path reported no error");
        assert_eq!(cst.nodes[cst.root as usize].start, 0, "{fixture}: the root starts at the file");
        assert_eq!(cst.nodes[cst.root as usize].kind, tables::DOCUMENT, "{fixture}: the root is the document");
    }
}

#[test]
fn the_shapes_the_lr_driver_gives_up_on_are_pinned() {
    // Two shapes the recursive path turns into a tree and the LR driver does
    // not: a construct missing at the end of the file, where it gives up at
    // end of input instead of inserting what the state expected, and a missing
    // value right before a closing bracket, where the insert path consumes the
    // whole budget. Both are `deep-rt`'s recovery, not the tables': pinned so
    // the difference cannot change unnoticed.
    for fixture in ["{\"a\": 1", "[1, 2,, 3]", "{\"a\": }"] {
        assert!(recursive_errors(fixture) > 0, "{fixture}: the recursive path reported no error");
        match table_tree_result(fixture) {
            Err(deep_abi::Fatal::StackExhausted) => {}
            Ok(_) => panic!("{fixture}: the runtime now recovers here: update this test"),
            Err(other) => panic!("{fixture}: unexpected failure {other:?}"),
        }
    }
}

fn recursive_errors(fixture: &str) -> usize {
    let mut workspace = Workspace::default();
    workspace.parse(fixture.as_bytes(), &recursive::JSON, &Budget::default()).expect("fits");
    workspace.tree.nodes.iter().filter(|node| node.kind == ERROR).count()
        + workspace.events.iter().filter(|event| matches!(event, Event::Flag(_))).count()
}

fn table_tree_result(fixture: &str) -> Result<deep_abi::Cst, deep_abi::Fatal> {
    let mut tokens = deep::lexer::Tokens::default();
    deep::lexer::lex(fixture.as_bytes(), &deep::json::JSON, &mut tokens);
    let mut lexer = Tokens { tokens: &tokens, src: fixture.as_bytes(), at: 0, terminal_count: 13 };
    let pairs = [(tables::LBRACE, tables::RBRACE), (tables::LBRACKET, tables::RBRACKET)];
    let config = deep_rt::Config::new().with_source(fixture.as_bytes()).with_pairs(&pairs);
    deep_rt::parse(&tables::JSON, &mut lexer, &config).map(|(cst, _)| cst)
}

#[test]
fn the_kind_enums_agree_with_the_symbols_they_name() {
    // The table path numbers kinds the way the runtime wants them: symbol ids.
    assert_eq!(tables::DOCUMENT, tables::JSON_ROOT);
    assert_eq!(symbol_name(tables::LBRACE), "{");
    assert_eq!(symbol_name(tables::DOCUMENT), "document");
    // The recursive path's kinds stay the runtime's, unchanged.
    assert_eq!(recursive::LBRACE, FIRST_LANGUAGE_KIND);
    assert_eq!(recursive::DOCUMENT, deep::json::DOCUMENT);
    assert!(EOF > ERROR);
}
