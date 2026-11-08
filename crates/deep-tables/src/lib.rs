//! LALR(1) table generation for the deep scan, standard library only.
//!
//! The pipeline is: [`GrammarBuilder`] numbers the grammar, `first` computes
//! nullability/FIRST/FOLLOW, `lr0` builds the canonical item sets, `lalr` derives
//! lookaheads, `table` resolves actions and conflicts, and `compress` lays the
//! result out as row-displacement tables that `deep_abi::Grammar` can point at.
//!
//! Symbol numbering: 0 is end of input, `1..=terminals` are the terminals in
//! declaration order, then the non-terminals, with the augmented start symbol
//! last. A runtime that speaks `deep_abi::Grammar` must translate its own
//! [`deep_abi::EOF_SYMBOL`] to 0 before a lookup.

#![forbid(unsafe_code)]

pub mod compress;
mod error;
pub mod first;
pub mod grammars;
pub mod lalr;
pub mod lr0;
mod pack;
mod place;
pub mod report;
pub mod table;

use deep_abi::SymbolId;
use std::fmt;

pub use error::Conflict;

pub use compress::{Tables, Width};
pub use pack::Packed;
pub use report::Report;
pub use table::Action;

/// Associativity of a precedence level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Associativity {
    /// `a + b + c` groups as `(a + b) + c`.
    Left,
    /// `a = b = c` groups as `a = (b = c)`.
    Right,
    /// The operator may not be chained; `a == b == c` is an error.
    NonAssoc,
}

/// A precedence level and its associativity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Precedence {
    pub level: u8,
    pub assoc: Associativity,
}

/// One production. Index 0 is always the augmented start rule `S' -> start`.
#[derive(Clone, Debug)]
pub struct Production {
    pub lhs: SymbolId,
    pub rhs: Vec<SymbolId>,
    /// Precedence of the rule, taken from its rightmost terminal.
    pub precedence: Option<Precedence>,
    /// False only for the augmented production.
    pub user: bool,
}


/// Builds a grammar and compiles it to LALR(1) tables.
pub struct GrammarBuilder {
    name: String,
    terminals: Vec<String>,
    rules: Vec<(String, Vec<String>)>,
    precedences: Vec<(u8, Associativity, Vec<String>)>,
}

impl Default for GrammarBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GrammarBuilder {
    /// A builder with no terminals, rules or precedences.
    pub fn new() -> Self {
        GrammarBuilder {
            name: "grammar".to_string(),
            terminals: Vec::new(),
            rules: Vec::new(),
            precedences: Vec::new(),
        }
    }

    /// Set the name carried by the generated [`deep_abi::Grammar`].
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Declare a terminal. Order fixes its symbol number.
    pub fn terminal(mut self, name: &str) -> Self {
        self.terminals.push(name.to_string());
        self
    }

    /// Add a rule; `rhs` is empty for an epsilon production.
    pub fn rule(mut self, lhs: &str, rhs: &[&str]) -> Self {
        self.rules.push((lhs.to_string(), rhs.iter().map(|s| s.to_string()).collect()));
        self
    }

    /// Give a precedence level and associativity to terminals.
    pub fn precedence(mut self, level: u8, assoc: Associativity, symbols: &[&str]) -> Self {
        self.precedences
            .push((level, assoc, symbols.iter().map(|s| s.to_string()).collect()));
        self
    }

    /// Number the grammar, build the LALR(1) tables and compress them.
    pub fn build(self) -> Result<Compiled, Conflict> {
        compile(self)
    }
}

/// A compiled grammar: numbered symbols, productions, tables and a size report.
pub struct Compiled {
    name: String,
    terminals: Vec<String>,
    nonterminals: Vec<String>,
    productions: Vec<Production>,
    terminal_prec: Vec<Option<Precedence>>,
    first: first::Analysis,
    raw: table::RawTables,
    tables: Tables,
    start: SymbolId,
    augmented: SymbolId,
}

impl Compiled {
    /// The grammar's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Number of states.
    pub fn states(&self) -> usize {
        self.tables.states as usize
    }

    /// Number of terminals, end of input included.
    pub fn terminal_count(&self) -> usize {
        self.terminals.len() + 1
    }

    /// Number of non-terminals, the augmented start symbol included.
    pub fn nonterminal_count(&self) -> usize {
        self.nonterminals.len()
    }

    /// Number of symbols.
    pub fn symbol_count(&self) -> usize {
        self.tables.symbols as usize
    }

    /// Number of productions, the augmented start rule included.
    pub fn production_count(&self) -> usize {
        self.productions.len()
    }

    /// The symbol number of `name`, if the grammar defines it.
    pub fn symbol(&self, name: &str) -> Option<SymbolId> {
        if name == "$" {
            return Some(0);
        }
        self.terminals
            .iter()
            .position(|t| t == name)
            .map(|i| (i + 1) as SymbolId)
            .or_else(|| {
                self.nonterminals
                    .iter()
                    .position(|n| n == name)
                    .map(|i| (self.terminals.len() + 1 + i) as SymbolId)
            })
    }

    /// The name of a symbol number.
    pub fn symbol_name(&self, sym: SymbolId) -> Option<&str> {
        if sym == 0 {
            return Some("$");
        }
        let t = self.terminals.len() as u16;
        if sym <= t {
            self.terminals.get(sym as usize - 1).map(|s| s.as_str())
        } else {
            self.nonterminals.get((sym - t - 1) as usize).map(|s| s.as_str())
        }
    }

    /// Is `sym` a terminal (end of input included)?
    pub fn is_terminal(&self, sym: SymbolId) -> bool {
        self.first.is_terminal(sym)
    }

    /// The start symbol.
    pub fn start(&self) -> SymbolId {
        self.start
    }

