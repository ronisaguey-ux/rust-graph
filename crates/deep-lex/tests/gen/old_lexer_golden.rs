//! Regenerates `tests/fixtures/differential/golden.tsv` from the old lexer.
//!
//! This file is **not** compiled by `cargo test`: it lives in a subdirectory so
//! cargo ignores it. It cannot be a test target of `deep-lex` because the old
//! lexer is part of the `rust-graph` binary crate (and `deep-grammars` includes
//! it by path), which `deep-lex` must not depend on — that would put the
//! replacement and the thing it replaces in one dependency graph.
//!
//! # Regenerating
//!
//! ```text
//! mkdir -p /tmp/oldlex/src /tmp/oldlex/.cargo
//! cp tests/gen/old_lexer_golden.rs /tmp/oldlex/src/main.rs
//! cat > /tmp/oldlex/Cargo.toml <<'TOML'
//! [package]
//! name = "oldlex"
//! version = "0.1.0"
//! edition = "2021"
//!
//! [dependencies]
//! deep-grammars = { path = "<worktree>/crates/deep-grammars" }
//!
//! [workspace]
//! TOML
//! # This host has no C toolchain: build static musl, and give the host-side
//! # proc-macro build the lld wrapper.
//! printf '[build]\ntarget = "x86_64-unknown-linux-musl"\n\n\
//! [target.x86_64-unknown-linux-musl]\nlinker = "rust-lld"\n' \
//!   > /tmp/oldlex/.cargo/config.toml
//! CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$HOME/.local/bin/rust-lld-cc \
//!   cargo run --manifest-path /tmp/oldlex/Cargo.toml
//! ```
//!
//! The `#[path]` below must point at this worktree's `src/deep/json.rs`; the
//! JSON lexer is only in the root crate, and `deep-grammars` re-exports the
//! runtime modules it needs, so the include borrows those instead of copying
//! `src/deep` a third time.

mod oldjson {
    pub use deep_grammars::deep::{budget, lexer, pairs, parser, syntax, tree};
    #[path = "/home/roni/Roni_workspace/rg-f3/src/deep/json.rs"]
    pub mod json;
}

use std::fs;
use std::path::Path;

use deep_grammars::deep::lexer::{lex, Tokens};
use deep_grammars::deep::syntax::{Class, Language};

fn class_name(class: Class) -> &'static str {
    match class {
        Class::Node => "node",
        Class::Token => "token",
        Class::Trivia => "trivia",
        Class::BadToken => "bad",
        Class::Open(_) => "open",
        Class::Close(_) => "close",
        // The old lexer reports branch lines as trivia to the parser.
        Class::BranchIf | Class::BranchElse | Class::BranchEnd => "trivia",
    }
}

struct Source {
    name: String,
    bytes: Vec<u8>,
}

/// Parse `corpus.txt`: `#name <path> <byte length>` then exactly that many bytes.
fn corpus(path: &Path) -> Vec<Source> {
    let bytes = fs::read(path).unwrap();
    let mut rest: &[u8] = &bytes;
    let mut out = Vec::new();
    while let Some(at) = rest
        .windows(6)
        .position(|window| window == b"#name ")
    {
        rest = &rest[at + 6..];
        let line_end = rest.iter().position(|&b| b == b'\n').expect("header newline");
        let header = std::str::from_utf8(&rest[..line_end]).unwrap();
        let (name, length) = header.rsplit_once(' ').unwrap();
        let length: usize = length.parse().unwrap();
        let body = &rest[line_end + 1..];
        out.push(Source {
            name: name.to_string(),
            bytes: body[..length].to_vec(),
        });
        rest = &body[length..];
    }
    out
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/differential");
    let sources = corpus(&root.join("corpus.txt"));
    let mut out = String::new();
    out.push_str("# deep-lex differential golden: the old lexer's tokens\n");
    out.push_str("# One section per corpus input, in the same order.\n");
    out.push_str("# A section is '#name <path> <token count>', then kind\\tclass\\tstart\\tend.\n");
    for source in &sources {
        let lang: &'static Language = match source.name.split('/').next().unwrap() {
            "python" => &deep_grammars::python::PYTHON,
            "json" => &oldjson::json::JSON,
            "javascript" => &deep_grammars::javascript::JAVASCRIPT,
            "typescript" => &deep_grammars::typescript::TYPESCRIPT,
            other => panic!("no old language for {other}"),
        };
        let mut tokens = Tokens::default();
        lex(&source.bytes, lang, &mut tokens);
        out.push_str(&format!("#name {} {}\n", source.name, tokens.len()));
        for i in 0..tokens.len() {
            let kind = tokens.kind[i];
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                lang.name(kind),
                class_name(lang.class(kind)),
                tokens.start(i),
                tokens.end[i]
            ));
        }
    }
    fs::write(root.join("golden.tsv"), out).unwrap();
    println!("wrote {} sections", sources.len());
}
