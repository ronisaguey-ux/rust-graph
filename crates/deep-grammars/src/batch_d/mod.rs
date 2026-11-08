//! The long tail: structural and legacy languages of the deep scan.
//!
//! One module per language, each with a `grammar!`, a hand-written step lexer
//! and fixtures under `fixtures/longtail/<language>/`. The module comments name
//! what each grammar models and what it cannot.

pub mod lex;

pub mod json;
pub mod shell;
pub mod solidity;
pub mod sql;
pub mod tcl;
pub mod awk;
pub mod clojure;
pub mod css;
pub mod elixir;
pub mod erlang;
pub mod html;
pub mod julia;
pub mod lua;
pub mod perl;
pub mod r;
pub mod yaml;
pub mod zig;
