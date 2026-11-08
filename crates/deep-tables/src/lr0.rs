//! LR(0) item sets: closure, goto and the canonical collection.

use crate::Production;
use deep_abi::SymbolId;
use std::collections::{BTreeMap, BTreeSet};

/// An LR(0) item: a production and a dot position in its right-hand side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Item {
    pub production: u32,
    pub dot: u32,
}

/// The LR(0) automaton. Kernels are the canonical identity of a state; closures
/// are cached because every later pass needs them.
pub struct Automaton {
    /// Kernel items of each state, sorted.
    pub kernels: Vec<Vec<Item>>,
    /// Full closure of each state, sorted.
    pub closures: Vec<Vec<Item>>,
    /// Transitions of each state, sorted by symbol.
    pub transitions: Vec<Vec<(SymbolId, u32)>>,
    /// Position of each kernel item inside its state, for lookahead propagation.
    pub kernel_pos: Vec<BTreeMap<Item, usize>>,
}

impl Automaton {
    /// The state reached from `state` on `sym`, if any.
    pub fn goto(&self, state: u32, sym: SymbolId) -> Option<u32> {
        let row = &self.transitions[state as usize];
        row.binary_search_by_key(&sym, |e| e.0).ok().map(|i| row[i].1)
    }

    /// Number of states.
    pub fn len(&self) -> usize {
        self.kernels.len()
    }

    /// True when the automaton has no states, which only the empty grammar does.
    pub fn is_empty(&self) -> bool {
        self.kernels.is_empty()
    }
}

/// Closure of a kernel item set under the LR(0) rules.
pub fn closure(productions: &[Production], kernel: &[Item]) -> Vec<Item> {
    let mut seen: BTreeSet<Item> = kernel.iter().copied().collect();
    let mut work: Vec<Item> = seen.iter().copied().collect();
    while let Some(item) = work.pop() {
        let p = &productions[item.production as usize];
        let Some(&sym) = p.rhs.get(item.dot as usize) else { continue };
        for (pi, prod) in productions.iter().enumerate() {
            if prod.lhs == sym {
                let next = Item { production: pi as u32, dot: 0 };
                if seen.insert(next) {
                    work.push(next);
                }
            }
        }
    }
    seen.into_iter().collect()
}

/// The canonical collection of LR(0) item sets.
pub fn build(productions: &[Production]) -> Automaton {
    let start = Item { production: 0, dot: 0 };
    let mut kernels: Vec<Vec<Item>> = vec![vec![start]];
    let mut index: BTreeMap<Vec<Item>, u32> = BTreeMap::new();
    index.insert(vec![start], 0);
    let mut closures: Vec<Vec<Item>> = Vec::new();
    let mut transitions: Vec<Vec<(SymbolId, u32)>> = Vec::new();

    let mut state = 0usize;
    while state < kernels.len() {
        let closed = closure(productions, &kernels[state]);
        let mut moves: BTreeMap<SymbolId, Vec<Item>> = BTreeMap::new();
        for item in &closed {
            let p = &productions[item.production as usize];
            if let Some(&sym) = p.rhs.get(item.dot as usize) {
                moves
                    .entry(sym)
                    .or_default()
                    .push(Item { production: item.production, dot: item.dot + 1 });
            }
        }
        let mut row = Vec::with_capacity(moves.len());
        for (sym, mut target) in moves {
            target.sort();
            target.dedup();
            let id = match index.get(&target) {
                Some(&id) => id,
                None => {
                    let id = kernels.len() as u32;
                    index.insert(target.clone(), id);
                    kernels.push(target);
                    id
                }
            };
            row.push((sym, id));
        }
        closures.push(closed);
        transitions.push(row);
        state += 1;
    }

    let kernel_pos = kernels
        .iter()
        .map(|k| k.iter().enumerate().map(|(i, it)| (*it, i)).collect())
        .collect();
    Automaton { kernels, closures, transitions, kernel_pos }
}
