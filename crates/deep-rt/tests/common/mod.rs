//! A hand-built grammar, its tables and a hand-written lexer.
//!
//! Small enough to check by eye. The mini language is
//!
//! ```text
//! file    : item*
//! item    : def | assign
//! def     : "def" NAME "(" ")" ":" block
//! block   : "{" item* "}"
//! assign  : NAME "=" NUM ";"
//! ```
//!
//! and the tables are the LR(0) automaton written out by hand, one row per
//! state, so a test failure points at a state and a symbol rather than at a
//! generator.

#![allow(dead_code)]

use deep_rt::{reduce_cell, shift_cell, Grammar, StateId, SymbolId, EOF_SYMBOL};

pub const DEF: SymbolId = 1;
pub const NAME: SymbolId = 2;
pub const NUM: SymbolId = 3;
pub const EQ: SymbolId = 4;
pub const SEMI: SymbolId = 5;
pub const LP: SymbolId = 6;
pub const RP: SymbolId = 7;
pub const COLON: SymbolId = 8;
pub const LBRACE: SymbolId = 9;
pub const RBRACE: SymbolId = 10;
pub const BAD: SymbolId = 11;

pub const FILE: SymbolId = 20;
pub const ITEMS: SymbolId = 21;
pub const ITEM: SymbolId = 22;
pub const DEFN: SymbolId = 23;
pub const BLOCK: SymbolId = 24;
pub const ASSIGN: SymbolId = 25;
pub const START: SymbolId = 26;

/// Small table builder: one row per state, written by hand.
struct Rows<'a> {
    action: &'a mut [u32],
    goto: &'a mut [u32],
    width: usize,
    symbols: usize,
}

impl Rows<'_> {
    fn column(&self, symbol: SymbolId) -> usize {
        if symbol == EOF_SYMBOL {
            self.symbols
        } else {
            symbol as usize
        }
    }
    fn shift(&mut self, state: usize, symbol: SymbolId, target: StateId) {
        let column = self.column(symbol);
        self.action[state * self.width + column] = shift_cell(target);
    }
    fn reduce(&mut self, state: usize, symbol: SymbolId, production: u32) {
        let column = self.column(symbol);
        self.action[state * self.width + column] = reduce_cell(production);
    }
    fn accept(&mut self, state: usize, symbol: SymbolId) {
        let column = self.column(symbol);
        self.action[state * self.width + column] = deep_rt::table::ACTION_ACCEPT;
    }
    fn go(&mut self, state: usize, symbol: SymbolId, target: StateId) {
        self.goto[state * 32 + symbol as usize] = target as u32 + 1;
    }
}

pub struct Tables {
    pub name: &'static str,
    pub states: u32,
    pub symbols: u32,
    pub action_index: Vec<i32>,
    pub action_table: Vec<u32>,
    pub goto_index: Vec<i32>,
    pub goto_table: Vec<u32>,
    pub default_reduce: Vec<SymbolId>,
    pub production_len: Vec<u8>,
}

impl Tables {
    pub fn grammar(&self) -> Grammar<'_> {
        Grammar {
            name: self.name,
            states: self.states,
            symbols: self.symbols,
            action_index: &self.action_index,
            action_table: &self.action_table,
            goto_index: &self.goto_index,
            goto_table: &self.goto_table,
            default_reduce: &self.default_reduce,
            production_len: &self.production_len,
        }
    }
}

const MINI_ROW: usize = 16;
const MINI_STATES: u32 = 19;

