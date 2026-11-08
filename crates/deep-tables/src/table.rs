//! Action and goto construction, with precedence-based conflict resolution.

use crate::first::{Analysis, SymSet};
use crate::lr0::{Automaton, Item};
use crate::{lalr, Associativity, Conflict, Precedence, Production};
use deep_abi::SymbolId;
use std::collections::BTreeMap;

/// What the parser does in a state on a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Push the terminal and move to the given state.
    Shift(u16),
    /// Reduce by the given production id.
    Reduce(u16),
    /// Reduce by the augmented start production: the parse is complete.
    Accept,
    /// The terminal cannot appear here.
    Error,
}

/// The uncompressed tables: one complete row per state, plus defaults.
pub struct RawTables {
    pub action: Vec<Vec<(SymbolId, Action)>>,
    pub goto: Vec<Vec<(SymbolId, u16)>>,
    pub default_reduce: Vec<Option<u16>>,
}

impl RawTables {
    /// Look up an action the way the parser would, default reduction included.
    pub fn action(&self, state: u32, sym: SymbolId) -> Action {
        let row = &self.action[state as usize];
        if let Ok(i) = row.binary_search_by_key(&sym, |e| e.0) {
            return row[i].1;
        }
        match self.default_reduce[state as usize] {
            Some(p) => Action::Reduce(p),
            None => Action::Error,
        }
    }
}

/// Build actions and gotos, resolving conflicts by precedence where declared.
pub fn build(
    productions: &[Production],
    first: &Analysis,
    automaton: &Automaton,
    terminal_prec: &[Option<Precedence>],
    terminals: &[String],
) -> Result<RawTables, Conflict> {
    let la = lalr::lookaheads(productions, first, automaton, 0);
    let mut action: Vec<Vec<(SymbolId, Action)>> = Vec::with_capacity(automaton.len());
    let mut goto: Vec<Vec<(SymbolId, u16)>> = Vec::with_capacity(automaton.len());

    for s in 0..automaton.len() {
        let seeds: Vec<(Item, SymSet)> = automaton.kernels[s]
            .iter()
            .enumerate()
            .map(|(ki, item)| (*item, la[s][ki].clone()))
            .collect();
        let closed = lalr::closure_la(productions, first, first.symbol_count, &seeds);
        let mut row: BTreeMap<SymbolId, Action> = BTreeMap::new();
        for (item, look) in &closed {
            let p = &productions[item.production as usize];
            match p.rhs.get(item.dot as usize) {
                Some(&sym) if first.is_terminal(sym) => {
                    let Some(target) = automaton.goto(s as u32, sym) else { continue };
                    insert(
                        &mut row,
                        sym,
                        Action::Shift(target as u16),
                        s as u32,
                        productions,
                        terminal_prec,
                        terminals,
                    )?;
                }
                Some(_) => {}
                None => {
                    let act = if !p.user {
                        Action::Accept
                    } else {
                        Action::Reduce(item.production as u16)
                    };
                    for a in look.iter() {
                        if a as usize > first.terminal_count {
                            continue;
                        }
                        insert(
                            &mut row,
                            a,
                            act,
                            s as u32,
                            productions,
                            terminal_prec,
                            terminals,
                        )?;
                    }
                }
            }
        }
        action.push(row.into_iter().collect());
        goto.push(
            automaton.transitions[s]
                .iter()
                .filter(|(sym, _)| !first.is_terminal(*sym))
                .map(|(sym, st)| (*sym, *st as u16))
                .collect(),
        );
    }

    let default_reduce = choose_defaults(&action);
    Ok(RawTables { action, goto, default_reduce })
}

/// Insert an action, resolving a clash with what is already there.
fn insert(
    row: &mut BTreeMap<SymbolId, Action>,
    sym: SymbolId,
    new: Action,
    state: u32,
    productions: &[Production],
    terminal_prec: &[Option<Precedence>],
    terminals: &[String],
) -> Result<(), Conflict> {
    let Some(&old) = row.get(&sym) else {
        row.insert(sym, new);
        return Ok(());
    };
    if old == new {
        return Ok(());
    }
    let name = if sym == 0 {
        "$".to_string()
    } else {
        terminals
            .get(sym as usize - 1)
            .cloned()
            .unwrap_or_else(|| format!("symbol {sym}"))
    };
    match (old, new) {
        (Action::Shift(s), Action::Reduce(p)) | (Action::Reduce(p), Action::Shift(s)) => {
            match resolve(productions[p as usize].precedence, terminal_prec[sym as usize]) {
                Some(Action::Reduce(_)) => {
                    row.insert(sym, Action::Reduce(p));
                }
                Some(Action::Shift(_)) => {
                    row.insert(sym, Action::Shift(s));
                }
                Some(Action::Error) => {
                    row.insert(sym, Action::Error);
                }
                _ => {
                    return Err(Conflict::ShiftReduce {
                        state,
                        terminal: sym,
                        terminal_name: name,
                        shift_to: s as u32,
                        reduce_by: p,
                    })
                }
            }
        }
        (Action::Reduce(a), Action::Reduce(b)) => {
            return Err(Conflict::ReduceReduce {
                state,
                terminal: sym,
                terminal_name: name,
                rule_a: a.min(b),
                rule_b: a.max(b),
            })
        }
        (Action::Accept, Action::Reduce(p)) | (Action::Reduce(p), Action::Accept) => {
            return Err(Conflict::ReduceReduce {
                state,
                terminal: sym,
                terminal_name: name,
                rule_a: 0,
                rule_b: p,
            })
        }
        (Action::Accept, Action::Shift(s)) | (Action::Shift(s), Action::Accept) => {
            return Err(Conflict::ShiftReduce {
                state,
                terminal: sym,
                terminal_name: name,
                shift_to: s as u32,
                reduce_by: 0,
            })
        }
        (Action::Shift(a), Action::Shift(b)) => {
            return Err(Conflict::ShiftReduce {
                state,
                terminal: sym,
                terminal_name: name,
                shift_to: a.max(b) as u32,
                reduce_by: 0,
            })
        }
        (Action::Accept, Action::Accept) => {}
        (Action::Error, _) | (_, Action::Error) => {}
    }
    Ok(())
}

/// Decide a shift/reduce clash from the rule and terminal precedences.
fn resolve(rule: Option<Precedence>, token: Option<Precedence>) -> Option<Action> {
    match (rule, token) {
        (Some(r), Some(t)) if r.level > t.level => Some(Action::Reduce(0)),
        (Some(r), Some(t)) if t.level > r.level => Some(Action::Shift(0)),
        (Some(_), Some(t)) => match t.assoc {
            Associativity::Left => Some(Action::Reduce(0)),
            Associativity::Right => Some(Action::Shift(0)),
            Associativity::NonAssoc => Some(Action::Error),
        },
        _ => None,
    }
}

/// A state defaults to its most frequent reduction, ties going to the lower
/// production id. States with no reduction have no default.
fn choose_defaults(action: &[Vec<(SymbolId, Action)>]) -> Vec<Option<u16>> {
    action
        .iter()
        .map(|row| {
            let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
            for (_, act) in row {
                if let Action::Reduce(p) = act {
                    *counts.entry(*p).or_default() += 1;
                }
            }
            let mut best: Option<(u16, usize)> = None;
            for (p, c) in counts {
                match best {
                    Some((_, bc)) if c <= bc => {}
                    _ => best = Some((p, c)),
                }
            }
            best.map(|(p, _)| p)
        })
        .collect()
}
