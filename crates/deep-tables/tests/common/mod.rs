//! A minimal LR driver that reads nothing but the flat ABI slices.
//!
//! It exists so the tests exercise the published contract, not the crate's own
//! convenience methods: a lookup is a displacement into `action_table`, a tag
//! comparison, and then the `default_reduce` fallback.

#![allow(dead_code)]

use deep_abi::{Grammar, SymbolId};
use deep_tables::Action;

/// The shift bit inside a 16-bit action payload.
pub const SHIFT_BIT: u16 = 0x8000;

/// Resolve an action from the raw ABI arrays.
pub fn abi_action(g: &Grammar, state: u32, sym: SymbolId) -> Action {
    let s = state as usize;
    let base = g.action_index[s * 2];
    let count = g.action_index[s * 2 + 1];
    if base >= 0 && count > 0 {
        let slot = base as usize + sym as usize;
        if let Some(&cell) = g.action_table.get(slot) {
            if (cell >> 16) as u16 == sym {
                let payload = cell as u16;
                return if payload == 0 {
                    Action::Error
                } else if payload & SHIFT_BIT != 0 {
                    Action::Shift(payload & !SHIFT_BIT)
                } else if payload == 1 {
                    Action::Accept
                } else {
                    Action::Reduce(payload - 1)
                };
            }
        }
    }
    match g.default_reduce[s] {
        0 => Action::Error,
        d => Action::Reduce(d - 1),
    }
}

/// Resolve a goto from the raw ABI arrays.
pub fn abi_goto(g: &Grammar, state: u32, sym: SymbolId) -> Option<u16> {
    let s = state as usize;
    let base = g.goto_index[s * 2];
    let count = g.goto_index[s * 2 + 1];
    if base < 0 || count == 0 {
        return None;
    }
    let slot = base as usize + sym as usize;
    let cell = *g.goto_table.get(slot)?;
    if (cell >> 16) as u16 != sym {
        return None;
    }
    let payload = cell as u16;
    if payload == 0 {
        None
    } else {
        Some(payload - 1)
    }
}

/// A reduce tree, so a test can check shape and not just acceptance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tree {
    Leaf(SymbolId),
    Node(SymbolId, Vec<Tree>),
}

/// Parse `tokens` over the ABI tables; `productions[i]` is `(lhs, len)`.
pub fn parse(g: &Grammar, productions: &[(SymbolId, usize)], tokens: &[SymbolId]) -> Option<Tree> {
    let mut states: Vec<u16> = vec![0];
    let mut trees: Vec<Tree> = Vec::new();
    let mut pos = 0usize;
    for _ in 0..1_000_000 {
        let state = *states.last()?;
        let lookahead = tokens.get(pos).copied().unwrap_or(0);
        match abi_action(g, state as u32, lookahead) {
            Action::Shift(next) => {
                states.push(next);
                trees.push(Tree::Leaf(lookahead));
                pos += 1;
            }
            Action::Reduce(p) => {
                let (lhs, len) = *productions.get(p as usize)?;
                if trees.len() < len {
                    return None;
                }
                let children = trees.split_off(trees.len() - len);
                for _ in 0..len {
                    states.pop()?;
                }
                let from = *states.last()?;
                let next = abi_goto(g, from as u32, lhs)?;
                states.push(next);
                trees.push(Tree::Node(lhs, children));
            }
            Action::Accept => {
                if pos != tokens.len() {
                    return None;
                }
                return trees.pop();
            }
            Action::Error => return None,
        }
    }
    None
}

/// Render a tree as `symbol(child,...)`, leaves included.
pub fn render(names: &[String], tree: &Tree) -> String {
    match tree {
        Tree::Leaf(sym) => names[*sym as usize].clone(),
        Tree::Node(sym, children) => {
            let inner: Vec<String> = children.iter().map(|c| render(names, c)).collect();
            format!("{}({})", names[*sym as usize], inner.join(","))
        }
    }
}

/// The symbol name table of a compiled grammar, indexed by symbol id.
pub fn names(c: &deep_tables::Compiled) -> Vec<String> {
    (0..c.symbol_count() as u16)
        .map(|s| c.symbol_name(s).unwrap_or("?").to_string())
        .collect()
}

/// `(lhs, length)` per production, indexed by production id.
pub fn productions(c: &deep_tables::Compiled) -> Vec<(SymbolId, usize)> {
    (0..c.production_count() as u16)
        .map(|p| (c.production(p).lhs, c.production(p).rhs.len()))
        .collect()
}
