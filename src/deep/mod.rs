//! The deep scan: a real parser behind the `parser` feature. The default build
//! never compiles this module.
//!
//! ```text
//! &[u8] -> lexer -> delimiter pass -> grammar on the parser runtime -> flat tree
//!          tokens    pair[]             events, budgets                 16 B nodes
//! ```
//! Slice 1 is the runtime and JSON written by hand on it. Slice 2 is `grammar!`
//! and JSON through it (`json_grammar`), proved event for event against the hand.
//! Parts of the runtime API (precede, checkpoints) are first used by the grammars
//! of later slices and by the tests here.
#![allow(dead_code)]

// The C, C++ and Objective-C grammars, included by path for the same reason as
// Python and JavaScript: `deep-grammars` compiles the one copy, and a second
// copy here would be a fork. Their `batch_c` helpers are reached through
// `super`, so the files keep working in either crate.
#[path = "../../crates/deep-grammars/src/batch_c/mod.rs"]
pub mod batch_c;
pub mod budget;
pub mod go;
pub mod java;
// The one JavaScript grammar: the same file `deep-grammars` compiles against
// its runtime shim, so the two paths can never drift apart again. Its own
// tests are in `javascript_tests`, beside this include.
#[path = "../../crates/deep-grammars/src/javascript.rs"]
pub mod javascript;
#[cfg(test)]
pub mod javascript_tests;
pub mod json;
pub mod json_grammar;
pub mod lexer;
pub mod pairs;
pub mod parser;
// The one Python grammar: the same file `deep-grammars` compiles against
// its runtime shim, so the two paths can never drift apart again.
#[path = "../../crates/deep-grammars/src/python.rs"]
pub mod python;
pub mod rust;
pub mod syntax;
pub mod tree;

#[cfg(test)]
mod tests;

pub use budget::Budget;
pub use parser::Workspace;

/// Bumped whenever parser output changes. Part of the `facts.pack` key from
/// Slice 4, so a new parser never reads facts cached by an old one.
pub const PARSER_VERSION: u32 = 1;

/// The language for a path, by extension: JSON, Python, Rust, JavaScript,
/// TypeScript, TSX, Go and Java so far.
pub fn language_for(path: &str) -> Option<&'static syntax::Language> {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase())?;
    match extension.as_str() {
        "json" => Some(&json::JSON),
        "py" | "pyi" => Some(&python::PYTHON),
        "rs" => Some(&rust::RUST),
        "js" | "mjs" | "cjs" | "jsx" => Some(&javascript::JAVASCRIPT),
        "ts" | "mts" | "cts" => Some(&javascript::TYPESCRIPT),
        "tsx" => Some(&javascript::TSX),
        "go" => Some(&go::GO),
        "java" => Some(&java::JAVA),
        _ => None,
    }
}
