//! LALR(1) lookaheads by spontaneous generation and propagation.
//!
//! This is Algorithm 4.63 of the Dragon book, not the DeRemer/Pennello lane
//! table: a dummy terminal `#` is pushed through each kernel item's closure, the
//! lookaheads it names are recorded as spontaneous, and the edges it crosses
//! become propagation links that are iterated to a fixpoint. The result is the
//! same LALR(1) lookahead set, derived from an LR(0) automaton.

use crate::first::{Analysis, SymSet};
use crate::lr0::{Automaton, Item};
use crate::Production;
use deep_abi::SymbolId;
use std::collections::BTreeMap;

/// Closure of a set of seeds, carrying lookahead sets along the way.
pub(crate) fn closure_la(
    productions: &[Production],
    first: &Analysis,
    symbol_count: usize,
    seeds: &[(Item, SymSet)],
) -> BTreeMap<Item, SymSet> {
    let mut map: BTreeMap<Item, SymSet> = BTreeMap::new();
    let mut work: Vec<Item> = Vec::new();
    for (item, set) in seeds {
        let entry = map
            .entry(*item)
            .or_insert_with(|| SymSet::new(symbol_count + 1));
        if entry.union(set) {
            work.push(*item);
        } else if !work.contains(item) {
            work.push(*item);
        }
    }
    while let Some(item) = work.pop() {
        let Some(set) = map.get(&item).cloned() else { continue };
        let p = &productions[item.production as usize];
        let Some(&sym) = p.rhs.get(item.dot as usize) else { continue };
        let (mut la, nullable) = first.first_of(&p.rhs[item.dot as usize + 1..]);
        if nullable {
            la.union(&set);
        }
        for (pi, prod) in productions.iter().enumerate() {
            if prod.lhs != sym {
                continue;
            }
            let next = Item { production: pi as u32, dot: 0 };
            let entry = map
                .entry(next)
                .or_insert_with(|| SymSet::new(symbol_count + 1));
            if entry.union(&la) && !work.contains(&next) {
                work.push(next);
            }
        }
    }
    map
}

/// One edge of the lookahead propagation graph.
#[derive(Clone, Copy, Debug)]
struct Edge {
    from_state: u32,
    from_kernel: usize,
    to_state: u32,
    to_kernel: usize,
}

/// LALR(1) lookaheads, aligned with `automaton.kernels`.
pub fn lookaheads(
    productions: &[Production],
    first: &Analysis,
    automaton: &Automaton,
    eof: SymbolId,
) -> Vec<Vec<SymSet>> {
    let symbol_count = first.symbol_count;
    let dummy = symbol_count as SymbolId;
    let mut la: Vec<Vec<SymSet>> = automaton
        .kernels
        .iter()
        .map(|k| vec![SymSet::new(symbol_count + 1); k.len()])
        .collect();
    // The start item carries the end-of-input lookahead from the outset.
    la[0][0].insert(eof);

    let mut edges: Vec<Edge> = Vec::new();
    for (s, kernel) in automaton.kernels.iter().enumerate() {
        for (ki, &item) in kernel.iter().enumerate() {
            let mut seed = SymSet::new(symbol_count + 1);
            seed.insert(dummy);
            let closed = closure_la(productions, first, symbol_count, &[(item, seed)]);
            for (probe, set) in &closed {
                let p = &productions[probe.production as usize];
                let Some(&sym) = p.rhs.get(probe.dot as usize) else { continue };
                // Terminals propagate too: a shifted item still carries the
                // lookaheads its later reductions will need.
                let Some(j) = automaton.goto(s as u32, sym) else { continue };
                let target = Item { production: probe.production, dot: probe.dot + 1 };
                let Some(&ti) = automaton.kernel_pos[j as usize].get(&target) else {
                    continue;
                };
                for a in set.iter() {
                    if a != dummy {
                        la[j as usize][ti].insert(a);
                    }
                }
                if set.contains(dummy) {
                    edges.push(Edge {
                        from_state: s as u32,
                        from_kernel: ki,
                        to_state: j,
                        to_kernel: ti,
                    });
                }
            }
        }
    }

    loop {
        let mut changed = false;
        for e in &edges {
            let source = la[e.from_state as usize][e.from_kernel].clone();
            changed |= la[e.to_state as usize][e.to_kernel].union(&source);
        }
        if !changed {
            break;
        }
    }
    la
}
