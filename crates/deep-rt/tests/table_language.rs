//! A language with generated tables, parsed by the GLR driver through
//! [`deep_rt::Workspace::parse`].
//!
//! The tables are the hand-built LR(0) automaton of [`common::mini`]: a small
//! language with `def`, assignment and a braced block. The point of the test is the
//! *bridge*, not the tables: a [`Language`] carries a [`TableGrammar`], `parse`
//! picks the GLR engine by itself, and the result is the flat [`Tree`] every other
//! caller already knows, with the same `subtree_end`/`TOKEN`/`HAS_ERROR` shape.
//!
//! The same language also has a hand-written descent parser, so both engines are
//! run on the same inputs and their leaves compared. Node shape differs — the
//! driver has a node for every terminal and every unit reduction, the descent
//! parser only where the grammar calls `bump` — but the tokens, their ranges and
//! their order must be the same, and both roots must span the file.

#[path = "common/mod.rs"]
mod common;

use deep_rt::lexer::Cursor;
use deep_rt::syntax::{Class, Kind, KindInfo, Language, COMMENT, ERROR, FIRST_LANGUAGE_KIND, WHITESPACE};
use deep_rt::tree::{HAS_ERROR, MISSING, TOKEN};
use deep_rt::{Budget, Outcome, Parser, TableGrammar, Workspace};

use common::{mini, Tables, COLON, DEF, EQ, LBRACE, LP, NAME, NUM, RBRACE, RP, SEMI};

const K_DEF: Kind = FIRST_LANGUAGE_KIND;
const K_NAME: Kind = K_DEF + 1;
const K_NUM: Kind = K_DEF + 2;
const K_EQ: Kind = K_DEF + 3;
const K_SEMI: Kind = K_DEF + 4;
const K_LP: Kind = K_DEF + 5;
const K_RP: Kind = K_DEF + 6;
const K_COLON: Kind = K_DEF + 7;
const K_LBRACE: Kind = K_DEF + 8;
const K_RBRACE: Kind = K_DEF + 9;
/// A byte the grammar has no symbol for.
const K_ODD: Kind = K_DEF + 10;

const ROOT: Kind = K_DEF + 11;
const ITEMS: Kind = K_DEF + 12;
const ITEM: Kind = K_DEF + 13;
const DEFN: Kind = K_DEF + 14;
const BLOCK: Kind = K_DEF + 15;
const ASSIGN: Kind = K_DEF + 16;

const KINDS: &[KindInfo] = &[
    KindInfo { name: "def", class: Class::Token },
    KindInfo { name: "name", class: Class::Token },
    KindInfo { name: "num", class: Class::Token },
    KindInfo { name: "=", class: Class::Token },
    KindInfo { name: ";", class: Class::Token },
    KindInfo { name: "(", class: Class::Open(0) },
    KindInfo { name: ")", class: Class::Close(0) },
    KindInfo { name: ":", class: Class::Token },
    KindInfo { name: "{", class: Class::Open(1) },
    KindInfo { name: "}", class: Class::Close(1) },
    KindInfo { name: "odd", class: Class::BadToken },
    KindInfo { name: "file", class: Class::Node },
    KindInfo { name: "items", class: Class::Node },
    KindInfo { name: "item", class: Class::Node },
    KindInfo { name: "defn", class: Class::Node },
    KindInfo { name: "block", class: Class::Node },
    KindInfo { name: "assign", class: Class::Node },
];

/// The grammar's terminals as kinds, and back.
fn symbol(kind: Kind) -> deep_rt::SymbolId {
    match kind {
        K_DEF => DEF,
        K_NAME => NAME,
        K_NUM => NUM,
        K_EQ => EQ,
        K_SEMI => SEMI,
        K_LP => LP,
        K_RP => RP,
        K_COLON => COLON,
        K_LBRACE => LBRACE,
        K_RBRACE => RBRACE,
        // 0 is "not in the grammar": the driver has no action for it and makes an
        // ERROR node of the token, so no byte falls outside the tree.
        _ => 0,
    }
}