/// The mini language above, 19 states.
///
/// Productions, by id:
/// ```text
/// 0 START  -> FILE
/// 1 FILE   -> ITEMS
/// 2 ITEMS  -> ITEMS ITEM
/// 3 ITEMS  -> ε
/// 4 ITEM   -> DEFN
/// 5 ITEM   -> ASSIGN
/// 6 DEFN   -> DEF NAME LP RP COLON BLOCK
/// 7 BLOCK  -> LBRACE ITEMS RBRACE
/// 8 ASSIGN -> NAME EQ NUM SEMI
/// ```
pub fn mini() -> Tables {
    let states = MINI_STATES as usize;
    let mut action_index = vec![0i32; states * 2];
    let mut action_table = vec![0u32; states * MINI_ROW];
    let mut goto_index = vec![0i32; states * 2];
    let mut goto_table = vec![0u32; states * 32];
    let mut default_reduce = vec![0 as SymbolId; states];
    let production_len = vec![1u8, 1, 2, 0, 1, 1, 6, 3, 4];

    for state in 0..states {
        action_index[state * 2] = (state * MINI_ROW) as i32;
        goto_index[state * 2] = (state * 32) as i32;
    }

    let mut rows = Rows {
        action: &mut action_table,
        goto: &mut goto_table,
        width: MINI_ROW,
        symbols: 11,
    };

    // 0: items are the empty ITEMS; the default reduce is P3.
    default_reduce[0] = ITEMS;
    action_index[0 * 2 + 1] = -4; // P3
    rows.go(0, FILE, 1);
    rows.go(0, ITEMS, 2);

    // 1: START -> FILE .   accept at EOF.
    rows.accept(1, EOF_SYMBOL);

    // 2: FILE -> ITEMS .   and the item closure.
    rows.reduce(2, EOF_SYMBOL, 1);
    default_reduce[2] = FILE;
    rows.shift(2, DEF, 5);
    rows.shift(2, NAME, 7);
    rows.go(2, ITEM, 3);
    rows.go(2, DEFN, 4);
    rows.go(2, ASSIGN, 6);

    // 3: ITEMS -> ITEMS ITEM .   P2
    default_reduce[3] = ITEMS;
    action_index[3 * 2 + 1] = -3;

    // 4: ITEM -> DEFN .   P4
    default_reduce[4] = ITEM;
    action_index[4 * 2 + 1] = -5;

    // 5: DEFN -> DEF . NAME LP RP COLON BLOCK
    rows.shift(5, NAME, 8);

    // 6: ITEM -> ASSIGN .   P5
    default_reduce[6] = ITEM;
    action_index[6 * 2 + 1] = -6;

    // 7: ASSIGN -> NAME . EQ NUM SEMI
    rows.shift(7, EQ, 9);

    // 8: DEFN -> DEF NAME . LP RP COLON BLOCK
    rows.shift(8, LP, 10);

    // 9: ASSIGN -> NAME EQ . NUM SEMI
    rows.shift(9, NUM, 11);

    // 10: DEFN -> DEF NAME LP . RP COLON BLOCK
    rows.shift(10, RP, 12);

    // 11: ASSIGN -> NAME EQ NUM . SEMI
    rows.shift(11, SEMI, 13);

    // 12: DEFN -> DEF NAME LP RP . COLON BLOCK
    rows.shift(12, COLON, 14);

    // 13: ASSIGN -> NAME EQ NUM SEMI .   P8
    default_reduce[13] = ASSIGN;
    action_index[13 * 2 + 1] = -9;

    // 14: DEFN -> ... COLON . BLOCK
    rows.shift(14, LBRACE, 16);
    rows.go(14, BLOCK, 15);

    // 15: DEFN -> DEF NAME LP RP COLON BLOCK .   P6
    default_reduce[15] = DEFN;
    action_index[15 * 2 + 1] = -7;

    // 16: BLOCK -> LBRACE . ITEMS RBRACE   (and the empty ITEMS)
    default_reduce[16] = ITEMS;
    action_index[16 * 2 + 1] = -4;
    rows.go(16, ITEMS, 17);

    // 17: BLOCK -> LBRACE ITEMS . RBRACE   and the item closure.
    rows.shift(17, RBRACE, 18);
    rows.shift(17, DEF, 5);
    rows.shift(17, NAME, 7);
    rows.go(17, ITEM, 3);
    rows.go(17, DEFN, 4);
    rows.go(17, ASSIGN, 6);

    // 18: BLOCK -> LBRACE ITEMS RBRACE .   P7
    default_reduce[18] = BLOCK;
    action_index[18 * 2 + 1] = -8;

    Tables {
        name: "mini",
        states: MINI_STATES,
        symbols: 11,
        action_index,
        action_table,
        goto_index,
        goto_table,
        default_reduce,
        production_len,
    }
}

/// Parenthesised nesting, for the depth test.
///
/// ```text
/// 0 START -> EXPRS
/// 1 EXPRS -> EXPRS EXPR
/// 2 EXPRS -> EXPR
/// 3 EXPR  -> LP EXPR RP
/// 4 EXPR  -> NUM
/// ```
pub const NEST_LP: SymbolId = 1;
pub const NEST_NUM: SymbolId = 2;
pub const NEST_RP: SymbolId = 3;
pub const NEST_EXPRS: SymbolId = 21;
pub const NEST_EXPR: SymbolId = 22;

pub fn nest() -> Tables {
    let states = 8usize;
    let mut action_index = vec![0i32; states * 2];
    let mut action_table = vec![0u32; states * 8];
    let mut goto_index = vec![0i32; states * 2];
    let mut goto_table = vec![0u32; states * 32];
    let mut default_reduce = vec![0 as SymbolId; states];
    let production_len = vec![1u8, 2, 1, 3, 1];

    for state in 0..states {
        action_index[state * 2] = (state * 8) as i32;
        goto_index[state * 2] = (state * 32) as i32;
    }
    let mut rows = Rows {
        action: &mut action_table,
        goto: &mut goto_table,
        width: 8,
        symbols: 23,
    };

    rows.shift(0, NEST_LP, 3);
    rows.shift(0, NEST_NUM, 4);
    rows.go(0, NEST_EXPRS, 1);
    rows.go(0, NEST_EXPR, 2);

    rows.accept(1, EOF_SYMBOL);
    rows.shift(1, NEST_LP, 3);
    rows.shift(1, NEST_NUM, 4);
    rows.go(1, NEST_EXPR, 5);

    default_reduce[2] = NEST_EXPRS;
    action_index[2 * 2 + 1] = -3; // P2

    rows.shift(3, NEST_LP, 3);
    rows.shift(3, NEST_NUM, 4);
    rows.go(3, NEST_EXPR, 6);

    default_reduce[4] = NEST_EXPR;
    action_index[4 * 2 + 1] = -5; // P4

    default_reduce[5] = NEST_EXPRS;
    action_index[5 * 2 + 1] = -2; // P1

    rows.shift(6, NEST_RP, 7);

    default_reduce[7] = NEST_EXPR;
    action_index[7 * 2 + 1] = -4; // P3

    Tables {
        name: "nest",
        states: states as u32,
        symbols: 23,
        action_index,
        action_table,
        goto_index,
        goto_table,
        default_reduce,
        production_len,
    }
}

/// A lexer that never needs a parser state: the whole input is tokenised first.

mod lex;
mod shape;
#[allow(unused_imports)]
pub use lex::*;
#[allow(unused_imports)]
pub use shape::*;
