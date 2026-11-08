//! Row displacement: the packing step behind the compressed tables.

use deep_abi::SymbolId;

/// Row displacement: place each row at the lowest base where none of its cells
/// collide, longest rows first so the dense ones get the cheap alignments.
pub(crate) fn place(rows: &[Vec<(SymbolId, u32)>]) -> (Vec<i32>, Vec<u32>) {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(rows[i].len()), i));
    let mut pairs = vec![0i32; rows.len() * 2];
    let mut used: Vec<bool> = Vec::new();
    let mut table: Vec<u32> = Vec::new();
    // Rows must not share a base: a cell carries only the symbol it belongs to,
    // so two rows at the same base would answer each other's lookups with a tag
    // that matches.
    let mut bases: Vec<bool> = Vec::new();

    for &r in &order {
        let row = &rows[r];
        if row.is_empty() {
            pairs[r * 2] = -1;
            pairs[r * 2 + 1] = 0;
            continue;
        }
        let syms: Vec<usize> = row.iter().map(|(s, _)| *s as usize).collect();
        let mut base = 0usize;
        'search: loop {
            if bases.get(base).copied().unwrap_or(false) {
                base += 1;
                continue;
            }
            for &s in &syms {
                if used.get(base + s).copied().unwrap_or(false) {
                    base += 1;
                    continue 'search;
                }
            }
            break;
        }
        if bases.len() <= base {
            bases.resize(base + 1, false);
        }
        bases[base] = true;
        let top = base + syms.iter().copied().max().unwrap_or(0);
        if used.len() <= top {
            used.resize(top + 1, false);
        }
        if table.len() <= top {
            table.resize(top + 1, 0);
        }
        for (i, &s) in syms.iter().enumerate() {
            used[base + s] = true;
            table[base + s] = row[i].1;
        }
        pairs[r * 2] = base as i32;
        pairs[r * 2 + 1] = row.len() as i32;
    }
    (pairs, table)
}
