//! Reading the flat tables `deep-abi::Grammar` carries.
//!
//! The ABI fixes the shapes but not the packing, so this module is the one
//! place that names it. Everything below is a pure function over slices.
//!
//! ```text
//! action_index: 2 x i32 per state = (row, default)
//!     row >= 0  ->  action_table[row + column(symbol)] is this state's cell
//!     the EOF column is `symbols` (the slot past the last symbol), so every
//!     action row must be `symbols + 1` wide
//!     row <  0  ->  no row at all, only the default action applies
//!     default < 0 -> reduce by production (-default - 1) when the cell is 0
//!     default >= 0 -> no default action (it is reserved and must be 0)
//!
//! action_table cell (u32):
//!     0                     no action
//!     1 ..= states          shift, to state (cell - 1)
//!     0x4000_0000           accept
//!     0x8000_0000 | prod    reduce by production prod
//!
//! goto_index: 2 x i32 per state = (row, reserved)
//!     goto_table[row + nonterminal] = state + 1, or 0 for no goto
//! ```
//!
//! `default_reduce[state]` names the nonterminal a state reduces by, which is
//! how a production gets its node kind: the ABI has no per-production LHS.

use deep_abi::{Grammar, StateId, SymbolId, EOF_SYMBOL, ERROR_SYMBOL};

/// No action in a cell.
pub const ACTION_NONE: u32 = 0;
/// Accept, in a cell.
pub const ACTION_ACCEPT: u32 = 0x4000_0000;
/// Reduce, ORed with the production id.
pub const ACTION_REDUCE: u32 = 0x8000_0000;
/// The mask that tells shift, accept and reduce apart.
pub const ACTION_KIND: u32 = 0xC000_0000;

/// A decoded table cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Shift(StateId),
    Reduce(u32),
    Accept,
}

impl Action {
    pub fn decode(cell: u32) -> Action {
        if cell == ACTION_NONE {
            Action::None
        } else if cell == ACTION_ACCEPT {
            Action::Accept
        } else if cell & ACTION_KIND == ACTION_REDUCE {
            Action::Reduce(cell & !ACTION_KIND)
        } else {
            Action::Shift((cell - 1) as StateId)
        }
    }
}

/// Encode a shift, for the table generator and for tests.
pub fn shift_cell(state: StateId) -> u32 {
    state as u32 + 1
}

/// Encode a reduce, for the table generator and for tests.
pub fn reduce_cell(production: u32) -> u32 {
    (ACTION_REDUCE | production) as u32
}

#[inline]
fn row_of(index: &[i32], state: StateId) -> Option<(usize, i32)> {
    let at = state as usize * 2;
    if at + 1 >= index.len() {
        return None;
    }
    let row = index[at];
    let default = index[at + 1];
    if row < 0 {
        None
    } else {
        Some((row as usize, default))
    }
}

/// The column a symbol occupies in an action row.
///
/// `EOF` is not a grammar symbol, so it gets the column just past the last
/// symbol: a row must be at least `symbols + 1` wide.
#[inline]
pub fn column(grammar: &Grammar<'_>, symbol: SymbolId) -> usize {
    if symbol == EOF_SYMBOL || symbol == ERROR_SYMBOL {
        grammar.symbols as usize
    } else {
        symbol as usize
    }
}

/// The action for a state and terminal.
pub fn action(grammar: &Grammar<'_>, state: StateId, symbol: SymbolId) -> Action {
    if symbol != EOF_SYMBOL && symbol as u32 >= grammar.symbols {
        // A symbol the grammar never numbered: the lexer invented it, so no
        // state can act on it and it must not fall through to a default reduce.
        return Action::None;
    }
    match row_of(grammar.action_index, state) {
        Some((row, default)) => {
            let cell = row
                .checked_add(column(grammar, symbol))
                .and_then(|at| grammar.action_table.get(at))
                .copied()
                .unwrap_or(ACTION_NONE);
            if cell != ACTION_NONE {
                Action::decode(cell)
            } else if default < 0 {
                Action::Reduce((-default - 1) as u32)
            } else {
                Action::None
            }
        }
        None => Action::None,
    }
}

