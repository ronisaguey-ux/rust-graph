//! The parser runtime, compiled from `rust-graph`'s `src/deep` by path.
//!
//! The parallel build gives the runtime its own crate (`crates/deep-runtime`,
//! owned by another field). Until that crate exists in this worktree the
//! grammars need *a* runtime to run on and to generate against, and the one
//! already proved against JSON, Rust, Go, Java and these three languages is
//! `src/deep`. Copying it here would fork it, so it is included by path and the
//! grammar modules keep the default `runtime = crate::deep`, which is exactly
//! what they would use after the split.
//!
//! Only the six modules a grammar needs are included; the root package's
//! `mod.rs` also declares the languages and their tests, which belong to the
//! packages that own them.

#[path = "../../../src/deep/budget.rs"]
pub mod budget;
#[path = "../../../src/deep/lexer.rs"]
pub mod lexer;
#[path = "../../../src/deep/pairs.rs"]
pub mod pairs;
#[path = "../../../src/deep/parser.rs"]
pub mod parser;
#[path = "../../../src/deep/syntax.rs"]
pub mod syntax;
#[path = "../../../src/deep/tree.rs"]
pub mod tree;

pub use budget::Budget;
pub use parser::Workspace;
