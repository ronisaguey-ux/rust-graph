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
#[path = "../../crates/deep-grammars/src/batch_a/mod.rs"]
pub mod batch_a;
#[path = "../../crates/deep-grammars/src/batch_b/mod.rs"]
pub mod batch_b;
#[path = "../../crates/deep-grammars/src/batch_c/mod.rs"]
pub mod batch_c;
// The scripting and table grammars: every language the lexer knows and this
// package has a grammar for takes the CST path. Written once, in
// `deep-grammars`, and compiled here by path like the C family.
#[path = "../../crates/deep-grammars/src/batch_d/mod.rs"]
pub mod batch_d;
#[path = "../../crates/deep-grammars/src/batch_e/mod.rs"]
pub mod batch_e;
#[path = "../../crates/deep-grammars/src/batch_f/mod.rs"]
pub mod batch_f;
// The runtime lives once, in `crates/deep-rt`: one lexer, one pair pass, one
// tree, one budget, one node-kind table. These names stay because every grammar
// and every test in this package writes `crate::deep::…`.
pub use deep_rt::{budget, lexer, pairs, parser, syntax, tree};

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
// The one Python grammar: the same file `deep-grammars` compiles against
// its runtime shim, so the two paths can never drift apart again.
#[path = "../../crates/deep-grammars/src/python.rs"]
pub mod python;
pub mod rust;

#[cfg(test)]
mod tests;

pub use budget::Budget;
pub use parser::Workspace;

/// Bumped whenever parser output changes. Part of the `facts.pack` key from
/// Slice 4, so a new parser never reads facts cached by an old one.
pub const PARSER_VERSION: u32 = 1;

/// The language for a path, by extension: JSON, Python, Rust, JavaScript,
/// TypeScript, TSX, Go, Java, C, C++, C#, Kotlin, Scala and Swift.
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
        "c" | "h" => Some(&batch_c::c::C),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => Some(&batch_c::cpp::CPP),
        "cs" => Some(&batch_a::csharp::CSHARP),
        "kt" | "kts" => Some(&batch_a::kotlin::KOTLIN),
        "scala" | "sc" => Some(&batch_b::scala::SCALA),
        "swift" => Some(&batch_b::swift::SWIFT),
        "m" => Some(&batch_c::objc::OBJC),
        // The grammars that were written but never connected: each of these is
        // a real reader now, and its corpus is measurable.
        "php" | "phtml" => Some(&batch_b::php::PHP),
        "rb" | "rake" | "gemspec" => Some(&batch_b::ruby::RUBY),
        "dart" => Some(&batch_f::dart::DART),
        "ex" | "exs" => Some(&batch_d::elixir::ELIXIR),
        "hs" | "lhs" => Some(&batch_e::haskell::HASKELL),
        "lua" => Some(&batch_d::lua::LUA),
        "r" | "rmd" => Some(&batch_d::r::R),
        "sol" => Some(&batch_d::solidity::SOLIDITY),
        "sql" | "ddl" => Some(&batch_d::sql::SQL),
        "sh" | "bash" | "zsh" | "ksh" => Some(&batch_d::shell::SHELL),
        "groovy" | "gradle" => Some(&batch_f::groovy::GROOVY),
        "jl" => Some(&batch_d::julia::JULIA),
        "vb" | "vbs" | "bas" | "cls" => Some(&batch_f::visual_basic::VISUAL_BASIC),
        "pas" | "dpr" | "pp" => Some(&batch_f::pascal::PASCAL),
        "cob" | "cbl" | "cpy" => Some(&batch_f::cobol::COBOL),
        "nim" => Some(&batch_e::nim::NIM),
        "cr" => Some(&batch_e::crystal::CRYSTAL),
        "zig" => Some(&batch_d::zig::ZIG),
        "d" => Some(&batch_e::d::D),
        "fs" | "fsx" => Some(&batch_f::fsharp::FSHARP),
        "ml" | "mli" => Some(&batch_e::ocaml::OCAML),
        "erl" | "hrl" => Some(&batch_d::erlang::ERLANG),
        "clj" | "cljs" | "cljc" => Some(&batch_d::clojure::CLOJURE),
        "tcl" => Some(&batch_d::tcl::TCL),
        "awk" => Some(&batch_d::awk::AWK),
        _ => None,
    }
}