/// The kind a grammar symbol is shown as.
fn kind(symbol: deep_rt::SymbolId) -> Kind {
    match symbol {
        DEF => K_DEF,
        NAME => K_NAME,
        NUM => K_NUM,
        EQ => K_EQ,
        SEMI => K_SEMI,
        LP => K_LP,
        RP => K_RP,
        COLON => K_COLON,
        LBRACE => K_LBRACE,
        RBRACE => K_RBRACE,
        common::DEFN => DEFN,
        common::BLOCK => BLOCK,
        common::ASSIGN => ASSIGN,
        common::ITEM => ITEM,
        common::ITEMS => ITEMS,
        // START and FILE are both the root of the tree the driver hands back.
        _ => ROOT,
    }
}

fn lex(c: &mut Cursor) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
            WHITESPACE
        }
        b'#' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'(' => single(c, K_LP),
        b')' => single(c, K_RP),
        b'{' => single(c, K_LBRACE),
        b'}' => single(c, K_RBRACE),
        b'=' => single(c, K_EQ),
        b';' => single(c, K_SEMI),
        b':' => single(c, K_COLON),
        b'0'..=b'9' => {
            c.eat_while(|byte| byte.is_ascii_digit());
            K_NUM
        }
        byte if byte.is_ascii_alphabetic() || byte == b'_' => {
            let rest = c.rest();
            let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
            if &rest[..length] == b"def" {
                K_DEF
            } else {
                K_NAME
            }
        }
        _ => K_ODD,
    }
}

fn single(c: &mut Cursor, kind: Kind) -> Kind {
    c.bump();
    kind
}

/// `file: item*`, `item: def NAME ( ) : block | NAME = NUM ;`,
/// `block: { item* }` — written by hand, so a tableless caller of this same
/// language has an engine too.
fn file(p: &mut Parser) {
    while !p.at_end() {
        item(p);
    }
}

fn item(p: &mut Parser) {
    match p.current() {
        K_DEF => {
            let marker = p.open();
            p.bump();
            p.expect(K_NAME);
            if p.at(K_LP) {
                p.group(DEFN, def_tail);
            } else {
                p.missing();
            }
            p.close(marker, ITEM);
        }
        K_NAME => {
            let marker = p.open();
            p.bump();
            p.expect(K_EQ);
            p.expect(K_NUM);
            p.expect(K_SEMI);
            p.close(marker, ITEM);
        }
        _ => p.error_one(),
    }
}

fn def_tail(p: &mut Parser) {
    p.bump();
    p.expect(K_RP);
    p.expect(K_COLON);
    if p.at(K_LBRACE) {
        p.group(BLOCK, block);
    } else {
        p.missing();
    }
}

fn block(p: &mut Parser) {
    while !p.at_end() {
        item(p);
    }
}

/// The bracket pairs recovery may jump, in the grammar's numbering.
static PAIRS: &[(deep_rt::SymbolId, deep_rt::SymbolId)] = &[(LBRACE, RBRACE), (LP, RP)];

/// The tables, built once and leaked so a `'static` [`Language`] can point at them.
fn tables() -> &'static TableGrammar {
    static CELL: std::sync::OnceLock<&'static TableGrammar> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        let tables: &'static Tables = Box::leak(Box::new(mini()));
        Box::leak(Box::new(TableGrammar::new(tables.grammar(), kind, symbol, PAIRS)))
    })
}

/// The language through the GLR engine.
fn glr() -> Language {
    Language::new("mini", ROOT, KINDS, lex, file).with_tables(tables())
}

/// The same language by descent, for the leaf comparison.
fn descent() -> Language {
    Language::new("mini", ROOT, KINDS, lex, file)
}

fn parse(language: &Language, src: &str) -> (Workspace, Outcome) {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
    (workspace, outcome)
}

/// Every leaf that is a token: kind name and byte range, in document order.
fn leaves(language: &Language, workspace: &Workspace) -> Vec<(&'static str, u32, u32)> {
    let nodes = &workspace.tree.nodes;
    let mut out = Vec::new();
    for index in 1..nodes.len() {
        let node = &nodes[index];
        if node.flags & TOKEN != 0 {
            out.push((language.name(node.kind), node.start, node.end));
        }
    }
    out
}

#[test]
fn a_language_with_tables_parses_on_the_glr_engine() {
    let language = glr();
    assert!(language.tables.is_some());
    let (workspace, outcome) = parse(&language, "x = 1;");
    assert!(outcome.complete(), "{outcome:?}");
    let nodes = &workspace.tree.nodes;
    assert_eq!(nodes[0].kind, ROOT, "the root is the start production, mapped to the language's root");
    assert_eq!((nodes[0].start, nodes[0].end), (0, 6));
    assert_eq!(nodes[0].subtree_end as usize, nodes.len());
    assert!(nodes[0].flags & HAS_ERROR == 0);
    assert_eq!(nodes[1].kind, ITEMS);
    assert_eq!(workspace.tree.depth(), 4, "file > items > item > assign > tokens");
}

