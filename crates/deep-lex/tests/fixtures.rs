//! Byte-exact fixtures. Every assertion is a `(kind, start, end)` triple, so a
//! change in position fails as loudly as a change in kind.

use deep_lex::{tokenize, Kind, Language};

fn load(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn tokens(src: &[u8], lang: Language) -> Vec<(Kind, u32, u32)> {
    tokenize(src, lang)
        .into_iter()
        .map(|t| {
            let kind = Kind::from_symbol(t.symbol).expect("identity symbol map");
            (kind, t.start, t.end)
        })
        .collect()
}

macro_rules! t {
    ($kind:ident, $start:expr, $end:expr) => {
        (Kind::$kind, $start, $end)
    };
}

#[test]
fn crlf_offsets_are_the_raw_bytes() {
    let src = load("crlf.js");
    assert_eq!(src.len(), 26);
    // The `\r\n` pairs are two bytes each and belong to no token.
    assert_eq!(
        tokens(&src, Language::JavaScript),
        vec![
            t!(Ident, 0, 3),      // let
            t!(Ident, 4, 5),      // a
            t!(Eq, 6, 7),         // =
            t!(String, 8, 11),    // "x"
            t!(Semicolon, 11, 12),
            t!(Ident, 14, 17),    // let, after \r\n at 12..14
            t!(Ident, 18, 19),
            t!(Eq, 20, 21),
            t!(Number, 22, 23),
            t!(Semicolon, 23, 24),
        ]
    );
}

#[test]
fn utf8_identifiers_span_their_multibyte_bytes() {
    let src = load("utf8.js");
    assert_eq!(src.len(), 42);
    assert_eq!(
        tokens(&src, Language::JavaScript),
        vec![
            t!(Ident, 0, 3),    // let
            t!(Ident, 4, 9),    // café: 4 bytes + two-byte é
            t!(Eq, 10, 11),
            t!(String, 12, 20), // "naïve"
            t!(Semicolon, 20, 21),
            t!(Ident, 22, 25),  // let
            t!(Ident, 26, 32),  // 日本: two three-byte scalars
            t!(Eq, 33, 34),
            t!(Ident, 35, 40),  // café
            t!(Semicolon, 40, 41),
        ]
    );
    let toks = tokenize(&src, Language::JavaScript);
    let cafe = &src[toks[1].start as usize..toks[1].end as usize];
    assert_eq!(cafe, "café".as_bytes());
    let jp = &src[toks[6].start as usize..toks[6].end as usize];
    assert_eq!(jp, "日本".as_bytes());
}

#[test]
fn unterminated_string_ends_at_the_line_break() {
    let src = load("unterminated.js");
    assert_eq!(src.len(), 24);
    assert_eq!(
        tokens(&src, Language::JavaScript),
        vec![
            t!(Ident, 0, 3),    // let
            t!(Ident, 4, 5),    // s
            t!(Eq, 6, 7),
            // The quote at 8 opens a literal that never closes; the token stops
            // before the newline at 12 instead of swallowing the rest of file.
            t!(String, 8, 12),
            t!(Ident, 13, 16),  // let
            t!(Ident, 17, 18),  // t
            t!(Eq, 19, 20),
            t!(Number, 21, 22), // 2
            t!(Semicolon, 22, 23),
        ]
    );
    let toks = tokenize(&src, Language::JavaScript);
    assert_eq!(&src[toks[3].start as usize..toks[3].end as usize], b"\"abc");
}

#[test]
fn bracket_mismatch_still_lexes_every_token() {
    let src = load("mismatch.js");
    assert_eq!(
        tokens(&src, Language::JavaScript),
        vec![
            t!(Ident, 0, 1),    // f
            t!(LParen, 1, 2),
            t!(Ident, 2, 3),    // a
            t!(Comma, 3, 4),
            t!(LBracket, 5, 6),
            t!(Ident, 6, 7),    // b
            t!(RParen, 7, 8),   // mismatched: the open bracket is `[`
            t!(RBracket, 8, 9),
            t!(RParen, 9, 10),
            t!(Semicolon, 10, 11),
        ]
    );
}

#[test]
fn python_nested_indentation_layout() {
    let src = load("nested.py");
    assert_eq!(src.len(), 152);
    assert_eq!(
        tokens(&src, Language::Python),
        vec![
            t!(Ident, 0, 3),     // def
            t!(Ident, 4, 5),     // f
            t!(LParen, 5, 6),
            t!(Ident, 6, 7),     // a
            t!(RParen, 7, 8),
            t!(Colon, 8, 9),
            t!(Newline, 9, 9),   // zero width, before the \n at 9
            t!(Indent, 14, 14),  // zero width, at the first code byte
            t!(Ident, 14, 16),   // if
            t!(Ident, 17, 18),   // a
            t!(Colon, 18, 19),
            t!(Newline, 19, 19),
            t!(Indent, 28, 28),
            t!(Ident, 28, 34),   // return
            t!(LBrace, 35, 36),  // opens a group: newlines inside are suppressed
            t!(String, 49, 52),  // "k"
            t!(Colon, 52, 53),
            t!(Number, 54, 55),
            t!(Comma, 55, 56),
            t!(RBrace, 65, 66),  // closes the group
            t!(Newline, 66, 66), // the logical line ends here, not at 36
            t!(Dedent, 86, 86),  // comment and blank lines emitted nothing
            t!(Ident, 86, 92),   // return
            t!(Ident, 93, 94),   // a
            t!(Plus, 105, 106),  // continuation line, no INDENT at 105
            t!(Number, 107, 108),
            t!(Newline, 108, 108),
            t!(Ident, 113, 118), // total
            t!(Eq, 119, 120),
            t!(Ident, 121, 122), // f
            t!(LParen, 122, 123),
            t!(Ident, 132, 133), // a, inside brackets: no layout
            t!(Comma, 133, 134),
            t!(Number, 143, 144), // 2
            t!(Comma, 144, 145),
            t!(RParen, 150, 151),
            t!(Newline, 151, 151),
            t!(Dedent, 152, 152), // file end closes the last suite
        ]
    );
}

#[test]
fn template_literal_nests_modes() {
    let src = load("template.js");
    assert_eq!(src.len(), 27);
    assert_eq!(
        tokens(&src, Language::JavaScript),
        vec![
            t!(Ident, 0, 3),
            t!(Ident, 4, 5),
            t!(Eq, 6, 7),
            t!(TemplateHead, 8, 12),   // `a${
            t!(Ident, 12, 13),         // b
            t!(Plus, 14, 15),
            t!(TemplateHead, 16, 19),  // the nested `${
            t!(Ident, 19, 20),         // c
            t!(RBrace, 20, 21),        // closes the nested substitution
            t!(TemplateTail, 21, 22),  // the nested template's `
            t!(RBrace, 22, 23),        // closes the outer substitution
            t!(TemplateTail, 23, 25),  // d`
            t!(Semicolon, 25, 26),
        ]
    );
}

#[test]
fn c_raw_string_uses_its_delimiter() {
    let src = load("raw.c");
    assert_eq!(src.len(), 42);
    assert_eq!(
        tokens(&src, Language::C),
        vec![
            t!(Ident, 0, 5),      // const
            t!(Ident, 6, 10),     // char
            t!(Star, 11, 12),     // *
            t!(Ident, 12, 13),    // s
            t!(Eq, 14, 15),
            // `)"` appears inside, but only `)xx"` closes the literal.
            t!(RawString, 16, 29),
            t!(Semicolon, 29, 30),
            t!(Ident, 31, 34),    // int
            t!(Ident, 35, 36),    // x
            t!(Eq, 37, 38),
            t!(Number, 39, 40),
            t!(Semicolon, 40, 41),
        ]
    );
    let toks = tokenize(&src, Language::C);
    assert_eq!(
        &src[toks[5].start as usize..toks[5].end as usize],
        br#"R"xx(a)"b)xx""#
    );
}

#[test]
fn slash_is_a_regex_after_equals_and_division_after_an_identifier() {
    let src = load("regex.js");
    assert_eq!(
        tokens(&src, Language::JavaScript),
        vec![
            t!(Ident, 0, 3),
            t!(Ident, 4, 6),      // re
            t!(Eq, 7, 8),
            t!(Regex, 9, 19),     // /ab[/]c/gi
            t!(Semicolon, 19, 20),
            t!(Ident, 21, 24),    // let
            t!(Ident, 25, 26),    // d
            t!(Eq, 27, 28),
            t!(Ident, 29, 30),    // a
            t!(Slash, 31, 32),    // division: `a` can end an expression
            t!(Ident, 33, 34),    // b
            t!(Semicolon, 34, 35),
            t!(Ident, 36, 37),    // f
            t!(LParen, 37, 38),
            t!(Regex, 38, 42),    // after `(` a slash can only be a regex
            t!(Comma, 42, 43),
            t!(Ident, 44, 45),    // d
            t!(RParen, 45, 46),
            t!(Semicolon, 46, 47),
        ]
    );
}
