//! The failures a build can report.

use deep_abi::SymbolId;
use std::fmt;

/// Every way a build can fail: a table conflict, or a grammar that cannot be
/// numbered. The name is the contract's, not a restriction to conflicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// A state can both shift and reduce on one terminal, with no precedence to
    /// decide it.
    ShiftReduce {
        state: u32,
        terminal: SymbolId,
        terminal_name: String,
        shift_to: u32,
        reduce_by: u16,
    },
    /// A state can reduce by two different productions on one terminal.
    ReduceReduce {
        state: u32,
        terminal: SymbolId,
        terminal_name: String,
        rule_a: u16,
        rule_b: u16,
    },
    /// A name appears on a right-hand side but was never declared or defined.
    UndefinedSymbol(String),
    /// `.terminal` was called twice with the same name.
    DuplicateTerminal(String),
    /// A non-terminal is defined twice as the same rule.
    DuplicateRule(String),
    /// No rules were added.
    NoRules,
    /// The grammar does not fit the `u16` ABI or the tagged entry encoding.
    TooLarge(String),
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Conflict::ShiftReduce { state, terminal, terminal_name, shift_to, reduce_by } => write!(
                f,
                "shift/reduce conflict in state {state} on terminal {terminal} ({terminal_name:?}): \
                 shift to state {shift_to} or reduce by production {reduce_by}"
            ),
            Conflict::ReduceReduce { state, terminal, terminal_name, rule_a, rule_b } => write!(
                f,
                "reduce/reduce conflict in state {state} on terminal {terminal} ({terminal_name:?}): \
                 reduce by production {rule_a} or {rule_b}"
            ),
            Conflict::UndefinedSymbol(name) => write!(f, "undefined symbol {name:?}"),
            Conflict::DuplicateTerminal(name) => write!(f, "terminal {name:?} declared twice"),
            Conflict::DuplicateRule(name) => write!(f, "rule {name:?} defined twice"),
            Conflict::NoRules => write!(f, "the grammar has no rules"),
            Conflict::TooLarge(what) => write!(f, "grammar too large: {what}"),
        }
    }
}

impl std::error::Error for Conflict {}
