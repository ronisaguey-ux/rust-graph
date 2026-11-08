//! The five long-tail grammars of field G1: Haskell, Nim, OCaml, D and Crystal.
//!
//! One module per language, each with a `grammar!`, a hand-written step lexer
//! and fixtures under `fixtures/longtail2/<language>/`. The module comments name
//! what each grammar models and what it cannot.
//!
//! This module is not declared in `lib.rs` yet: the owner of that file adds
//! `pub mod batch_e;` with the other fields' lines.

pub mod d;
pub mod ocaml;

#[cfg(test)]
mod tests;