#[test]
fn both_engines_see_the_same_tokens_in_the_same_order() {
    for src in ["", "x = 1;", "def f(): { }", "x = 1; def f(): { y = 2; }", "  def f(): { x = 1; }  ", "x = 1; x = 2;"] {
        let (glr_workspace, glr_outcome) = parse(&glr(), src);
        let (descent_workspace, descent_outcome) = parse(&descent(), src);
        assert!(glr_outcome.complete(), "{src:?}: {glr_outcome:?}");
        assert!(descent_outcome.complete(), "{src:?}: {descent_outcome:?}");
        assert_eq!(glr_workspace.tree.root().kind, descent_workspace.tree.root().kind, "{src:?}");
        assert_eq!(
            leaves(&glr(), &glr_workspace),
            leaves(&descent(), &descent_workspace),
            "the two engines disagree on the tokens of {src:?}"
        );
        // Both roots own the whole input, trivia included.
        assert_eq!((glr_workspace.tree.root().start, glr_workspace.tree.root().end), (0, src.len() as u32));
    }
}

#[test]
fn a_broken_item_still_leaves_a_root_over_the_whole_file() {
    let language = glr();
    for src in ["def f(): { x = ; }", "}", "def : { }", "x 1;", "x = 1; } } }"] {
        let (workspace, outcome) = parse(&language, src);
        assert!(outcome.complete(), "{src:?}: {outcome:?}");
        let nodes = &workspace.tree.nodes;
        assert_eq!(nodes[0].kind, ROOT, "{src:?}: the root survived");
        assert_eq!((nodes[0].start, nodes[0].end), (0, src.len() as u32), "{src:?}: the root spans the file");
        // Either a token was skipped (an `ERROR` node) or one was inserted (a
        // `MISSING` node): both say the input was wrong, and neither escapes.
        assert!(
            nodes[0].flags & HAS_ERROR != 0 || nodes.iter().any(|node| node.flags & MISSING != 0),
            "{src:?}: the root must report the error below it"
        );
        // Every byte of the source is still inside some node.
        assert_eq!(nodes[nodes.len() - 1].end, src.len() as u32);
    }
}

#[test]
fn a_kind_outside_the_grammar_becomes_an_error_node_not_a_gap() {
    let language = glr();
    let (workspace, _) = parse(&language, "x = 1; @");
    let nodes = &workspace.tree.nodes;
    assert_eq!(nodes[0].kind, ROOT);
    // The runtime's own `ERROR` kind, not a symbol the language invented.
    assert!(nodes.iter().any(|node| node.kind == ERROR && node.flags & HAS_ERROR != 0));
    assert_eq!((nodes[0].start, nodes[0].end), (0, 8));
}

#[test]
fn the_step_ceiling_is_reported_as_exhaustion() {
    let language = glr();
    let budget = Budget { steps_per_token: 0, base_steps: 1, ..Budget::default() };
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(b"x = 1; x = 2;", &language, &budget).unwrap();
    assert_eq!(outcome.exhausted, Some(deep_rt::Exhausted::Steps));
    assert!(!outcome.complete());
}

#[test]
fn the_same_tables_give_the_same_tree_twice() {
    // The arena and the tree are per-parse state, so two parses cannot share them.
    let language = glr();
    let (first, _) = parse(&language, "def f(): { x = 1; }");
    let (second, _) = parse(&language, "def f(): { x = 1; }");
    assert_eq!(first.tree.sexp(&language, true), second.tree.sexp(&language, true));
    assert_eq!(first.tree.nodes.len(), second.tree.nodes.len());
}

#[test]
fn the_shape_of_a_terminal_is_a_leaf_with_its_range() {
    let language = glr();
    let (workspace, _) = parse(&language, "x = 7;");
    let index = workspace.tree.nodes.iter().position(|node| node.kind == K_NUM).expect("the number");
    let number = workspace.tree.nodes[index];
    assert_eq!((number.start, number.end), (4, 5));
    assert!(number.flags & TOKEN != 0, "a shifted terminal is a token leaf");
    assert_eq!(number.subtree_end as usize, index + 1, "a leaf owns nothing below it");
}
