//! The differential test for the incremental parser: incremental output must be
//! byte-identical to a full parse, and it must be faster.
//!
//! The grammar here is a *structural* Python grammar — statements, blocks,
//! indentation and a permissive expression soup — because the point is not to
//! parse Python perfectly but to have a real CST over real Python files with
//! hundreds of nested nodes, so a spliced subtree has something to be right or
//! wrong about. The fixtures are the repository's own
//! `crates/deep-grammars/fixtures/python/*.py`, read at run time.
//!
//! The module under test lives in `src/incremental.rs`; it is pulled in with
//! `#[path]` so the test runs before `pub mod incremental;` lands in `lib.rs`.
//! Once it does, replace the three shim modules and the `#[path]` line with
//! `use deep_rt::incremental::*;`.

// The shims make `crate::cst`, `crate::driver` and `crate::table` resolve the
// same way inside the included module as they would inside the crate.
mod cst {
    pub use deep_rt::cst::*;
}
mod driver {
    pub use deep_rt::driver::*;
}
mod table {
    pub use deep_rt::table::*;
}

#[path = "../src/incremental.rs"]
mod incremental;

use std::time::{Duration, Instant};

use deep_rt::{
    parse, reduce_cell, shift_cell, Config, Cst, Grammar, Lexer, Node, NodeId, StateId, SymbolId,
    Token, EOF_SYMBOL, NONE,
};
use incremental::{Edit, Report};

// ---------------------------------------------------------------------------
// The language: terminals, nonterminals, productions
// ---------------------------------------------------------------------------

const NAME: SymbolId = 1;
const NUM: SymbolId = 2;
const STR: SymbolId = 3;
const OP: SymbolId = 4;
const NL: SymbolId = 5;
const INDENT: SymbolId = 6;
const DEDENT: SymbolId = 7;
const LP: SymbolId = 8;
const RP: SymbolId = 9;
const LB: SymbolId = 10;
const RB: SymbolId = 11;
const LC: SymbolId = 12;
const RC: SymbolId = 13;
/// The `:` that ends a compound header, emitted by the lexer only at bracket
/// depth zero on a line that starts with `def`/`class`/a block keyword.
const COLON: SymbolId = 14;
const DEF: SymbolId = 15;
const CLASS: SymbolId = 16;
const HEAD: SymbolId = 17;
const AT: SymbolId = 18;
/// Any other `:` — dict entries, slices, annotations, lambdas.
const COLON2: SymbolId = 19;
/// A block keyword that is not the first token of its line (`x if y else z`).
const HEAD_EXPR: SymbolId = 20;
/// `@` in expression position (`a @ b`), as opposed to a decorator.
const AT_EXPR: SymbolId = 21;

const FILE: SymbolId = 24;
const STMTS: SymbolId = 25;
const STMT: SymbolId = 26;
const LINE: SymbolId = 27;
const ITEMS: SymbolId = 28;
const ITEM: SymbolId = 29;
const SUITE: SymbolId = 30;
const SYMBOLS: u32 = 31;

/// `(lhs, rhs)`, in production-id order. The first entry is the augmented
/// start; its state accepts instead of reducing, so it needs no kind of its own.
///
/// The line-start keywords have their own terminals (the scanner knows whether
/// a keyword is the first token of its logical line), which is what keeps
/// `def f():` from also being readable as the item list `def f ( ) :` — one
/// ambiguity there and the table goes non-LR.
const PRODUCTIONS: &[(SymbolId, &[SymbolId])] = &[
    (0, &[FILE]),                                     // 0  AUG -> FILE
    (FILE, &[STMTS]),                                 // 1
    (STMTS, &[STMTS, STMT]),                          // 2
    (STMTS, &[]),                                     // 3
    (STMT, &[LINE, NL]),                              // 4
    (STMT, &[NL]),                                    // 5
    (STMT, &[DEF, NAME, ITEMS, COLON, SUITE]),        // 6
    (STMT, &[DEF, NAME, ITEMS, COLON, LINE, NL]),     // 7
    (STMT, &[CLASS, NAME, COLON, SUITE]),             // 8
    (STMT, &[CLASS, NAME, COLON, LINE, NL]),          // 9
    (STMT, &[CLASS, NAME, ITEMS, COLON, SUITE]),      // 10
    (STMT, &[CLASS, NAME, ITEMS, COLON, LINE, NL]),   // 11
    (STMT, &[HEAD, ITEMS, COLON, SUITE]),             // 12
    (STMT, &[HEAD, ITEMS, COLON, LINE, NL]),          // 13
    (STMT, &[HEAD, COLON, SUITE]),                    // 14  `else:` / `try:`
    (STMT, &[HEAD, COLON, LINE, NL]),                 // 15
    (STMT, &[AT, ITEMS, NL]),                         // 16  a decorator
    (LINE, &[ITEMS]),                                 // 17
    (ITEMS, &[ITEMS, ITEM]),                          // 18
    (ITEMS, &[ITEM]),                                 // 19
    (ITEM, &[NAME]),                                  // 20
    (ITEM, &[NUM]),                                   // 21
    (ITEM, &[STR]),                                   // 22
    (ITEM, &[OP]),                                    // 23
    (ITEM, &[COLON2]),                                // 24
    (ITEM, &[LP, ITEMS, RP]),                         // 25
    (ITEM, &[LB, ITEMS, RB]),                         // 26
    (ITEM, &[LC, ITEMS, RC]),                         // 27
    (ITEM, &[HEAD_EXPR]),                             // 28  `x if y else z`
    (ITEM, &[AT_EXPR]),                               // 29  matrix `a @ b`
    (SUITE, &[NL, INDENT, STMTS, DEDENT]),            // 30
    (ITEM, &[LP, RP]),                                // 31  `f()`, `def f():`
    (ITEM, &[LB, RB]),                                // 32
    (ITEM, &[LC, RC]),                                // 33
];

