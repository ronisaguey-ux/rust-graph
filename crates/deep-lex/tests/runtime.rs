//! The ABI seam, end to end: a real `deep_rt::parse` driving `deep_lex` through
//! `&mut dyn deep_abi::Lexer`, with a hand-written four-state grammar.
//!
//! The grammar is tiny on purpose — it exists to prove three things at once:
//!
//! 1. `DeepLexer` is object-safe and complete enough for the table-driven
//!    runtime (`next_token(state)` plus `mode()`);
//! 2. the `StateId -> Mode` table actually reaches the lexer through the trait,
//!    because state 2 turns `/x/` into a regex instead of two divisions;
//! 3. a parse finishes with a tree and no errors, so nothing about the seam is a
//!    stub.
//!
//! ```text
//! S -> IDENT REGEX          action_index: 2 x i32 per state = (row, default)
//! ```
//!
//! States: 0 IDENT -> 1, 1 REGEX -> 2, 2 reduce S (len 2), goto from 0 to
//! state 3, 3 accept on EOF. State 1 is why the mode table matters: after an
//! identifier `/` is a division by default, and only `Mode::Regex` at state 1
//! makes `a /x/` lex as `IDENT REGEX`.

use deep_abi::{Grammar, Lexer, StateId, SymbolId};
use deep_lex::{DeepLexer, Kind, Language, Mode};

// A grammar numbers its own symbols; this one says IDENT is 1, not the lexer's
// internal Kind discriminant, which is what `SymbolMap` is for.
const IDENT: SymbolId = 1;
const REGEX: SymbolId = 2;
/// The one nonterminal.
const S: SymbolId = 3;
/// `symbols` counts terminals and nonterminals; action rows are `symbols + 1`
/// wide because EOF takes the column past the last symbol.
const SYMBOLS: u32 = 4;

/// The lexer's kinds under this grammar's numbering.
fn symbols() -> deep_lex::SymbolMap {
    let mut map = deep_lex::SymbolMap::identity();
    map.set(Kind::Ident, IDENT);
    map.set(Kind::Regex, REGEX);
    map
}

const STATES: u32 = 4;

/// Action rows are `symbols + 1` wide (EOF takes the last column).
const ROW: usize = SYMBOLS as usize + 1;
const NONE: u32 = 0;
const ACCEPT: u32 = 0x4000_0000;

fn shift(state: StateId) -> u32 {
    state as u32 + 1
}

fn tables() -> (Vec<i32>, Vec<u32>, Vec<i32>, Vec<u32>, Vec<SymbolId>, Vec<u8>) {
    let mut action = vec![NONE; ROW * 4]; // one row per state
    action[IDENT as usize] = shift(1);
    action[ROW + REGEX as usize] = shift(2);
    // State 2's row is all zero so its default applies: reduce by production 0.
    // A `row < 0` state has no default at all, which is the trap this row avoids.
    // State 3 accepts on EOF, which sits in the last column.
    action[3 * ROW + SYMBOLS as usize] = ACCEPT;

    let action_index = vec![
        0, 0, // state 0: row 0
        ROW as i32, 0, // state 1: row 1
        2 * ROW as i32, -1, // state 2: empty row, default reduce by production 0
        3 * ROW as i32, 0, // state 3: row 3, accept on EOF
    ];
    let goto_index = vec![0, 0, -1, 0, -1, 0, -1, 0];
    let mut goto_table = vec![0u32; SYMBOLS as usize];
    goto_table[S as usize] = 3 + 1;
    let default_reduce = vec![0, 0, S, 0];
    let production_len = vec![2u8];
    (action_index, action, goto_index, goto_table, default_reduce, production_len)
}

#[test]
fn the_runtime_drives_the_lexer_through_the_abi() {
    let (action_index, action_table, goto_index, goto_table, default_reduce, production_len) =
        tables();
    let grammar = Grammar {
        name: "s-eq-regex",
        states: STATES,
        symbols: SYMBOLS,
        action_index: &action_index,
        action_table: &action_table,
        goto_index: &goto_index,
        goto_table: &goto_table,
        default_reduce: &default_reduce,
        production_len: &production_len,
    };

    // State 1 says "an operand starts here", which is the only reason `/x/`
    // is one token and not `/ x /`.
    let state_modes = [Mode::Default, Mode::Regex, Mode::Default, Mode::Default];
    let src = b"a /x/";
    let mut lexer = DeepLexer::new(src, Language::JavaScript)
        .with_symbols(symbols())
        .with_state_modes(&state_modes);
    let config = deep_rt::Config::new();
    let (cst, stats) = deep_rt::parse(&grammar, &mut lexer, &config).expect("a table and a tree");

    assert!(cst.root != deep_abi::NONE, "the parse produced a root");
    assert_eq!(cst.errors, 0, "no error nodes");
    assert_eq!(stats.tokens, 3, "IDENT, REGEX and EOF");
    assert_eq!(stats.shifts, 2);

    // The root covers the whole source: the regex token's span is the proof the
    // mode table arrived through `next_token(state)`.
    let root = cst.nodes[cst.root as usize];
    assert_eq!((root.start, root.end), (0, src.len() as u32));
}

#[test]
fn the_same_parse_without_the_mode_table_is_not_the_same_lexing() {
    // Without state 1's hint the lexer reads `/` after an identifier as
    // division and splits `/x/` into three tokens, so the grammar must recover:
    // this is what the mode table is for, and the test keeps the table honest.
    let (action_index, action_table, goto_index, goto_table, default_reduce, production_len) =
        tables();
    let grammar = Grammar {
        name: "s-eq-regex",
        states: STATES,
        symbols: SYMBOLS,
        action_index: &action_index,
        action_table: &action_table,
        goto_index: &goto_index,
        goto_table: &goto_table,
        default_reduce: &default_reduce,
        production_len: &production_len,
    };
    let src = b"a /x/";
    let mut lexer = DeepLexer::new(src, Language::JavaScript).with_symbols(symbols());
    let config = deep_rt::Config::new();
    let (cst, stats) = deep_rt::parse(&grammar, &mut lexer, &config).expect("recovery still yields a tree");
    assert!(stats.tokens > 2, "the slashes were separate tokens");
    assert!(cst.errors > 0 || stats.inserts > 0 || stats.skips > 0, "the parse had to recover");
}

#[test]
fn the_lexer_can_be_held_as_a_trait_object() {
    // `deep_rt::parse` takes `&mut dyn Lexer`; this checks the coercion is not
    // hidden behind a generic and that `mode()` is reachable through it.
    let mut owned = DeepLexer::new(b"x", Language::JavaScript);
    let lexer: &mut dyn Lexer = &mut owned;
    assert_eq!(lexer.mode(), "default");
    let token = lexer.next_token(0).expect("x");
    assert_eq!((token.start, token.end), (0, 1));
}
