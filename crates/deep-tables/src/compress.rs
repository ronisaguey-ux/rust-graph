//! Row-displacement compression, default reductions and u16/u8 narrowing.
//!
//! Every table cell carries the symbol it belongs to in its high bits, so a row
//! needs no `check` array: a lookup is a displacement (`base + symbol`) followed
//! by a tag comparison. Slots are never shared between rows, which is what makes
//! the tag a sufficient guard against hitting another row's entry.

use crate::pack::{narrow_index_width, pack_entries, pack_index, Packed};
use crate::place::place;
use crate::table::{Action, RawTables};
use crate::Conflict;
use deep_abi::{Grammar, SymbolId};

/// The width of one packed table cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// 16-bit cell: 8-bit symbol tag, 8-bit payload. Needs at most 255 symbols,
    /// 127 states and 126 productions.
    Half,
    /// 32-bit cell: 16-bit symbol tag, 16-bit payload.
    Word,
}

impl Width {
    /// Bytes one cell occupies when packed.
    pub fn entry_bytes(self) -> usize {
        match self {
            Width::Half => 2,
            Width::Word => 4,
        }
    }

    pub(crate) fn shift_bit(self) -> u16 {
        match self {
            Width::Half => 0x80,
            Width::Word => 0x8000,
        }
    }

    pub(crate) fn shift_mask(self) -> u16 {
        self.shift_bit() - 1
    }

    pub(crate) fn tag(self, cell: u32) -> u16 {
        match self {
            Width::Half => ((cell >> 8) & 0xFF) as u16,
            Width::Word => (cell >> 16) as u16,
        }
    }

    pub(crate) fn payload(self, cell: u32) -> u16 {
        match self {
            Width::Half => (cell & 0xFF) as u16,
            Width::Word => cell as u16,
        }
    }

    pub(crate) fn cell(self, tag: SymbolId, payload: u16) -> u32 {
        match self {
            Width::Half => ((tag as u32 & 0xFF) << 8) | (payload as u32 & 0xFF),
            Width::Word => ((tag as u32) << 16) | payload as u32,
        }
    }
}

/// The compressed tables, in the exact shape [`deep_abi::Grammar`] borrows.
pub struct Tables {
    pub states: u32,
    pub symbols: u32,
    /// The packing width the table needs.
    pub width: Width,
    /// `(offset, count)` per state: displacement base and explicit cell count.
    pub action_index: Vec<i32>,
    pub action_table: Vec<u32>,
    pub goto_index: Vec<i32>,
    pub goto_table: Vec<u32>,
    /// Production id + 1 per state, or 0 when the state has no default.
    pub default_reduce: Vec<SymbolId>,
    pub production_len: Vec<u8>,
}

impl Tables {
    /// What the parser does in `state` on `sym`.
    pub fn action(&self, state: u32, sym: SymbolId) -> Action {
        let s = state as usize;
        if s * 2 + 1 < self.action_index.len() {
            let base = self.action_index[s * 2];
            let count = self.action_index[s * 2 + 1];
            if base >= 0 && count > 0 {
                let slot = base as usize + sym as usize;
                if let Some(&cell) = self.action_table.get(slot) {
                    if Width::Word.tag(cell) == sym {
                        return decode(Width::Word, cell);
                    }
                }
            }
            match self.default_reduce[s] {
                0 => Action::Error,
                d => Action::Reduce(d - 1),
            }
        } else {
            Action::Error
        }
    }

    /// The state reached from `state` on non-terminal `sym`, if any.
    pub fn goto(&self, state: u32, sym: SymbolId) -> Option<u16> {
        let s = state as usize;
        let base = *self.goto_index.get(s * 2)?;
        let count = *self.goto_index.get(s * 2 + 1)?;
        if base < 0 || count == 0 {
            return None;
        }
        let slot = base as usize + sym as usize;
        let cell = *self.goto_table.get(slot)?;
        if Width::Word.tag(cell) != sym {
            return None;
        }
        let payload = Width::Word.payload(cell);
        if payload == 0 {
            None
        } else {
            Some(payload - 1)
        }
    }

    /// The production a state reduces by when its row has no entry.
    pub fn default_reduce(&self, state: u32) -> Option<u16> {
        match self.default_reduce.get(state as usize) {
            Some(0) | None => None,
            Some(d) => Some(d - 1),
        }
    }

