//! The separate deep-scan binary, built only with `--features parser`.

#[path = "../deep/mod.rs"]
mod deep;

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version") => {
            println!("rust-graph-deep-scan {} (parser {})", env!("CARGO_PKG_VERSION"), deep::PARSER_VERSION);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("rust-graph-deep-scan: no language grammars are built yet; only --version works");
            ExitCode::from(2)
        }
    }
}
