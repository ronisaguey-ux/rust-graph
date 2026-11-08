//! The narrow on-disk form: u16 cells and u8/u16 indices.

use crate::compress::{Tables, Width};

/// The narrow on-disk form of the tables.
pub struct Packed {
    pub states: u32,
    pub symbols: u32,
    pub idx_width: u8,
    pub ent_width: u8,
    pub action_index: Vec<u8>,
    pub action_table: Vec<u8>,
    pub goto_index: Vec<u8>,
    pub goto_table: Vec<u8>,
    pub default_reduce: Vec<u8>,
    pub production_len: Vec<u8>,
}

impl Packed {
    /// Total bytes of the packed form.
    pub fn bytes(&self) -> usize {
        self.action_index.len()
            + self.action_table.len()
            + self.goto_index.len()
            + self.goto_table.len()
            + self.default_reduce.len()
            + self.production_len.len()
    }

    /// Rebuild the canonical tables, proving the packing is lossless.
    pub fn decode(&self) -> Tables {
        let width = if self.ent_width == 2 { Width::Half } else { Width::Word };
        let states = self.states;
        Tables {
            states,
            symbols: self.symbols,
            width,
            action_index: unpack_index(&self.action_index, self.idx_width as usize, states as usize * 2),
            action_table: unpack_entries(&self.action_table, width),
            goto_index: unpack_index(&self.goto_index, self.idx_width as usize, states as usize * 2),
            goto_table: unpack_entries(&self.goto_table, width),
            default_reduce: self
                .default_reduce
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
            production_len: self.production_len.clone(),
        }
    }
}

pub(crate) fn narrow_index_width(a: &[i32], b: &[i32]) -> u8 {
    let max = a.iter().chain(b).map(|v| v.unsigned_abs()).max().unwrap_or(0);
    if max <= 0x7F {
        1
    } else if max <= 0x7FFF {
        2
    } else {
        4
    }
}

pub(crate) fn pack_index(values: &[i32], width: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * width);
    for v in values {
        let b = v.to_le_bytes();
        out.extend_from_slice(&b[..width]);
    }
    out
}

fn unpack_index(bytes: &[u8], width: usize, count: usize) -> Vec<i32> {
    let mut out = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(width).take(count) {
        let mut buf = [0u8; 4];
        buf[..width].copy_from_slice(chunk);
        let sign = width < 4 && chunk[width - 1] & 0x80 != 0;
        let v = i32::from_le_bytes(buf);
        out.push(if sign && width < 4 {
            v | (!0i32 << (width * 8))
        } else {
            v
        });
    }
    out
}

pub(crate) fn pack_entries(cells: &[u32], width: Width) -> Vec<u8> {
    let mut out = Vec::with_capacity(cells.len() * width.entry_bytes());
    for &cell in cells {
        match width {
            Width::Half => {
                let payload = cell as u16;
                let narrow_payload = if payload & 0x8000 != 0 {
                    0x80 | (payload & 0x7F)
                } else {
                    payload & 0x7F
                };
                let narrow = ((Width::Word.tag(cell) & 0xFF) << 8) | narrow_payload;
                out.extend_from_slice(&narrow.to_le_bytes());
            }
            Width::Word => out.extend_from_slice(&cell.to_le_bytes()),
        }
    }
    out
}

fn unpack_entries(bytes: &[u8], width: Width) -> Vec<u32> {
    match width {
        Width::Half => bytes
            .chunks_exact(2)
            .map(|c| {
                let narrow = u16::from_le_bytes([c[0], c[1]]);
                let payload = narrow & 0xFF;
                let wide = if payload & 0x80 != 0 { 0x8000 | (payload & 0x7F) } else { payload };
                Width::Word.cell((narrow >> 8) & 0xFF, wide)
            })
            .collect(),
        Width::Word => bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    }
}
