//! One place per reader, and one test that both tables stay complete.
//!
//! A language is added to the one-pass lexer by one row in `LANGUAGES` (plus its
//! token rules) and to the CST reader by one static and one arm of
//! `deep::language_for`. Anything else — a second list of suffixes, a second
//! dispatch table — means a language can exist in one reader and be invisible to
//! the other, and the comparison then scores the missing side as a bug.
//!
//! The tables are checked against each other rather than against a copy: every
//! suffix the lexer advertises must resolve through `by_content`, and every
//! grammar the CST ships must answer `language_for` on the extensions it claims.

#[path = "../src/langs.rs"]
mod langs;

use std::collections::BTreeSet;

#[test]
fn the_suffix_table_is_one_sorted_list_without_duplicates() {
    let rows = langs::all_suffixes();
    assert!(rows.len() > 40, "the table is the 45-language one: {} rows", rows.len());

    let mut seen = BTreeSet::new();
    for (suffix, tag, name) in &rows {
        assert!(!suffix.is_empty() && !tag.is_empty() && !name.is_empty(), "an empty field in {suffix:?}");
        assert!(seen.insert(*suffix), "{suffix} is registered twice");
        assert_eq!(*suffix, suffix.to_ascii_lowercase(), "{suffix} is not lowercase");
    }
    let sorted: Vec<&str> = rows.iter().map(|row| row.0).collect();
    let mut expect = sorted.clone();
    expect.sort_unstable();
    assert_eq!(sorted, expect, "all_suffixes promises sorted order");
}

#[test]
fn every_advertised_suffix_resolves_to_a_language() {
    for (suffix, tag, _) in langs::all_suffixes() {
        let language = langs::by_content(&suffix, b"")
            .unwrap_or_else(|| panic!(".{suffix} is advertised and does not resolve"));
        assert_eq!(language.tag, tag, ".{suffix} resolves to {} not {tag}", language.tag);
    }
}

#[cfg(all(feature = "deep", feature = "parser"))]
#[test]
fn every_cst_grammar_answers_on_the_extensions_it_ships() {
    let claims = [
        ("a.json", "json"),
        ("a.py", "python"),
        ("a.pyi", "python"),
        ("a.rs", "rust"),
        ("a.js", "javascript"),
        ("a.mjs", "javascript"),
        ("a.cjs", "javascript"),
        ("a.jsx", "javascript"),
        ("a.ts", "typescript"),
        ("a.mts", "typescript"),
        ("a.cts", "typescript"),
        ("a.tsx", "tsx"),
        ("a.go", "go"),
        ("a.java", "java"),
    ];
    for (path, name) in claims {
        let language = deep::language_for(path).unwrap_or_else(|| panic!("{path} has no CST grammar"));
        assert_eq!(language.name, name, "{path}");
    }
    assert!(deep::language_for("a.rb").is_none(), "an unregistered extension must not answer");
}

#[cfg(all(feature = "deep", feature = "parser"))]
#[path = "../src/deep/mod.rs"]
mod deep;