    /// The augmented start symbol `S'`.
    pub fn augmented(&self) -> SymbolId {
        self.augmented
    }

    /// Production `id`.
    pub fn production(&self, id: u16) -> &Production {
        &self.productions[id as usize]
    }

    /// FIRST and FOLLOW sets, for diagnostics.
    pub fn analysis(&self) -> &first::Analysis {
        &self.first
    }

    /// The precedence declared for a terminal, if any.
    pub fn terminal_precedence(&self, sym: SymbolId) -> Option<Precedence> {
        self.terminal_prec.get(sym as usize).copied().flatten()
    }

    /// The compressed tables.
    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    /// The uncompressed reference tables, kept for round-trip checks.
    pub fn raw(&self) -> &table::RawTables {
        &self.raw
    }

    /// The tables as the flat, borrowed shape the ABI expects.
    pub fn as_grammar(&self) -> deep_abi::Grammar<'_> {
        self.tables.as_grammar(&self.name)
    }

    /// What the parser does in `state` on `sym`.
    pub fn action(&self, state: u32, sym: SymbolId) -> Action {
        self.tables.action(state, sym)
    }

    /// The state reached from `state` on a non-terminal, if any.
    pub fn goto(&self, state: u32, sym: SymbolId) -> Option<u16> {
        self.tables.goto(state, sym)
    }

    /// The production a state reduces by when the row has no entry.
    pub fn default_reduce(&self, state: u32) -> Option<u16> {
        self.tables.default_reduce(state)
    }

    /// Byte-level size accounting for the tables.
    pub fn report(&self) -> Report {
        Report::new(self)
    }
}

impl fmt::Debug for Compiled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compiled")
            .field("name", &self.name)
            .field("states", &self.states())
            .field("symbols", &self.symbol_count())
            .field("productions", &self.production_count())
            .finish()
    }
}

fn compile(b: GrammarBuilder) -> Result<Compiled, Conflict> {
    if b.rules.is_empty() {
        return Err(Conflict::NoRules);
    }
    let terminals = b.terminals;
    for (i, t) in terminals.iter().enumerate() {
        if terminals[..i].contains(t) {
            return Err(Conflict::DuplicateTerminal(t.clone()));
        }
    }
    // Non-terminals: every distinct left-hand side, in first-definition order.
    let mut nonterminals: Vec<String> = Vec::new();
    for (lhs, _) in &b.rules {
        if terminals.contains(lhs) {
            return Err(Conflict::DuplicateTerminal(lhs.clone()));
        }
        if !nonterminals.contains(lhs) {
            nonterminals.push(lhs.clone());
        }
    }
    let mut seen_rules: Vec<(String, Vec<String>)> = Vec::new();
    for (lhs, rhs) in &b.rules {
        if seen_rules.iter().any(|(l, r)| l == lhs && r == rhs) {
            return Err(Conflict::DuplicateRule(lhs.clone()));
        }
        seen_rules.push((lhs.clone(), rhs.clone()));
    }

    let t = terminals.len() as u16;
    let n = nonterminals.len() as u16;
    let augmented = t + n + 1;
    let symbol_count = t as usize + n as usize + 2;

    let mut terminal_prec: Vec<Option<Precedence>> = vec![None; t as usize + 1];
    for (level, assoc, names) in &b.precedences {
        for name in names {
            let Some(id) = terminals.iter().position(|x| x == name) else {
                return Err(Conflict::UndefinedSymbol(name.clone()));
            };
            terminal_prec[id + 1] = Some(Precedence { level: *level, assoc: *assoc });
        }
    }

    let lookup = |name: &str| -> Option<SymbolId> {
        if name == "$" || name == "EOF" {
            return Some(0);
        }
        if let Some(i) = terminals.iter().position(|x| x == name) {
            return Some(i as SymbolId + 1);
        }
        nonterminals.iter().position(|x| x == name).map(|i| t + 1 + i as SymbolId)
    };

    let mut productions: Vec<Production> = Vec::with_capacity(b.rules.len() + 1);
    productions.push(Production {
        lhs: augmented,
        rhs: vec![1],
        precedence: None,
        user: false,
    });
    for (lhs, rhs) in &b.rules {
        let lhs_id = lookup(lhs).ok_or_else(|| Conflict::UndefinedSymbol(lhs.clone()))?;
        let mut ids = Vec::with_capacity(rhs.len());
        for name in rhs {
            ids.push(lookup(name).ok_or_else(|| Conflict::UndefinedSymbol(name.clone()))?);
        }
        let precedence = ids
            .iter()
            .rev()
            .find(|id| **id >= 1 && **id <= t && terminal_prec[**id as usize].is_some())
            .and_then(|id| terminal_prec[*id as usize]);
        productions.push(Production { lhs: lhs_id, rhs: ids, precedence, user: true });
    }
    // The augmented rule's sole right-hand side is the first declared rule.
    let start = productions[1].lhs;
    productions[0].rhs = vec![start];

    let analysis = first::compute(&productions, t as usize, symbol_count);
    let automaton = lr0::build(&productions);
    let raw = table::build(&productions, &analysis, &automaton, &terminal_prec, &terminals)?;
    let lengths: Vec<u8> = productions
        .iter()
        .map(|p| {
            if p.rhs.len() > u8::MAX as usize {
                Err(Conflict::TooLarge(format!("production {} has {} items", p.lhs, p.rhs.len())))
            } else {
                Ok(p.rhs.len() as u8)
            }
        })
        .collect::<Result<_, _>>()?;
    let tables = compress::compress(&raw, symbol_count, &lengths)?;

    Ok(Compiled {
        name: b.name,
        terminals,
        nonterminals,
        productions,
        terminal_prec,
        first: analysis,
        raw,
        tables,
        start,
        augmented,
    })
}
