//! The external scanner seam: one custom implementation proving the trait is
//! enough for a construct no table can express, plus the shipped scanner's
//! failure paths.

use deep_lex::{
    tokenize, Cursor, DeepLexer, ExternalScanner, Kind, Language, Lexer, Mode, NativeScanner,
    NoScanner, ScanCtx, Span,
};

/// A toy language's `@"..."` string: starts with a sigil, may span lines, and
/// has no closing rule a token table could encode.
struct AtStringScanner;

impl ExternalScanner for AtStringScanner {
    fn start(&mut self, cur: &Cursor<'_>, ctx: ScanCtx) -> bool {
        ctx.mode == Mode::Default && cur.starts_with(b"@\"")
    }

    fn scan(&mut self, cur: &mut Cursor<'_>, ctx: ScanCtx) -> Option<(Kind, Span)> {
        if !(ctx.mode == Mode::Default && cur.starts_with(b"@\"")) {
            return None;
        }
        let start = cur.pos();
        cur.advance(2);
        loop {
            match cur.peek() {
                None => break, // unterminated: to EOF, never panic
                Some(b'\\') => {
                    cur.bump();
                    if cur.peek().is_some() {
                        cur.bump();
                    }
                }
                Some(b'"') => {
                    cur.bump();
                    break;
                }
                Some(_) => {
                    cur.bump();
                }
            }
        }
        Some((Kind::String, Span::new(start, cur.pos())))
    }

    fn end(&self, _ctx: ScanCtx) -> bool {
        true
    }
}

fn with_custom_scanner(src: &[u8]) -> Vec<(Kind, u32, u32)> {
    let mut lexer = DeepLexer::with_scanner(src, Language::Generic, AtStringScanner);
    let mut out = Vec::new();
    while let Some(t) = lexer.next_token(0) {
        out.push((Kind::from_symbol(t.symbol).unwrap(), t.start, t.end));
    }
    out
}

#[test]
fn a_custom_scanner_can_own_a_token_shape() {
    let src = b"a @\"x\ny\" b";
    assert_eq!(
        with_custom_scanner(src),
        vec![
            (Kind::Ident, 0, 1),
            (Kind::String, 2, 8), // the sigil is part of the span, like a quote
            (Kind::Ident, 9, 10),
        ]
    );
}

#[test]
fn a_custom_scanner_is_not_asked_where_it_declines() {
    // The same bytes without the sigil must lex as an ordinary string plus
    // operator, so `start` is doing real work.
    let src = b"a + \"x\"";
    assert_eq!(
        with_custom_scanner(src),
        vec![
            (Kind::Ident, 0, 1),
            (Kind::Plus, 2, 3),
            (Kind::String, 4, 7),
        ]
    );
}

#[test]
fn no_scanner_is_a_valid_choice() {
    let src = b"a / b";
    let mut lexer = DeepLexer::with_scanner(src, Language::JavaScript, NoScanner);
    assert!(!NoScanner.start(&lexer.cursor().clone(), ScanCtx {
        mode: Mode::Default,
        prev: Kind::Error,
        lang: Language::JavaScript,
    }));
    let mut out = Vec::new();
    while let Some(t) = lexer.next_token(0) {
        out.push(Kind::from_symbol(t.symbol).unwrap());
    }
    assert_eq!(out, vec![Kind::Ident, Kind::Slash, Kind::Ident]);
}

#[test]
fn the_shipped_scanner_reads_regex_and_division_alike() {
    let mut scanner = NativeScanner::new(Language::JavaScript);
    let regex_ctx = ScanCtx {
        mode: Mode::Default,
        prev: Kind::Eq,
        lang: Language::JavaScript,
    };
    let src = b"/a[b/]c/g";
    let cur = Cursor::new(src);
    assert!(scanner.start(&cur, regex_ctx));
    let mut cur = cur;
    let (kind, span) = scanner.scan(&mut cur, regex_ctx).unwrap();
    assert_eq!(kind, Kind::Regex);
    assert_eq!((span.start, span.end), (0, 9));
    assert_eq!(cur.pos(), 9);

    let division_ctx = ScanCtx {
        prev: Kind::Ident,
        ..regex_ctx
    };
    let cur = Cursor::new(b"/ b");
    assert!(!scanner.start(&cur, division_ctx), "after an identifier, division");

    let comment_ctx = ScanCtx { prev: Kind::Eq, ..regex_ctx };
    assert!(
        !scanner.start(&Cursor::new(b"// x"), comment_ctx),
        "a comment is not a regex"
    );
    assert!(!scanner.start(&Cursor::new(b"/* x */"), comment_ctx));
}

#[test]
fn unterminated_c_raw_string_runs_to_eof() {
    let src = b"x = R\"(abc";
    assert_eq!(
        tokenize(src, Language::C)
            .into_iter()
            .map(|t| (Kind::from_symbol(t.symbol).unwrap(), t.start, t.end))
            .collect::<Vec<_>>(),
        vec![
            (Kind::Ident, 0, 1),
            (Kind::Eq, 2, 3),
            (Kind::RawString, 4, 10),
        ]
    );
}

#[test]
fn a_malformed_raw_string_falls_back_to_ordinary_tokens() {
    // `R"bad delim(...)` has a space in the delimiter, which C forbids. The
    // scanner must decline, put the cursor back, and let the table lex `R` and
    // the quoted text.
    let src = b"R\"bad delim(x)\"";
    let toks: Vec<(Kind, u32, u32)> = tokenize(src, Language::C)
        .into_iter()
        .map(|t| (Kind::from_symbol(t.symbol).unwrap(), t.start, t.end))
        .collect();
    assert_eq!(toks[0], (Kind::Ident, 0, 1));
    assert_eq!(toks[1], (Kind::String, 1, 15));
}

#[test]
fn triple_quoted_python_strings_span_lines() {
    let src = b"s = \"\"\"a\nb\"\"\"\n";
    assert_eq!(src.len(), 14);
    assert_eq!(
        tokenize(src, Language::Python)
            .into_iter()
            .map(|t| (Kind::from_symbol(t.symbol).unwrap(), t.start, t.end))
            .collect::<Vec<_>>(),
        vec![
            (Kind::Ident, 0, 1),
            (Kind::Eq, 2, 3),
            (Kind::String, 4, 13),
            (Kind::Newline, 13, 13),
        ]
    );
}
