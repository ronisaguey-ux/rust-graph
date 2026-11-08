//! The long tail: structural and legacy languages of the deep scan.
//!
//! One module per language, each with a `grammar!`, a hand-written step lexer
//! and fixtures under `fixtures/longtail/<language>/`. The module comments name
//! what each grammar models and what it cannot.

pub mod lex;

pub mod json;
pub mod shell;
pub mod sql;
pub mod css;
pub mod html;
pub mod lua;
pub mod r;
pub mod yaml;
