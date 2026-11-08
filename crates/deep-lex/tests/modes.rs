//! Mode stack, parser-driven modes, and the layout cases not covered by the
//! Python fixture.

use deep_lex::{
    DeepLexer, IndentStack, Kind, Language, Lexer, Mode, ModeStack, INDENT_CAP, MODE_CAP,
};

fn kinds(src: &[u8], lang: Language) -> Vec<(Kind, u32, u32)> {
    deep_lex::tokenize(src, lang)
        .into_iter()
        .map(|t| (Kind::from_symbol(t.symbol).unwrap(), t.start, t.end))
        .collect()
}

#[test]
fn a_state_can_force_regex_instead_of_division() {
    // `a /b/ c`: by the default rule `/` after an identifier is division.
    let src = b"a /b/ c";
    let plain = kinds(src, Language::JavaScript);
    assert_eq!(plain[1].0, Kind::Slash);
    assert_eq!(plain[1], (Kind::Slash, 2, 3));

    // The parser says state 2 is a regex context.
    let table = [Mode::Default, Mode::Default, Mode::Regex, Mode::Default];
    let mut lexer = DeepLexer::new(src, Language::JavaScript).with_state_modes(&table);
    let first = lexer.next_token(0).unwrap();
    assert_eq!((first.start, first.end), (0, 1));
    let second = lexer.next_token(2).unwrap();
    assert_eq!(
        Kind::from_symbol(second.symbol),
        Some(Kind::Regex),
        "state 2 selects regex mode"
    );
    assert_eq!((second.start, second.end), (2, 5));
    assert_eq!(lexer.mode(), "regex", "the hint is the mode for that token");
    let third = lexer.next_token(0).unwrap();
    assert_eq!((third.start, third.end), (6, 7));
    assert_eq!(lexer.mode(), "default", "state 0's hint drops back");
}

#[test]
fn template_modes_are_visible_between_tokens() {
    let src = b"x = `a${b}c`;";
    let mut lexer = DeepLexer::new(src, Language::JavaScript);
    let mut seen = Vec::new();
    let mut spans = Vec::new();
    while let Some(t) = lexer.next_token(0) {
        seen.push(lexer.mode());
        spans.push((t.start, t.end));
    }
    assert_eq!(
        spans,
        vec![(0, 1), (2, 3), (4, 8), (8, 9), (9, 10), (10, 12), (12, 13)]
    );
    assert_eq!(
        seen,
        vec![
            "default",       // x
            "default",       // =
            "template-expr", // `a${ pushes the substitution mode
            "template-expr", // b
            "template",      // } closes it and returns to the template
            "default",       // c` closes the template
            "default",
        ]
    );
}

#[test]
fn nested_templates_keep_one_frame_each() {
    let src = b"`${`${x}`}`";
    let mut lexer = DeepLexer::new(src, Language::JavaScript);
    let mut out = Vec::new();
    while let Some(t) = lexer.next_token(0) {
        out.push((Kind::from_symbol(t.symbol).unwrap(), t.start, t.end));
    }
    assert_eq!(
        out,
        vec![
            (Kind::TemplateHead, 0, 3),   // `${
            (Kind::TemplateHead, 3, 6),   // `${
            (Kind::Ident, 6, 7),          // x
            (Kind::RBrace, 7, 8),
            (Kind::TemplateTail, 8, 9),   // `
            (Kind::RBrace, 9, 10),
            (Kind::TemplateTail, 10, 11), // `
        ]
    );
    assert_eq!(lexer.mode(), "default");
    assert_eq!(lexer.modes().len(), 1);
}

#[test]
fn groups_and_continuations_suppress_python_layout() {
    let src = b"a = (1,\n     2)\nb = 3\n";
    assert_eq!(
        kinds(src, Language::Python),
        vec![
            (Kind::Ident, 0, 1),
            (Kind::Eq, 2, 3),
            (Kind::LParen, 4, 5),
            (Kind::Number, 5, 6),
            (Kind::Comma, 6, 7),
            (Kind::Number, 13, 14), // no NEWLINE or INDENT inside ( )
            (Kind::RParen, 14, 15),
            (Kind::Newline, 15, 15),
            (Kind::Ident, 16, 17),
            (Kind::Eq, 18, 19),
            (Kind::Number, 20, 21),
            (Kind::Newline, 21, 21),
        ]
    );
}

#[test]
fn blank_and_comment_only_lines_emit_nothing() {
    let src = b"if x:\n\n    # only a comment\n    y = 1\n";
    assert_eq!(
        kinds(src, Language::Python),
        vec![
            (Kind::Ident, 0, 2),
            (Kind::Ident, 3, 4),
            (Kind::Colon, 4, 5),
            (Kind::Newline, 5, 5),
            (Kind::Indent, 32, 32),
            (Kind::Ident, 32, 33), // y
            (Kind::Eq, 34, 35),
            (Kind::Number, 36, 37),
            (Kind::Newline, 37, 37),
            (Kind::Dedent, 37, 37), // on the last token of the block, as the old lexer placed it
        ]
    );
}

#[test]
fn errors_never_panic_on_malformed_bytes() {
    // Invalid UTF-8, an unterminated block comment, a stray backslash and a
    // lone quote: every byte must still produce a token and advance.
    let src = b"\xff\xfe /* open \x80 \\ '\n";
    let toks = deep_lex::tokenize(src, Language::Python);
    assert!(!toks.is_empty());
    let mut pos = 0u32;
    for t in &toks {
        assert!(t.start >= pos, "tokens never go backwards");
        assert!(t.end <= src.len() as u32);
        pos = t.end;
    }
}

#[test]
fn mode_stack_caps_without_panicking() {
    let mut stack = ModeStack::new();
    let mut pushed = 0;
    while stack.push(Mode::TemplateExpr, 0) {
        pushed += 1;
        assert!(pushed <= MODE_CAP);
    }
    assert_eq!(stack.depth(), MODE_CAP);
    assert_eq!(stack.top(), Mode::TemplateExpr);
    stack.set_top_depth(7);
    assert_eq!(stack.top_depth(), 7);
    while stack.pop() {}
    assert_eq!(stack.depth(), 1);
    assert_eq!(stack.top(), Mode::Default, "the bottom frame survives");
}

#[test]
fn indent_stack_keeps_its_base() {
    let mut stack = IndentStack::new();
    assert_eq!(stack.top(), 0);
    assert!(stack.push(4));
    assert!(stack.push(8));
    assert_eq!(stack.as_slice(), &[0, 4, 8]);
    assert_eq!(stack.pop_to_base(), 2);
    assert_eq!(stack.top(), 0);
    assert!(!stack.pop(), "the base level cannot be popped");
    for i in 1..INDENT_CAP {
        assert!(stack.push(i as u32));
    }
    assert!(!stack.push(999), "overflow is reported, not a panic");
}

#[test]
fn symbols_can_be_remapped_per_grammar() {
    use deep_lex::SymbolMap;

    let mut map = SymbolMap::identity();
    assert_eq!(map.get(Kind::Ident), Kind::Ident as u16);
    map.set(Kind::Ident, 700);
    map.set(Kind::Newline, 701);

    let mut lexer = DeepLexer::new(b"a\n", Language::Python).with_symbols(map);
    let first = lexer.next_token(0).unwrap();
    assert_eq!((first.symbol, first.start, first.end), (700, 0, 1));
    let newline = lexer.next_token(0).unwrap();
    assert_eq!((newline.symbol, newline.start, newline.end), (701, 1, 1));
}
