//! The default build is the lexer-only build, and it must stay that way.
//!
//! `deep` and `parser` bring the CST path in. A build without them is a
//! supported product — the one a user gets from `cargo build` with no flags —
//! and the failure this guards is quiet: a `#[cfg]` that stops compiling the
//! lexer path out, or a feature that becomes mandatory by accident. The check
//! is behavioural, not textual: self-report through the binary's own usage text,
//! because that is the interface a user actually sees.

use std::process::Command;

fn usage() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-graph"))
        .arg("--help")
        .output()
        .expect("the binary runs");
    // `--help` is not a flag this tool knows; it prints usage to stderr and
    // exits non-zero, which is the contract the real user sees too.
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn the_cst_path_is_compiled_out_of_the_default_build() {
    let text = usage();
    #[cfg(not(feature = "deep"))]
    {
        assert!(
            !text.contains("--deep"),
            "the default build advertises --deep, so the CST path is compiled in: {text}"
        );
        assert!(
            !text.contains("--compare"),
            "the default build advertises --compare: {text}"
        );
    }
    #[cfg(feature = "deep")]
    {
        assert!(
            text.contains("--deep"),
            "the deep build does not advertise --deep: {text}"
        );
    }
}

#[test]
fn a_language_list_needs_no_cst() {
    // The lexer path answers this on its own in every build shape; if a feature
    // were ever needed to list languages, the default product would be broken.
    let output = Command::new(env!("CARGO_BIN_EXE_rust-graph"))
        .arg("languages")
        .output()
        .expect("the binary runs");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "languages failed: {}", String::from_utf8_lossy(&output.stderr));
    for language in ["JavaScript", "TypeScript", "Python", "Rust"] {
        assert!(text.contains(language), "{language} missing from languages: {}", text.len());
    }
}
