//! The four profiles: one lexer, four languages, all differences in data.

use deep_lex::{
    profiles, DeepLexer, Kind, Language, Lexer, PairJump, SymbolMap, JAVASCRIPT, JSON, PYTHON,
    TYPESCRIPT,
};

fn triples(src: &[u8], profile: &'static deep_lex::Profile) -> Vec<(Kind, u32, u32)> {
    let mut lexer = DeepLexer::for_profile(src, profile);
    let mut out = Vec::new();
    while let Some(token) = lexer.next_token(0) {
        out.push((Kind::from_symbol(token.symbol).unwrap(), token.start, token.end));
    }
    out
}

#[test]
fn each_profile_lexes_its_own_language() {
    // The same call shape, four different rule sets. Layout only in Python.
    assert_eq!(
        triples(b"if x:\n    y = 1\n", &PYTHON),
        vec![
            (Kind::Ident, 0, 2),
            (Kind::Ident, 3, 4),
            (Kind::Colon, 4, 5),
            (Kind::Newline, 5, 5),
            (Kind::Indent, 10, 10),
            (Kind::Ident, 10, 11),
            (Kind::Eq, 12, 13),
            (Kind::Number, 14, 15),
            (Kind::Newline, 15, 15),
            (Kind::Dedent, 16, 16),
        ]
    );
    // JSON: signed number is one token, keywords stay identifiers for the
    // grammar to classify, and there is no layout.
    assert_eq!(
        triples(b"{\"a\": -12, \"b\": [true, false, null]}", &JSON),
        vec![
            (Kind::LBrace, 0, 1),
            (Kind::String, 1, 4),
            (Kind::Colon, 4, 5),
            (Kind::Number, 6, 9),
            (Kind::Comma, 9, 10),
            (Kind::String, 11, 14),
            (Kind::Colon, 14, 15),
            (Kind::LBracket, 16, 17),
            (Kind::Ident, 17, 21),
            (Kind::Comma, 21, 22),
            (Kind::Ident, 23, 28),
            (Kind::Comma, 28, 29),
            (Kind::Ident, 30, 34),
            (Kind::RBracket, 34, 35),
            (Kind::RBrace, 35, 36),
        ]
    );
    // JavaScript and TypeScript lex the same bytes identically: the profiles
    // differ only in name and the modes a parser may request.
    let js = triples(b"const r = /a+/g; x = a / b;", &JAVASCRIPT);
    let ts = triples(b"const r = /a+/g; x = a / b;", &TYPESCRIPT);
    assert_eq!(js, ts);
    assert_eq!(js[3], (Kind::Regex, 10, 15), "after `=` the `/` opens a regex");
    assert_eq!(js[8], (Kind::Slash, 23, 24), "after an identifier it divides");
}

#[test]
fn a_state_selects_regex_for_javascript_and_typescript() {
    let src = b"x = /a+/g;";
    for profile in [&JAVASCRIPT, &TYPESCRIPT] {
        // `x` and `=` are default-mode tokens; the token *after* `=` is read in
        // the operand-expected state, which is where `/` starts a regex.
        let mut lexer = DeepLexer::for_profile(src, profile);
        assert_eq!(lexer.next_token(0).unwrap().start, 0);
        assert_eq!(lexer.next_token(0).unwrap().start, 2);
        let regex = lexer.next_token(profiles::OPERAND_EXPECTED).unwrap();
        assert_eq!(
            Kind::from_symbol(regex.symbol),
            Some(Kind::Regex),
            "{} state 1 is a regex context",
            profile.name
        );
        assert_eq!((regex.start, regex.end), (4, 9));
        assert_eq!(lexer.mode(), "regex");
        // Back to default: the hint frame is popped.
        let semi = lexer.next_token(0).unwrap();
        assert_eq!((semi.start, semi.end), (9, 10));
        assert_eq!(lexer.mode(), "default");
    }
    // A parser that requests regex at the wrong byte still gets ordinary
    // tokens, not a swallowed rest of line.
    let mut lexer = DeepLexer::for_profile(b"a = b + c", &JAVASCRIPT);
    let mut out = Vec::new();
    while let Some(token) = lexer.next_token(profiles::OPERAND_EXPECTED) {
        out.push((token.start, token.end));
        if out.len() > 8 {
            break;
        }
    }
    assert_eq!(out, vec![(0, 1), (2, 3), (4, 5), (6, 7), (8, 9)]);
}

