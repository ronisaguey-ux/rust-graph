//! The bracket pass: one scan, one `Vec`, every mate or `NONE`.

use deep_lex::{tokenize, Brackets, Language, NONE};

#[test]
fn nesting_pairs_symmetrically() {
    let src = b"{[()]}";
    let b = Brackets::scan(src);
    assert_eq!(b.delimiters(), 6);
    assert_eq!(b.pair().len(), 12, "two slots per delimiter, nothing else");
    let offsets: Vec<u32> = (0..b.delimiters()).map(|i| b.offset(i).unwrap()).collect();
    assert_eq!(offsets, vec![0, 1, 2, 3, 4, 5]);
    // (0,5) (1,4) (2,3)
    assert_eq!(b.mate(0), Some(5));
    assert_eq!(b.mate(1), Some(4));
    assert_eq!(b.mate(2), Some(3));
    for i in 0..b.delimiters() {
        assert_eq!(b.mate(b.mate(i).unwrap()), Some(i), "symmetric at {i}");
    }
}

#[test]
fn mismatch_marks_only_the_offender() {
    // f(a, [b)]);
    let src = b"f(a, [b)]);\n";
    let b = Brackets::scan(src);
    assert_eq!(b.delimiters(), 5);
    let offsets: Vec<u32> = (0..b.delimiters()).map(|i| b.offset(i).unwrap()).collect();
    assert_eq!(offsets, vec![1, 5, 7, 8, 9]);
    assert_eq!(b.mate_offset(0), Some(9), "the outer ( pairs with ) at 9");
    assert_eq!(b.mate_offset(1), Some(8), "the [ pairs with ] at 8");
    assert!(
        b.is_unmatched(2),
        "the ) at 7 does not close a [ — it is the mismatch"
    );
    assert_eq!(b.mate_offset(3), Some(5));
    assert_eq!(b.mate_offset(4), Some(1));
}

#[test]
fn unmatched_openers_and_closers_are_none() {
    let b = Brackets::scan(b"(((");
    assert_eq!(b.delimiters(), 3);
    for i in 0..3 {
        assert!(b.is_unmatched(i));
    }
    let b = Brackets::scan(b")]}");
    assert_eq!(b.delimiters(), 3);
    for i in 0..3 {
        assert!(b.is_unmatched(i));
    }
}

#[test]
fn jump_goes_from_offset_to_offset() {
    let src = b"outer(inner[x])";
    let b = Brackets::scan(src);
    let open = src.iter().position(|&c| c == b'(').unwrap() as u32;
    let close = src.iter().rposition(|&c| c == b')').unwrap() as u32;
    assert_eq!(b.jump(open), Some(close));
    assert_eq!(b.jump(close), Some(open));
    let bracket = src.iter().position(|&c| c == b'[').unwrap() as u32;
    let rbracket = src.iter().position(|&c| c == b']').unwrap() as u32;
    assert_eq!(b.jump(bracket), Some(rbracket));
    // A byte that is not a delimiter has no jump.
    assert_eq!(b.jump(0), None);
}

#[test]
fn deep_lexer_exposes_the_precomputed_pairs() {
    let src = std::fs::read(format!(
        "{}/tests/fixtures/mismatch.js",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let lexer = deep_lex::DeepLexer::new(&src, Language::JavaScript);
    let b = lexer.brackets();
    assert_eq!(b.delimiters(), 5);
    assert_eq!(b.pair().len(), 10);
    assert_eq!(b.jump(1), Some(9));
    // NONE is the sentinel the raw slot holds for an unmatched delimiter.
    assert_eq!(b.pair()[2 * 2 + 1], NONE);
}

#[test]
fn linear_scan_on_a_large_input() {
    // 20k openers then 20k closers: a stack-based pass is the only thing that
    // can pair these, and it must do it without a second allocation.
    let mut src = vec![b'('; 20_000];
    src.extend(std::iter::repeat(b')').take(20_000));
    let b = Brackets::scan(&src);
    assert_eq!(b.delimiters(), 40_000);
    assert_eq!(b.pair().len(), 80_000);
    assert_eq!(b.mate(0), Some(39_999));
    assert_eq!(b.mate(19_999), Some(20_000));
    assert_eq!(b.jump(0), Some(39_999));
    assert_eq!(b.jump(19_999), Some(20_000));
    // Every delimiter in this input is paired.
    for i in 0..b.delimiters() {
        assert!(!b.is_unmatched(i));
    }
}

#[test]
fn tokens_and_brackets_agree_on_the_fixture() {
    let src = std::fs::read(format!(
        "{}/tests/fixtures/nested.py",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let toks = tokenize(&src, Language::Python);
    let lexer = deep_lex::DeepLexer::new(&src, Language::Python);
    let b = lexer.brackets();
    // Every delimiter token must have a bracket entry at the same offset.
    let mut seen = 0;
    for tok in &toks {
        if tok.start == tok.end || tok.start as usize >= src.len() {
            continue; // zero-width layout tokens at line ends / EOF
        }
        let c = src[tok.start as usize];
        if matches!(c, b'(' | b')' | b'[' | b']' | b'{' | b'}') {
            let ord = b
                .ordinal_at(tok.start)
                .unwrap_or_else(|| panic!("no bracket at {}", tok.start));
            assert_eq!(b.offset(ord), Some(tok.start));
            seen += 1;
        }
    }
    assert_eq!(seen, b.delimiters());
}