struct PyTables {
    states: u32,
    action_index: Vec<i32>,
    action_table: Vec<u32>,
    goto_index: Vec<i32>,
    goto_table: Vec<u32>,
    default_reduce: Vec<SymbolId>,
    production_len: Vec<u8>,
}

impl PyTables {
    fn grammar(&self) -> Grammar<'_> {
        Grammar {
            name: "py-structural",
            states: self.states,
            symbols: SYMBOLS,
            action_index: &self.action_index,
            action_table: &self.action_table,
            goto_index: &self.goto_index,
            goto_table: &self.goto_table,
            default_reduce: &self.default_reduce,
            production_len: &self.production_len,
        }
    }
}

#[inline]
fn is_terminal(symbol: SymbolId) -> bool {
    symbol >= 1 && symbol < FILE
}

/// FIRST of a symbol, as a bit set over terminals. Lookahead bit 0 is EOF;
/// terminal `t` is bit `t`.
fn first_of(symbol: SymbolId, first: &[u32], nullable: &[bool]) -> (u32, bool) {
    if is_terminal(symbol) {
        (1u32 << symbol, false)
    } else {
        let at = symbol as usize;
        (first[at], nullable[at])
    }
}

/// FIRST of a symbol string, followed by the lookahead set `la`.
fn first_seq(seq: &[SymbolId], la: u32, first: &[u32], nullable: &[bool]) -> u32 {
    let mut set = 0u32;
    for &symbol in seq {
        let (f, n) = first_of(symbol, first, nullable);
        set |= f;
        if !n {
            return set;
        }
    }
    set | la
}

/// One canonical LR(1) item: production, dot position, lookahead set (one bit).
type Item1 = (u16, u16, u32);

fn closure1(seed: Vec<Item1>, first: &[u32], nullable: &[bool]) -> Vec<Item1> {
    let mut items = seed;
    items.sort_unstable();
    items.dedup();
    let mut at = 0usize;
    while at < items.len() {
        let (production, dot, lookahead) = items[at];
        at += 1;
        let rhs = PRODUCTIONS[production as usize].1;
        if dot as usize >= rhs.len() {
            continue;
        }
        let symbol = rhs[dot as usize];
        if is_terminal(symbol) {
            continue;
        }
        let expect = first_seq(&rhs[dot as usize + 1..], lookahead, first, nullable);
        let mut bit = 0u32;
        while bit < 32 {
            if expect & (1u32 << bit) != 0 {
                for (id, &(lhs, _)) in PRODUCTIONS.iter().enumerate() {
                    if lhs != symbol {
                        continue;
                    }
                    let item = (id as u16, 0u16, 1u32 << bit);
                    if !items.contains(&item) {
                        items.push(item);
                    }
                }
            }
            bit += 1;
        }
    }
    items.sort_unstable();
    items.dedup();
    items
}

/// Build canonical LR(1) tables from [`PRODUCTIONS`], panicking with the item
/// set on a conflict. The ABI has one action cell per (state, terminal), so a
/// conflict here would have to be resolved by luck, and the differential test
/// would then be comparing two guesses.
fn build_tables() -> PyTables {
    // FIRST and nullability, fixed point over the productions.
    let mut first = vec![0u32; SYMBOLS as usize];
    let mut nullable = vec![false; SYMBOLS as usize];
    for _ in 0..PRODUCTIONS.len() * 2 {
        for &(lhs, rhs) in PRODUCTIONS {
            if lhs == 0 {
                continue;
            }
            let mut set = 0u32;
            let mut all_nullable = true;
            for &symbol in rhs {
                let (f, n) = first_of(symbol, &first, &nullable);
                set |= f;
                if !n {
                    all_nullable = false;
                    break;
                }
            }
            first[lhs as usize] |= set;
            if all_nullable {
                nullable[lhs as usize] = true;
            }
        }
    }

    // Canonical LR(1) item sets. The augmented start has EOF as its lookahead.
    let mut states: Vec<Vec<Item1>> = Vec::new();
    let mut transitions: Vec<(usize, SymbolId, usize)> = Vec::new();
    states.push(closure1(vec![(0, 0, 1u32)], &first, &nullable));
    let mut at = 0usize;
    while at < states.len() {
        let mut symbols: Vec<SymbolId> = Vec::new();
        for &(production, dot, _) in &states[at] {
            let rhs = PRODUCTIONS[production as usize].1;
            if (dot as usize) < rhs.len() {
                let symbol = rhs[dot as usize];
                if !symbols.contains(&symbol) {
                    symbols.push(symbol);
                }
            }
        }
        symbols.sort_unstable();
        for symbol in symbols {
            let mut seed: Vec<Item1> = Vec::new();
            for &(production, dot, lookahead) in &states[at] {
                let rhs = PRODUCTIONS[production as usize].1;
                if (dot as usize) < rhs.len() && rhs[dot as usize] == symbol {
                    seed.push((production, dot + 1, lookahead));
                }
            }
            let next = closure1(seed, &first, &nullable);
            let target = match states.iter().position(|state| *state == next) {
                Some(found) => found,
                None => {
                    states.push(next);
                    states.len() - 1
                }
            };
            transitions.push((at, symbol, target));
        }
        at += 1;
    }

    let action_width = SYMBOLS as usize + 1;
    let mut action_table = vec![0u32; states.len() * action_width];
    let mut goto_table = vec![0u32; states.len() * SYMBOLS as usize];
    let mut default_reduce = vec![0 as SymbolId; states.len()];

