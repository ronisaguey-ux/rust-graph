//! The parser runtime, re-exported from `crates/deep-rt`.
//!
//! The grammars used to compile the root package's `src/deep` by path, which
//! made a second copy of the runtime in every build. `deep-rt` is now the one
//! runtime — one lexer, one pair pass, one tree, one budget — so this module is
//! the shim the grammar modules and their tests name, and nothing is compiled
//! twice.

pub use deep_rt::{budget, lexer, pairs, parser, syntax, tree};
pub use deep_rt::{Budget, ParseError, Workspace};
