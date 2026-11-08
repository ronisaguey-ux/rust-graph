//! The other way to compile the same DSL: LALR(1) tables instead of one parser
//! function per rule.
//!
//! The DSL is an EBNF. The generator is a BNF one, so every repetition and
//! option becomes an anonymous non-terminal (`x?` is `A: | x`, `x*` is
//! `A: | A x`, `x+` is `A: x | A x`, `x % s` is `A: | A s x | x`), every nested
//! group becomes one of its own, and what is left is a plain production list
//! that [`deep_tables::GrammarBuilder`] numbers and compiles.
//!
//! Two things follow from choosing LR over recursive descent, and both are why
//! this module exists rather than reusing `check`:
//!
//! - **Left recursion is legal.** `expr: expr "+" term | term` is how an LR
//!   grammar says "left associative", so the left-recursion check of the
//!   recursive path does not apply, and is not run here.
//! - **Nothing is decided by lookahead.** A choice is resolved by the table,
//!   so `&x` predicates, `precedence` tables and `rule@level` have no meaning
//!   here; they are refused by name rather than silently ignored.
//!
//! A conflict is a build failure, not a warning: the message names the state,
//! the terminal and both actions, with the productions written as the grammar
//! author wrote them.
//!
//! The emitted code is a `deep_abi::Grammar` static plus one `u16` constant per
//! visible kind, numbered with the grammar's own symbols (the runtime stores
//! symbol ids in nodes). Hidden rules (a leading `_`) get no constant: they
//! make no node.

mod emit;
mod lower;

#[cfg(test)]
mod tests;

pub use emit::emit;

use deep_tables::Compiled;

use crate::read::{Expr, Grammar, Node};
use crate::Error;

use lower::Lowering;

/// The runtime the DSL assumes when it names none: the recursive parser
/// functions' runtime, which reads no tables.
pub const DEFAULT_RUNTIME: &str = "crate::deep";

/// Whether the grammar names a runtime that can drive tables. The built-in
/// default cannot, so a table grammar has to say which runtime reads them.
pub fn runtime_reads_tables(grammar: &Grammar) -> bool {
    grammar.runtime != DEFAULT_RUNTIME
}

/// A lexer mode switch is the step lexer's: the table driver never calls
/// `Language::lex`, so a table grammar that declared one would lose it without
/// a word. Refused by name here, and again by [`emit`] for `mode = tables;`.
pub fn unsupported_modes(grammar: &Grammar) -> Option<Error> {
    if grammar.modes.is_empty() && grammar.switches.is_empty() {
        return None;
    }
    let spot = grammar.modes.first().map(|mode| mode.spot).or_else(|| grammar.switches.first().map(|switch| switch.spot)).flatten();
    Some(Error::at(spot, "a lexer mode belongs to the step lexer, which the table driver never runs: use `mode = recursive;`"))
}

/// What the table path refuses, and why. `None` when the grammar is expressible.
pub fn unsupported(grammar: &Grammar) -> Option<Error> {
    if let Some(error) = unsupported_modes(grammar) {
        return Some(error);
    }
    if let Some(layout) = &grammar.layout {
        return Some(Error::at(
            layout.spot,
            "`layout` belongs to the recursive runtime's lexer, which inserts the tokens: a layout grammar needs `mode = recursive;`",
        ));
    }
    grammar.rules.iter().find_map(|rule| unsupported_in(&rule.body)).or_else(|| runtime_error(grammar))
}

/// A table grammar has to name the runtime that drives the tables.
fn runtime_error(grammar: &Grammar) -> Option<Error> {
    if runtime_reads_tables(grammar) {
        return None;
    }
    Some(Error::at(
        None,
        format!(
            "the default runtime (`{DEFAULT_RUNTIME}`) runs parser functions and reads no tables: \
             name the runtime that does, as in `runtime = deep_rt;`, or use `mode = recursive;`"
        ),
    ))
}

fn unsupported_in(node: &Node) -> Option<Error> {
    let spot = node.spot;
    match &node.expr {
        Expr::Pred(_) => Some(Error::at(spot, "a predicate `&x` decides by lookahead, which a table cannot: use `mode = recursive;`")),
        Expr::Table(_) => Some(Error::at(spot, "a `precedence` table is the recursive path's Pratt loop, not a table grammar")),
        Expr::Level(rule, label) => Some(Error::at(spot, format!("`{rule}@{label}` enters a precedence table, which a table grammar does not have"))),
        Expr::Seq(items) | Expr::Choice(items) => items.iter().find_map(unsupported_in),
        Expr::Opt(inner) | Expr::Star(inner) | Expr::Plus(inner) => unsupported_in(inner),
        Expr::Sep(inner, separator, _) => unsupported_in(inner).or_else(|| unsupported_in(separator)),
        _ => None,
    }
}

/// A grammar compiled to tables, with the names the emitted code needs.
pub struct Tables {
    pub compiled: Compiled,
    /// Symbols that make no node: hidden rules and the anonymous non-terminals.
    pub hidden: Vec<u16>,
    /// `(constant, symbol)` for every kind that does make a node.
    pub kinds: Vec<(String, u16)>,
    pub root: u16,
}

impl Tables {
    /// Bytes the emitted slices occupy through the ABI.
    pub fn bytes(&self) -> usize {
        self.compiled.runtime_tables().bytes()
    }
}

/// Lower the DSL to a production list and compile it.
pub fn build(grammar: &Grammar) -> Result<Tables, Error> {
    let mut lower = Lowering::new(grammar)?;
    let rules = lower.productions()?;
    let mut builder = deep_tables::GrammarBuilder::new().name(&grammar.language);
    for terminal in &lower.terminals {
        builder = builder.terminal(terminal);
    }
    for (lhs, rhs, _) in &rules {
        let rhs: Vec<&str> = rhs.iter().map(String::as_str).collect();
        builder = builder.rule(lhs, &rhs);
    }
    let compiled = builder.build().map_err(|conflict| emit::conflict_error(&rules, conflict))?;

    let mut hidden = Vec::new();
    let mut kinds = Vec::new();
    for symbol in 0..compiled.symbol_count() as u16 {
        // Symbol 0 is end of input: a lookahead, never a kind of node.
        if symbol == 0 {
            continue;
        }
        let Some(name) = compiled.symbol_name(symbol) else { continue };
        if is_hidden(name) {
            hidden.push(symbol);
            continue;
        }
        // Terminals keep the constant the grammar author wrote; every
        // non-terminal's constant is its name uppercased.
        let constant = grammar
            .tokens
            .iter()
            .find(|token| token.text == name)
            .map(|token| token.constant.clone())
            .unwrap_or_else(|| name.to_ascii_uppercase());
        kinds.push((constant, symbol));
    }
    let root = compiled
        .symbol(&grammar.rules[0].name)
        .ok_or_else(|| Error::at(grammar.rules[0].spot, "the root rule did not reach the compiled grammar"))?;
    Ok(Tables { compiled, hidden, kinds, root })
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('_')
}
