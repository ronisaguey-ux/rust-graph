//! The tables in the layout the runtime reads, not the layout this crate stores.
//!
//! [`crate::compress`] keeps every row's symbol tag in the cell so a row needs no
//! check array and two rows can share a slot. The runtime in `deep-rt` reads a
//! different encoding, documented in `deep-rt/src/table.rs` and repeated here
//! because the two must not drift:
//!
//! ```text
//! action_index: 2 x i32 per state = (row, default)
//!     row >= 0  ->  action_table[row + column(symbol)] is this state's cell
//!     the EOF column is `symbols` (the slot past the last symbol), so every
//!     action row is `symbols + 1` wide
//!     row <  0  ->  no row at all, only the default action applies
//!     default < 0 -> reduce by production (-default - 1) when the cell is 0
//!     default >= 0 -> no default action
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
//! There is no tag to tell a row's own cell from a neighbour's, so a row may
//! only be placed where the whole `symbols + 1` window is empty: every column
//! the parser may look up must be 0 unless this row puts a cell there. Rows are
//! placed first-fit into the zeros of the table, which still reuses the padding
//! a denser table would waste, but never two rows' cells.
//!
//! Symbol numbering is the one [`crate::compile`] uses — 0 is end of input,
//! `1..=terminals` the terminals, then the non-terminals — except that the
//! end-of-input action is stored in the column past the last symbol, because
//! that is the column the runtime asks for. A caller that emits these arrays
//! must tell the runtime `symbols = symbol_count`.

use crate::table::Action;
use crate::Compiled;
use deep_abi::SymbolId;

/// A cell holding no action.
pub const CELL_NONE: u32 = 0;
/// A cell that accepts.
pub const CELL_ACCEPT: u32 = 0x4000_0000;
/// The bit a reduce cell sets, ORed with the production id.
pub const CELL_REDUCE: u32 = 0x8000_0000;

/// The compiled grammar as the runtime's flat slices.
pub struct RuntimeTables {
    pub states: u32,
    /// The number of symbols, end of input included. The runtime's own EOF
    /// column is `symbols`, one past the last of these.
    pub symbols: u32,
    pub action_index: Vec<i32>,
    pub action_table: Vec<u32>,
    pub goto_index: Vec<i32>,
    pub goto_table: Vec<u32>,
    /// Per state: the non-terminal its reductions produce, or 0 when the state
    /// reduces by productions of more than one non-terminal (the ABI has no
    /// per-production left-hand side; see [`RuntimeTables::ambiguous_states`]).
    pub default_reduce: Vec<SymbolId>,
    pub production_len: Vec<u8>,
    /// States whose reductions produce more than one non-terminal, so
    /// `default_reduce` names only the default production's.
    pub ambiguous_states: Vec<u32>,
}

impl RuntimeTables {
    /// Bytes these slices occupy.
    pub fn bytes(&self) -> usize {
        self.action_index.len() * 4
            + self.action_table.len() * 4
            + self.goto_index.len() * 4
            + self.goto_table.len() * 4
            + self.default_reduce.len() * 2
            + self.production_len.len()
    }

    /// Explicit (non-zero) cells in both tables.
    pub fn cells(&self) -> usize {
        self.action_table.iter().filter(|cell| **cell != 0).count()
            + self.goto_table.iter().filter(|cell| **cell != 0).count()
    }
}

/// Encode a compiled grammar for the runtime.
pub fn encode(compiled: &Compiled) -> RuntimeTables {
    let raw = compiled.raw();
    let productions = compiled.production_count();
    let symbols = compiled.symbol_count();
    let states = compiled.states();
    debug_assert!(symbols + 1 <= u32::MAX as usize);

    // A row of `symbols + 1` columns, indexed by symbol: a cell for symbol 0
    // (end of input) belongs in the column past the last symbol.
    let width = symbols + 1;
    let mut rows: Vec<Vec<u32>> = Vec::with_capacity(states);
    let mut defaults: Vec<i32> = Vec::with_capacity(states);
    for state in 0..states {
        let mut row = vec![CELL_NONE; width];
        for (sym, action) in &raw.action[state] {
            let column = if *sym == 0 { symbols } else { *sym as usize };
            if column >= width || *sym as u32 >= symbols as u32 {
                continue;
            }
            row[column] = cell(*action);
        }
        match raw.default_reduce[state] {
            Some(production) => defaults.push(-(i32::from(production)) - 1),
            None => defaults.push(0),
        }
        rows.push(row);
    }
    let (action_index, action_table) = place(&rows, &defaults);

    let gotos: Vec<Vec<u32>> = (0..states)
        .map(|state| {
            let mut row = vec![0u32; symbols];
            for (sym, target) in &raw.goto[state] {
                let column = *sym as usize;
                if column < symbols {
                    row[column] = u32::from(*target) + 1;
                }
            }
            row
        })
        .collect();
    let goto_defaults = vec![0i32; states];
    let (goto_index, goto_table) = place(&gotos, &goto_defaults);

    let mut default_reduce = vec![0u16; states];
    let mut ambiguous_states = Vec::new();
    for (state, row) in raw.action.iter().enumerate() {
        let mut lhs: Option<SymbolId> = None;
        let mut ambiguous = false;
        for (_, action) in row {
            if let Action::Reduce(production) = action {
                let symbol = compiled.production(*production).lhs;
                match lhs {
                    Some(seen) if seen != symbol => ambiguous = true,
                    Some(_) => {}
                    None => lhs = Some(symbol),
                }
            }
        }
        if ambiguous {
            ambiguous_states.push(state as u32);
            default_reduce[state] = raw.default_reduce[state]
                .map(|production| compiled.production(production).lhs)
                .unwrap_or(0);
        } else {
            default_reduce[state] = lhs.unwrap_or(0);
        }
    }
    // States that reduce with no explicit reduce cell (the default) must still
    // name their production's non-terminal.
    for state in 0..states {
        if default_reduce[state] == 0 {
            if let Some(production) = raw.default_reduce[state] {
                default_reduce[state] = compiled.production(production).lhs;
            }
        }
    }

    RuntimeTables {
        states: states as u32,
        symbols: symbols as u32,
        action_index,
        action_table,
        goto_index,
        goto_table,
        default_reduce,
        production_len: (0..productions).map(|p| compiled.production(p as u16).rhs.len() as u8).collect(),
        ambiguous_states,
    }
}

