mod common;

use common::*;

#[test]
fn the_forty_inputs_lex_identically() {
    let corpus = read_corpus();
    let golden = read_golden();
    assert_eq!(corpus.len(), 40, "four languages, ten inputs each");
    assert_eq!(golden.len(), 40, "one golden section per input");
    for (source, (golden_name, tokens)) in corpus.iter().zip(golden.iter()) {
        assert_eq!(&source.name, golden_name, "corpus and golden stay in step");
        let profile = profile_for(&source.name);
        let diffs = compare(&source.bytes, tokens.clone(), profile);
        assert!(
            diffs.is_empty(),
            "{} ({} bytes):\n  {}",
            source.name,
            source.bytes.len(),
            diffs.join("\n  ")
        );
    }
    for language in ["python", "json", "javascript", "typescript"] {
        assert_eq!(
            corpus.iter().filter(|s| s.name.starts_with(language) || s.name.starts_with(&format!("{language}/"))).count(),
            10,
            "{language} contributes ten inputs"
        );
    }
}

#[test]
fn every_kind_has_a_symbol_and_a_name() {
    // The identity symbol map must cover the whole enum: a kind added past the
    // end of the table silently became symbol 0 (ERROR) once already.
    use deep_lex::{SymbolMap, ALL_KINDS};
    let map = SymbolMap::identity();
    for &kind in ALL_KINDS {
        assert_eq!(map.get(kind), kind as u16, "{kind:?} lost its symbol");
        assert_eq!(Kind::from_symbol(kind as u16), Some(kind));
    }
    assert_eq!(ALL_KINDS.len(), Kind::COUNT);
}

#[test]
fn the_known_differences_are_real_and_bounded() {
    // Prove the two documented categories are what the fixtures actually
    // exercise, so a silent change in either shows up here.
    let corpus = read_corpus();
    let golden = read_golden();
    let section = |needle: &str| {
        let at = corpus.iter().position(|s| s.name == needle).expect("fixture");
        (&corpus[at].bytes, golden[at].1.clone())
    };
    let (src, golden) = section("javascript/09_operators.js");
    assert!(
        golden.iter().any(|t| t.name == ">joined"),
        "the fixture must exercise the joined-closer difference"
    );
    let rejoined = rejoin_joined(golden.iter().filter(|t| t.class != "trivia").cloned().collect());
    let deep = deep_tokens(src, &deep_lex::JAVASCRIPT);
    let old_shift = rejoined.iter().find(|t| t.name == ">joined").unwrap();
    assert!(
        deep.iter().any(|(kind, start, end)| *kind == Kind::GtGt
            && *start == old_shift.start
            && *end == old_shift.end),
        "one GtGt replaces the run of per-byte >joined tokens"
    );

    let (bad_src, bad) = section("javascript/10_malformed.js");
    let bad_token = bad.iter().find(|t| t.name == "bad_literal").unwrap();
    let deep_bad = deep_tokens(bad_src, &deep_lex::JAVASCRIPT);
    assert!(
        deep_bad.iter().any(|(kind, start, end)| *kind == Kind::String
            && *start == bad_token.start
            && *end == bad_token.end),
        "the bad literal keeps its span, under Kind::String"
    );
}
