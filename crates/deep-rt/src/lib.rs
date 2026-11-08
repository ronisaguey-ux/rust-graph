//! The runtime of the deep scan — one crate, two engines, one tree.
//!
//! The deep parser had two runtimes: a hand-written one in `rust-graph`'s
//! `src/deep` and this crate. They are now one crate. Everything a grammar can
//! name lives here:
//!
//! | module | what it is | who calls it |
//! |---|---|---|
//! | [`syntax`] | kinds, the per-language kind table, [`Layout`] | the `grammar!` output, every language |
//! | [`lexer`] | [`Cursor`], [`Tokens`], layout inference | every `lex` function |
//! | [`pairs`] | the delimiter pass: openers paired with closers | [`Workspace::parse`] |
//! | [`parser`] | the descent driver, and [`Workspace`] | the `grammar!` output |
//! | [`tree`] | the flat CST: one preorder `Vec<Node>` | every consumer |
//! | [`budget`] | steps, depth, clock, [`Outcome`] | [`Workspace::parse`] |
//! | [`json`] | JSON written by hand, no tables needed | a caller with no generator |
//! | [`cst`], [`driver`], [`language_tables`] | the GLR engine for generated tables | [`Workspace::parse`] |
//!
//! # Two engines, one entry point
//!
//! [`Workspace::parse`] takes a byte slice, a [`Language`] and a [`Budget`], and
//! returns an [`Outcome`] while the tree lands in [`Workspace::tree`]. Which engine
//! runs is a property of the language:
//!
//! * a `grammar!` language has no tables and parses on the descent driver — the
//!   one whose API the macro generates calls to ([`Parser`]);
//! * a language built by a table generator carries a [`TableGrammar`] and parses
//!   on the GLR driver in [`driver`], whose arena is flattened into the same
//!   [`Tree`].
//!
//! Either way the caller reads the same tokens, the same [`Tree`] node layout
//! (`kind`, `flags`, `start..end`, `subtree_end`) and the same [`Outcome`] fields.
//! `UNIFICATION.md` says why the split is not a merging of the two algorithms: a
//! `grammar!` grammar is a choice of recursive descent functions, not a grammar
//! from which LR tables can be derived, so the tables cannot be generated from it
//! without a second generator.
//!
//! # The GLR engine
//!
//! Built against [`deep_abi`], which is frozen: the node is 20 bytes, arena
//! indices are `u32`, states are `u16`, and the grammar is flat parallel arrays.
//! Nothing here links an external crate, nothing recurses on input depth, and
//! the parse stack is a `Vec` of `(state, node)` entries — a
//! [`stack::Gss`] once two versions share a prefix, which is a graph of the same
//! entries with parent links.
//!
//! # Reading the tables
//!
//! The ABI fixes the arrays but not the packing; [`table`] documents it:
//!
//! ```text
//! action_index: 2 x i32 per state = (row, default)
//!     row >= 0  -> action_table[row + terminal] is the cell
//!     row <  0  -> no row; only the default can apply
//!     default < 0 -> reduce by production (-default - 1)
//! action_table cell: 0 none, 1..=states shift, 0x4000_0000 accept,
//!                    0x8000_0000 | production reduce
//! goto_index:   2 x i32 per state = (row, reserved)
//!     goto_table[row + nonterminal] = state + 1, or 0
//! default_reduce[state] names the symbol a reduction produces; the ABI has no
//! per-production LHS, so that slice is what gives a node its kind.
//! ```
//!
//! State 0 is the start state. A grammar whose `action_index` is empty is
//! [`Fatal::NoTable`].
//!
//! # Recovery
//!
//! [`deep_abi::Recovery`] says what the parser may do when a state has no
//! action for a terminal:
//!
//! * **Insert** — the driver asks [`table::expected`] for the shortest chain of
//!   gotos ending in a shiftable terminal and inserts those symbols as
//!   zero-width `FLAG_MISSING` nodes. This is bounded by
//!   [`driver::Config::max_inserts`] consecutive inserts per position.
//! * **SkipTo** — tokens are skipped until the state has an action again. A run
//!   of skipped tokens becomes one `FLAG_ERROR` node, so the tree still covers
//!   the source range.
//! * **Abandon** — the cascade rule: if the failing token opens a balanced
//!   group, the driver finds the matching close (from the lexer's pair table,
//!   or by scanning [`driver::Config::source`]) and wraps the whole group as one
//!   ERROR node, then resumes after it. The enclosing production still
//!   reduces, so the root survives.
//!
//! Every failed state forks: one version inserts, one skips. The version set is
//! capped at six live plus four overflow versions; past that the highest-cost
//! branch is dropped and [`driver::Stats::version_drops`] counts it.

pub mod budget;
pub mod cst;
pub mod driver;
pub mod incremental;
pub mod json;
pub mod language_tables;
pub mod lexer;
pub mod pairs;
pub mod parser;
pub mod recovery;
pub mod stack;
pub mod syntax;
pub mod table;
pub mod tree;
pub mod version;

pub use budget::{Budget, Exhausted, Outcome};
pub use cst::{depth, preorder, Arena};
pub use driver::{parse, Config, Stats};
pub use language_tables::TableGrammar;
pub use lexer::{lex, Cursor, Tokens};
pub use parser::{ParseError, Parser, Workspace};
pub use stack::{Base, Entry, Gss, GssId, MAX_BRANCHES};
pub use syntax::{Class, Kind, KindInfo, Language, Layout};
pub use table::{action, goto, lhs, production_len, reduce_cell, shift_cell, Action, Expected, Step};
pub use tree::{Event, Tree};
pub use version::{Lane, Version, MAX_LIVE, MAX_OVERFLOW, MAX_VERSIONS};

pub use deep_abi::{
    Cst, Fatal, Grammar, Lexer, Node, NodeId, Offset, Recovery, StateId, SymbolId, Token,
    EOF_SYMBOL, ERROR_SYMBOL, FLAG_AMBIGUOUS, FLAG_ERROR, FLAG_EXTRA, FLAG_MISSING, NONE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_node_layout_is_pinned() {
        // The ABI's doc comment says sixteen bytes, but the six fields it names
        // add up to twenty: 2 + 2 + 4 + 4 + 4 + 4. The layout is what the other
        // crates compile against, so that is what is pinned here.
        assert_eq!(std::mem::size_of::<Node>(), 20);
        assert_eq!(std::mem::align_of::<Node>(), 4);
        assert_eq!(std::mem::size_of::<Token>(), 12);
        assert_eq!(std::mem::size_of::<deep_abi::StateId>(), 2);
        assert_eq!(std::mem::size_of::<deep_abi::NodeId>(), 4);
    }
}
