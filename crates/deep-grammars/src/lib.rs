//! The Python, JavaScript and TypeScript grammars of the deep scan, written in
//! the `grammar!` DSL, with their hand-written step lexers.
//!
//! A grammar is a token list, a lexer and a set of rules; `grammar!` generates
//! the kind table, the `Language` static and one `parse_<rule>` function per
//! rule. Nothing here is a stub: every construct the DSL cannot express is
//! named in `README.md` and in the module comment of the language it affects.
//!
//! ```text
//! &[u8] -> lexer -> delimiter pass -> grammar on the parser runtime -> flat tree
//! ```
//!
//! The runtime itself (`syntax`, `lexer`, `pairs`, `parser`, `tree`, `budget`)
//! is the one in `src/deep` of the `rust-graph` package. It is compiled into
//! this crate by path because the separate `deep-runtime` crate it will become
//! is not in this worktree yet; see `deep.rs` and `README.md`. The grammar
//! definitions below never name a runtime path other than the default
//! `crate::deep`, so moving them onto that crate later is a one-line change.

pub mod batch_a;
pub mod batch_b;
pub mod batch_c;
pub mod batch_d;
pub mod batch_f;
pub mod deep;
pub mod javascript;
pub mod python;
pub mod typescript;
