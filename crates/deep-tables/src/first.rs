//! Nullability, FIRST and FOLLOW over the numbered grammar.

use crate::Production;
use deep_abi::SymbolId;

/// A set of grammar symbols, as a fixed-width bitset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymSet {
    words: Vec<u64>,
}

impl SymSet {
    /// An empty set over `bits` symbols.
    pub fn new(bits: usize) -> Self {
        SymSet { words: vec![0; bits.div_ceil(64)] }
    }

    /// Insert `sym`, returning true when it was not already present.
    pub fn insert(&mut self, sym: SymbolId) -> bool {
        let i = sym as usize;
        let (w, b) = (i / 64, i % 64);
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        let mask = 1u64 << b;
        let new = self.words[w] & mask == 0;
        self.words[w] |= mask;
        new
    }

    /// Does the set contain `sym`?
    pub fn contains(&self, sym: SymbolId) -> bool {
        let i = sym as usize;
        self.words.get(i / 64).is_some_and(|w| w & (1u64 << (i % 64)) != 0)
    }

    /// Number of symbols in the set.
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// True when the set is empty.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// Add every symbol of `other`; true when the set grew.
    pub fn union(&mut self, other: &SymSet) -> bool {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        let mut changed = false;
        for (i, w) in other.words.iter().enumerate() {
            let new = self.words[i] | w;
            if new != self.words[i] {
                self.words[i] = new;
                changed = true;
            }
        }
        changed
    }

    /// The symbols in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = SymbolId> + '_ {
        self.words.iter().enumerate().flat_map(|(i, w)| {
            let mut bits = *w;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                Some((i * 64 + b as usize) as SymbolId)
            })
        })
    }
}

/// FIRST, FOLLOW and nullability for every symbol.
pub struct Analysis {
    /// Bitset width, i.e. the number of symbols.
    pub symbol_count: usize,
    /// Symbols `0..=terminal_count` are terminals, with 0 reserved for EOF.
    pub terminal_count: usize,
    /// FIRST set per symbol id.
    pub first: Vec<SymSet>,
    /// FOLLOW set per symbol id; only non-terminals carry one.
    pub follow: Vec<SymSet>,
    /// Nullability per symbol id.
    pub nullable: Vec<bool>,
}

impl Analysis {
    /// Is `sym` a terminal (EOF included)?
    pub fn is_terminal(&self, sym: SymbolId) -> bool {
        sym as usize <= self.terminal_count
    }

    /// FIRST of a symbol sequence, plus whether the whole sequence is nullable.
    pub fn first_of(&self, seq: &[SymbolId]) -> (SymSet, bool) {
        let mut set = SymSet::new(self.symbol_count);
        for &s in seq {
            set.union(&self.first[s as usize]);
            if !self.nullable[s as usize] {
                return (set, false);
            }
        }
        (set, true)
    }
}

/// Compute nullability, FIRST and FOLLOW by fixpoint.
pub fn compute(productions: &[Production], terminal_count: usize, symbol_count: usize) -> Analysis {
    let mut first = vec![SymSet::new(symbol_count); symbol_count];
    let mut nullable = vec![false; symbol_count];
    for s in 0..=terminal_count {
        first[s].insert(s as SymbolId);
    }

    loop {
        let mut changed = false;
        for p in productions {
            let mut add = SymSet::new(symbol_count);
            let mut all_null = true;
            for &s in &p.rhs {
                add.union(&first[s as usize]);
                if !nullable[s as usize] {
                    all_null = false;
                    break;
                }
            }
            changed |= first[p.lhs as usize].union(&add);
            if all_null && !nullable[p.lhs as usize] {
                nullable[p.lhs as usize] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut follow = vec![SymSet::new(symbol_count); symbol_count];
    // The start symbol is followed by end of input.
    if let Some(start) = productions.first().and_then(|p| p.rhs.first()) {
        follow[*start as usize].insert(0);
    }
    loop {
        let mut changed = false;
        for p in productions {
            for i in 0..p.rhs.len() {
                let sym = p.rhs[i];
                if sym as usize <= terminal_count {
                    continue;
                }
                let mut add = SymSet::new(symbol_count);
                let mut all_null = true;
                for &s in &p.rhs[i + 1..] {
                    add.union(&first[s as usize]);
                    if !nullable[s as usize] {
                        all_null = false;
                        break;
                    }
                }
                if all_null {
                    add.union(&follow[p.lhs as usize]);
                }
                changed |= follow[sym as usize].union(&add);
            }
        }
        if !changed {
            break;
        }
    }

    Analysis { symbol_count, terminal_count, first, follow, nullable }
}