/// The goto for a state and nonterminal.
pub fn goto(grammar: &Grammar<'_>, state: StateId, symbol: SymbolId) -> Option<StateId> {
    let (row, _) = row_of(grammar.goto_index, state)?;
    let cell = row
        .checked_add(symbol as usize)
        .and_then(|at| grammar.goto_table.get(at))
        .copied()
        .unwrap_or(0);
    if cell == 0 {
        None
    } else {
        Some((cell - 1) as StateId)
    }
}

/// The node kind a reduction in `state` produces.
///
/// The ABI names the symbol per state (`default_reduce`) but a production is
/// what a reduce action carries. Use the state entry; if a generator only
/// filled the slice per production, that length is used instead.
pub fn lhs(grammar: &Grammar<'_>, state: StateId, production: u32) -> SymbolId {
    let at = state as usize;
    if at < grammar.default_reduce.len() {
        let named = grammar.default_reduce[at];
        if named != 0 {
            return named;
        }
    }
    let at = production as usize;
    if at < grammar.default_reduce.len() {
        return grammar.default_reduce[at];
    }
    0
}

/// The length of a production, or 0 when the table does not list it.
pub fn production_len(grammar: &Grammar<'_>, production: u32) -> u32 {
    grammar
        .production_len
        .get(production as usize)
        .copied()
        .unwrap_or(0) as u32
}

/// Whether a state can act on a terminal without recovery.
pub fn accepts(grammar: &Grammar<'_>, state: StateId, symbol: SymbolId) -> bool {
    action(grammar, state, symbol) != Action::None
}

/// One step of a path from a state to something it can shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// The nonterminal to enter.
    pub symbol: SymbolId,
    /// The state after that goto.
    pub state: StateId,
}

/// Where a state's missing input begins: the shortest chain of gotos that ends
/// in a shiftable terminal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expected {
    pub steps: Vec<Step>,
    pub terminal: Option<SymbolId>,
}

/// Search, breadth first and iteratively, for the shortest chain of gotos from
/// `state` that reaches a state with a shift on some terminal.
///
/// This is what lets recovery insert a whole empty prefix (`item -> def`, where
/// the missing `def` is itself missing) instead of giving up.
pub fn expected(grammar: &Grammar<'_>, state: StateId) -> Option<Expected> {
    let symbols = grammar.symbols.min(SymbolId::MAX as u32) as usize;
    if symbols == 0 || state as u32 >= grammar.states {
        return None;
    }
    // Visited states, then the parent link for the tree we build on the way out.
    let mut seen = vec![false; grammar.states as usize];
    let mut queue: Vec<StateId> = Vec::new();
    // (state, parent index in `queue`, the step taken to reach it)
    let mut steps: Vec<(StateId, usize, Step)> = Vec::new();
    seen[state as usize] = true;
    queue.push(state);
    steps.push((state, usize::MAX, Step { symbol: 0, state: 0 }));
    let mut head = 0;
    while head < queue.len() {
        let current = queue[head];
        for terminal in 1..symbols {
            let symbol = terminal as SymbolId;
            if symbol == ERROR_SYMBOL || symbol == EOF_SYMBOL {
                continue;
            }
            if matches!(action(grammar, current, symbol), Action::Shift(_)) {
                let mut chain = Vec::new();
                let mut at = head;
                while steps[at].1 != usize::MAX {
                    chain.push(steps[at].2);
                    at = steps[at].1;
                }
                chain.reverse();
                return Some(Expected {
                    steps: chain,
                    terminal: Some(symbol),
                });
            }
        }
        for nonterminal in 1..symbols {
            let symbol = nonterminal as SymbolId;
            if symbol == ERROR_SYMBOL || symbol == EOF_SYMBOL {
                continue;
            }
            if let Some(next) = goto(grammar, current, symbol) {
                if !seen[next as usize] {
                    seen[next as usize] = true;
                    queue.push(next);
                    let from = head;
                    steps.push((next, from, Step { symbol, state: next }));
                }
            }
        }
        head += 1;
    }
    None
}
