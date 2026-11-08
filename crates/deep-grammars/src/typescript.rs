//! TypeScript and TSX.
//!
//! TypeScript is a superset of JavaScript, so `javascript.rs` holds one
//! `grammar!` definition, named `TYPESCRIPT`, and the three languages differ
//! only in the lexer:
//!
//! - [`TYPESCRIPT`]: `<` is always an operator, so `<T>x` is a type assertion
//!   and `Map<K, V>` a type argument list.
//! - [`JAVASCRIPT`] and [`TSX`]: `<` where an operand starts opens a JSX
//!   element, which the lexer scans as one token with its braces balanced.
//!
//! This module is the TypeScript face of that shared grammar; it adds no rules
//! of its own. Splitting the definition in two would duplicate 550 rules to
//! change one token's meaning, which is a lexer question, not a grammar one.

pub use crate::javascript::{lex_jsx, lex_typescript, JAVASCRIPT, TSX, TYPESCRIPT};
