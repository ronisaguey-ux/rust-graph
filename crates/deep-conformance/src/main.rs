//! `deep-conformance`: the corpus measurement binary. The commands live in
//! `crates/deep-conformance/src/lib.rs`; this file only turns a code into an
//! exit status.

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(u8::try_from(deep_conformance::cli()).unwrap_or(deep_conformance::EXIT_REFUSED as u8))
}
