//! `rust-graph-deep-bench`: the benchmark harness behind the deep parser's 2x
//! gate. See `crates/deep-bench/README.md`.
//!
//! The harness itself is `crates/deep-bench/src/lib.rs`, included here by path
//! rather than linked as a dependency of the `rust-graph` package: a dependency
//! line, even an unused one, changes rust-graph's artifact hash, and the default
//! build has to stay byte for byte today's binary. This binary is also why the
//! harness lives in its own crate: `cargo test` here runs its tests, and
//! `cargo test -p deep-bench` runs the same ones against the library.

#[path = "lib.rs"]
mod harness;

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(u8::try_from(harness::cli()).unwrap_or(2))
}