    /// The flat borrowed shape the ABI expects.
    pub fn as_grammar<'a>(&'a self, name: &'a str) -> Grammar<'a> {
        Grammar {
            name,
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

    /// Bytes the tables cost through the `&[u32]`/`&[i32]` ABI.
    pub fn abi_bytes(&self) -> usize {
        self.action_index.len() * 4
            + self.action_table.len() * 4
            + self.goto_index.len() * 4
            + self.goto_table.len() * 4
            + self.default_reduce.len() * 2
            + self.production_len.len()
    }

    /// Bytes the narrow on-disk form costs.
    pub fn compressed_bytes(&self) -> usize {
        self.packed().bytes()
    }

    /// Narrow the tables to the smallest widths the values allow.
    pub fn packed(&self) -> Packed {
        let idx_width = narrow_index_width(&self.action_index, &self.goto_index);
        let ent_width = self.width.entry_bytes() as u8;
        Packed {
            states: self.states,
            symbols: self.symbols,
            idx_width,
            ent_width,
            action_index: pack_index(&self.action_index, idx_width as usize),
            action_table: pack_entries(&self.action_table, self.width),
            goto_index: pack_index(&self.goto_index, idx_width as usize),
            goto_table: pack_entries(&self.goto_table, self.width),
            default_reduce: self
                .default_reduce
                .iter()
                .flat_map(|d| d.to_le_bytes())
                .collect(),
            production_len: self.production_len.clone(),
        }
    }
}

fn decode(width: Width, cell: u32) -> Action {
    let payload = width.payload(cell);
    if payload == 0 {
        Action::Error
    } else if payload & width.shift_bit() != 0 {
        Action::Shift(payload & width.shift_mask())
    } else {
        let prod = payload - 1;
        if prod == 0 {
            Action::Accept
        } else {
            Action::Reduce(prod)
        }
    }
}

/// Compress compact rows by row displacement plus default reductions.
pub fn compress(
    raw: &RawTables,
    symbols: usize,
    production_len: &[u8],
) -> Result<Tables, Conflict> {
    let states = raw.action.len();
    if states > 0x7FFF {
        return Err(Conflict::TooLarge(format!("{states} states exceed the 32767 tagged limit")));
    }
    if symbols > 0xFFFE {
        return Err(Conflict::TooLarge(format!("{symbols} symbols exceed the u16 limit")));
    }
    if production_len.len() > 0x7FFE {
        return Err(Conflict::TooLarge("more than 32766 productions".to_string()));
    }
    let width = if symbols <= 255 && states <= 127 && production_len.len() <= 126 {
        Width::Half
    } else {
        Width::Word
    };

    let mut action_rows: Vec<Vec<(SymbolId, u32)>> = Vec::with_capacity(states);
    for (s, row) in raw.action.iter().enumerate() {
        let def = raw.default_reduce[s];
        let mut cells = Vec::with_capacity(row.len());
        for (sym, act) in row {
            if let (Some(d), Action::Reduce(p)) = (def, act) {
                if d == *p {
                    continue;
                }
            }
            let payload = match act {
                // Cells are always Word-encoded in the borrowed tables; the
                // narrow width only changes how they are packed on disk.
                Action::Shift(t) => 0x8000 | (t & 0x7FFF),
                Action::Reduce(p) => *p as u16 + 1,
                Action::Accept => 1,
                Action::Error => 0,
            };
            if width == Width::Half && *sym > 0xFF {
                return Err(Conflict::TooLarge(format!(
                    "state {s} uses symbol {sym}, past the u16 tag limit"
                )));
            }
            if width == Width::Half {
                match act {
                    Action::Shift(t) if *t > 0x7F => {
                        return Err(Conflict::TooLarge(format!(
                            "state {s} shifts to {t}, past the u16 cell limit"
                        )))
                    }
                    Action::Reduce(p) if *p > 0x7E => {
                        return Err(Conflict::TooLarge(format!(
                            "production {p} is past the u16 cell limit"
                        )))
                    }
                    _ => {}
                }
            }
            cells.push((*sym, Width::Word.cell(*sym, payload)));
        }
        action_rows.push(cells);
    }

    let mut goto_rows: Vec<Vec<(SymbolId, u32)>> = Vec::with_capacity(states);
    for row in &raw.goto {
        let mut cells = Vec::with_capacity(row.len());
        for (sym, target) in row {
            let payload = *target as u16 + 1;
            if width == Width::Half && (payload > 0x7F || *sym > 0xFF) {
                return Err(Conflict::TooLarge("goto does not fit a u16 cell".to_string()));
            }
            cells.push((*sym, Width::Word.cell(*sym, payload)));
        }
        goto_rows.push(cells);
    }

    let (action_index, action_table) = place(&action_rows);
    let (goto_index, goto_table) = place(&goto_rows);

    Ok(Tables {
        states: states as u32,
        symbols: symbols as u32,
        width,
        action_index,
        action_table,
        goto_index,
        goto_table,
        default_reduce: raw
            .default_reduce
            .iter()
            .map(|d| d.map(|p| p + 1).unwrap_or(0))
            .collect(),
        production_len: production_len.to_vec(),
    })
}