fn cell(action: Action) -> u32 {
    match action {
        Action::Shift(target) => u32::from(target) + 1,
        Action::Reduce(production) => CELL_REDUCE | u32::from(production),
        Action::Accept => CELL_ACCEPT,
        Action::Error => CELL_NONE,
    }
}

/// Place every row of `rows` first-fit into a table that starts with one zero
/// cell, so that a row of all zeros (or only a default) costs nothing.
///
/// `defaults` is the second `i32` of each index pair.
fn place(rows: &[Vec<u32>], defaults: &[i32]) -> (Vec<i32>, Vec<u32>) {
    let mut table: Vec<u32> = vec![0];
    // A slot that some row may look up. A row's whole window has to be free:
    // the runtime cannot tell this row's cell from a neighbour's, so two rows
    // may never share a slot even when both leave it empty.
    let mut taken: Vec<bool> = vec![false];
    let mut index: Vec<i32> = Vec::with_capacity(rows.len() * 2);
    for row in rows {
        let width = row.len();
        let row_at = if row.iter().all(|cell| *cell == 0) {
            -1
        } else {
            let base = first_fit(&taken, width);
            if table.len() < base + width {
                table.resize(base + width, 0);
                taken.resize(base + width, false);
            }
            for (column, cell) in row.iter().enumerate() {
                if *cell != 0 {
                    table[base + column] = *cell;
                }
                taken[base + column] = true;
            }
            base as i32
        };
        let at = index.len() / 2;
        index.push(row_at);
        index.push(*defaults.get(at).unwrap_or(&0));
    }
    (index, table)
}

/// The lowest offset where `width` slots are free.
fn first_fit(taken: &[bool], width: usize) -> usize {
    let mut base = 0usize;
    'outer: loop {
        for column in 0..width {
            if taken.get(base + column).copied().unwrap_or(false) {
                base += 1;
                continue 'outer;
            }
        }
        return base;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammars;

    #[test]
    fn every_table_cell_decodes_like_the_raw_table() {
        for builder in [grammars::expr(), grammars::json(), grammars::python_subset()] {
            let compiled = builder.build().expect("compiles");
            let tables = encode(&compiled);
            let raw = compiled.raw();
            for state in 0..compiled.states() {
                for symbol in 0..compiled.symbol_count() as u16 {
                    let want = raw.action(state as u32, symbol);
                    let got = lookup_action(&tables, state as u32, symbol);
                    assert_eq!(got, want, "{} state {state} symbol {symbol}", compiled.name());
                }
            }
        }
    }

    #[test]
    fn rows_never_share_a_cell() {
        let compiled = grammars::json().build().expect("compiles");
        let tables = encode(&compiled);
        // Re-derive which row owns each slot, and fail if two do.
        let mut owner: Vec<Option<usize>> = vec![None; tables.action_table.len()];
        for state in 0..tables.states as usize {
            let row = tables.action_index[state * 2];
            if row < 0 {
                continue;
            }
            for column in 0..compiled.symbol_count() + 1 {
                let at = row as usize + column;
                let cell = tables.action_table[at];
                if cell != 0 {
                    assert!(owner[at].is_none(), "slot {at} is used by two rows");
                    owner[at] = Some(state);
                }
            }
        }
    }

    /// The runtime's own lookup, written once here so the test does not need it.
    fn lookup_action(tables: &RuntimeTables, state: u32, symbol: u16) -> Action {
        let at = state as usize * 2;
        let row = tables.action_index[at];
        let default = tables.action_index[at + 1];
        if row >= 0 {
            let column = if symbol == 0 { tables.symbols as usize } else { symbol as usize };
            let cell = tables.action_table[(row as usize) + column];
            if cell != CELL_NONE {
                return if cell == CELL_ACCEPT {
                    Action::Accept
                } else if cell & CELL_REDUCE != 0 {
                    Action::Reduce((cell & !CELL_REDUCE) as u16)
                } else {
                    Action::Shift((cell - 1) as u16)
                };
            }
        }
        if default < 0 {
            Action::Reduce((-default - 1) as u16)
        } else {
            Action::Error
        }
    }
}