#[test]
fn the_python_profile_requests_a_raw_string_by_state() {
    let mut lexer = DeepLexer::for_profile(b"R\"a\\b\"", &PYTHON);
    let token = lexer.next_token(profiles::RAW_STRING_EXPECTED).unwrap();
    assert_eq!(Kind::from_symbol(token.symbol), Some(Kind::RawString));
    assert_eq!(lexer.mode(), "raw-string");
    let _ = lexer.next_token(profiles::DEFAULT);
    assert_eq!(lexer.mode(), "default", "the hint frame pops at state 0");
}

#[test]
fn a_json_table_says_no_mode_is_ever_needed() {
    for state in 0..3u16 {
        assert_eq!(JSON.mode_for(state), Some(deep_lex::Mode::Default));
    }
    assert_eq!(PYTHON.mode_for(profiles::RAW_STRING_EXPECTED), Some(deep_lex::Mode::RawString));
    assert_eq!(JAVASCRIPT.mode_for(profiles::OPERAND_EXPECTED), Some(deep_lex::Mode::Regex));
    // Past the table: the lexer keeps its current mode.
    assert_eq!(JSON.mode_for(9), None);
    assert_eq!(profiles::ALL.len(), 4);
}

#[test]
fn profiles_publish_pair_rules_in_the_grammars_numbering() {
    let map = SymbolMap::identity();
    let (pairs, n) = PYTHON.symbol_pairs(&map);
    assert_eq!(n, 3);
    assert_eq!(pairs[..n], [(Kind::LParen as u16, Kind::RParen as u16),
                           (Kind::LBracket as u16, Kind::RBracket as u16),
                           (Kind::LBrace as u16, Kind::RBrace as u16)]);
    let (json_pairs, json_n) = JSON.symbol_pairs(&map);
    assert_eq!(json_n, 2, "JSON has no parentheses");
    assert_eq!(json_pairs[0], (Kind::LBrace as u16, Kind::RBrace as u16));

    // A grammar with its own numbering gets its own symbols out.
    let mut custom = SymbolMap::identity();
    custom.set(Kind::LBrace, 300);
    custom.set(Kind::RBrace, 301);
    let (pairs, _) = JSON.symbol_pairs(&custom);
    assert_eq!(pairs[0], (300, 301));
}

#[test]
fn json_rejects_single_quotes_and_bare_words_as_the_old_lexer_did() {
    let toks = triples(b"'x' y", &JSON);
    assert_eq!(toks[0], (Kind::Error, 0, 1), "`'` is not a JSON delimiter");
    assert_eq!(toks[1], (Kind::Ident, 1, 2));
    assert_eq!(toks[2], (Kind::Error, 2, 3));
    assert_eq!(toks[3], (Kind::Ident, 4, 5));
    // Malformed JSON never panics and every byte is covered in order.
    let src = b"{\"unterminated: [1, 2,, }";
    let mut end = 0;
    for (_, start, stop) in triples(src, &JSON) {
        assert!(start >= end);
        end = stop;
    }
}

#[test]
fn every_language_is_reachable_through_the_profile_array() {
    for profile in profiles::ALL {
        let src: &[u8] = b"a(b)[c]{d}";
        let mut lexer = DeepLexer::for_profile(src, profile);
        let mut jumped = 0;
        while let Some(token) = lexer.next_token(0) {
            if token.start == 1 {
                let jump = lexer.jump(token.start).expect("( has a mate");
                assert_eq!(jump, 3);
                assert_eq!(lexer.pair().len(), 6 * 2, "three pairs per language");
                jumped += 1;
            }
        }
        assert_eq!(jumped, 1, "{} found its group", profile.name);
    }
}

#[test]
fn a_pair_jump_skips_a_failed_group_in_one_step() {
    let src = b"f(a, [b, c], d)";
    let lexer = DeepLexer::new(src, Language::JavaScript);
    let jump = PairJump::new(lexer.pair());
    assert_eq!(jump.jump(1), Some(14));
    assert_eq!(jump.skip_to_closer(src, 1), Some(15));
    assert_eq!(jump.group_end(src, 1), Some(14));
    assert_eq!(jump.skip_to_closer(src, 5), Some(11), "the inner group");
    // Not an opener: no cascade.
    assert_eq!(jump.skip_to_closer(src, 2), None);
    assert_eq!(jump.jump(2), None);
}

#[test]
fn a_deep_nest_still_jumps_alike() {
    // 10k nested groups: only a pair table can find the outer mate, and the
    // binary search must land on it and not on a neighbour.
    let mut src = vec![b'('; 10_000];
    src.extend(std::iter::repeat(b')').take(10_000));
    let lexer = DeepLexer::new(&src, Language::JavaScript);
    let jump = PairJump::new(lexer.pair());
    assert_eq!(jump.jump(0), Some(19_999));
    assert_eq!(jump.jump(9_999), Some(10_000));
    assert_eq!(jump.skip_to_closer(&src, 0), Some(20_000));
}
