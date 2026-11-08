//! Size accounting: the number the 10 MB budget depends on.

use crate::{Compiled, Width};
use std::fmt;

/// Byte-level accounting for one compiled grammar.
pub struct Report {
    pub name: String,
    pub states: usize,
    pub symbols: usize,
    pub terminals: usize,
    pub nonterminals: usize,
    pub productions: usize,
    /// A dense `states x terminals` action matrix plus `states x nonterminals`
    /// gotos, four bytes a cell: what the tables would cost uncompressed.
    pub raw_bytes: usize,
    /// The same tables through the flat `u32`/`i32` ABI slices.
    pub abi_bytes: usize,
    /// The narrow packed form: `u16` cells and `u8`/`u16` indices where they fit.
    pub packed_bytes: usize,
    /// Occupied cells in the action and goto tables.
    pub cells: usize,
    pub width: Width,
    pub index_width: u8,
}

impl Report {
    /// Measure a compiled grammar.
    pub fn new(c: &Compiled) -> Report {
        let t = c.tables();
        let terminals = c.terminal_count();
        let nonterminals = c.nonterminal_count();
        let states = c.states();
        let wide = states * terminals * 4 + states * nonterminals * 4;
        let packed = t.packed();
        Report {
            name: c.name().to_string(),
            states,
            symbols: c.symbol_count(),
            terminals,
            nonterminals,
            productions: c.production_count(),
            raw_bytes: wide,
            abi_bytes: t.abi_bytes(),
            packed_bytes: packed.bytes(),
            cells: t.action_table.iter().filter(|c| **c != 0).count()
                + t.goto_table.iter().filter(|c| **c != 0).count(),
            width: t.width,
            index_width: packed.idx_width,
        }
    }

    /// Raw bytes divided by packed bytes.
    pub fn ratio(&self) -> f64 {
        if self.packed_bytes == 0 {
            0.0
        } else {
            self.raw_bytes as f64 / self.packed_bytes as f64
        }
    }

    /// Bytes per state of the packed form.
    pub fn bytes_per_state(&self) -> f64 {
        if self.states == 0 {
            0.0
        } else {
            self.packed_bytes as f64 / self.states as f64
        }
    }

    /// The cell width in bytes.
    pub fn cell_bytes(&self) -> usize {
        self.width.entry_bytes()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let width = match self.width {
            Width::Half => "u16 cells",
            Width::Word => "u32 cells",
        };
        writeln!(f, "grammar:     {}", self.name)?;
        writeln!(f, "  states     {:>8}", self.states)?;
        writeln!(
            f,
            "  symbols    {:>8}  ({} terminals, {} non-terminals)",
            self.symbols, self.terminals, self.nonterminals
        )?;
        writeln!(f, "  productions{:>8}", self.productions)?;
        writeln!(
            f,
            "  raw        {:>8} bytes  ({}x{} action + {}x{} goto, u32 cells)",
            self.raw_bytes, self.states, self.terminals, self.states, self.nonterminals
        )?;
        writeln!(f, "  abi        {:>8} bytes  (u32/i32 slices)", self.abi_bytes)?;
        writeln!(
            f,
            "  packed     {:>8} bytes  ({width}, {}-byte indices)",
            self.packed_bytes, self.index_width
        )?;
        writeln!(
            f,
            "  ratio      {:>8.2}x  ({:.1} bytes/state, {} explicit cells)",
            self.ratio(),
            self.bytes_per_state(),
            self.cells
        )
    }
}
