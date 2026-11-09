//! The hand-written step lexers, feature by feature: strings (single, double,
//! triple, raw, template), comments, numbers and, for Python, the layout tokens
//! `NEWLINE`, `INDENT` and `DEDENT`.
//!
//! These read the tokens the runtime kept, so they check the lexer and the
//! layout pass together, not the tree.

use deep_grammars::deep::syntax::{Class, Language};
use deep_grammars::deep::{Budget, Workspace};
use deep_grammars::javascript::{JAVASCRIPT, TYPESCRIPT};
use deep_grammars::python::PYTHON;

/// Every token, trivia included, by kind name.
fn tokens(language: &'static Language, src: &str) -> Vec<&'static str> {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap_or_else(|_| panic!("{}: too large", language.name));
    (0..workspace.tokens.len()).map(|at| language.name(workspace.tokens.kind[at])).collect()
}

/// The tokens the grammar sees: trivia and layout-only bytes dropped.
fn significant(language: &'static Language, src: &str) -> Vec<&'static str> {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
    (0..workspace.tokens.len())
        .filter(|&at| !language.is_trivia(workspace.tokens.kind[at]))
        .map(|at| language.name(workspace.tokens.kind[at]))
        .collect()
}

/// The class of the first non-trivia token `src` lexes to.
fn class_of(language: &'static Language, src: &str) -> Class {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
    let kinds: Vec<_> = (0..workspace.tokens.len())
        .filter(|&at| !language.is_trivia(workspace.tokens.kind[at]))
        .map(|at| workspace.tokens.kind[at])
        .collect();
    assert!(!kinds.is_empty(), "{src:?} lexed to nothing");
    language.class(kinds[0])
}

#[test]
fn python_string_prefixes_are_one_token() {
    for src in [r#"x = "plain""#, "x = 'plain'", "x = b\"bytes\\n\"", "x = rb'\\d+'", "x = '''multi\nline'''"] {
        assert_eq!(significant(&PYTHON, src), ["identifier", "=", "string_literal", "newline"], "{src:?}");
        assert_eq!(class_of(&PYTHON, src.trim_start_matches("x = ")), Class::Token);
    }
    // An f-string is not one token: its quotes, the braces of each field, the
    // code between them and the bytes of a format spec are tokens of their own,
    // so the expression in a field is lexed as code and parsed. `{a!r:>{w}}`
    // is a field around `a`, a conversion, a spec, and one nested field.
    let src = r#"x = f"{a!r:>{w}}""#;
    assert_eq!(
        significant(&PYTHON, src),
        [
            "identifier",
            "=",
            "fstring_start",
            "fstring_field_open",
            "identifier",
            "!",
            "identifier",
            ":",
            "fstring_spec_text",
            "fstring_field_open",
            "identifier",
            "fstring_field_close",
            "fstring_field_close",
            "fstring_end",
            "newline",
        ],
        "{src:?}"
    );
    assert_eq!(class_of(&PYTHON, src.trim_start_matches("x = ")), Class::Token);
}

#[test]
fn python_comments_and_numbers() {
    assert_eq!(
        significant(&PYTHON, "y = 0x1F + 1_000.5e-3j + 0b1010 + 0o77\n"),
        ["identifier", "=", "number", "+", "number", "+", "number", "+", "number", "newline"]
    );
    assert_eq!(
        tokens(&PYTHON, "y = 1  # note\n"),
        ["identifier", "whitespace", "=", "whitespace", "number", "newline", "whitespace", "comment", "whitespace"]
    );
}

#[test]
fn python_layout_is_newline_indent_dedent() {
    assert_eq!(
        significant(&PYTHON, "if x:\n    pass\n\n    # c\n    y = 1\nz = 2\n"),
        ["if", "identifier", ":", "newline", "indent", "pass", "newline", "identifier", "=", "number", "newline", "dedent", "identifier", "=", "number", "newline"]
    );
    // Inside brackets a line break is whitespace; a backslash joins the line.
    assert_eq!(
        significant(&PYTHON, "x = (1,\n  2) + \\\n  3\n"),
        ["identifier", "=", "(", "number", ",", "number", ")", "+", "number", "newline"]
    );
    // The end of the file closes every open block.
    assert_eq!(
        significant(&PYTHON, "def f():\n  if a:\n    b"),
        ["def", "identifier", "(", ")", ":", "newline", "indent", "if", "identifier", ":", "newline", "indent", "identifier", "newline", "dedent", "dedent"]
    );
}

#[test]
fn python_an_unterminated_string_is_a_bad_token() {
    assert_eq!(class_of(&PYTHON, "\"open\n"), Class::BadToken);
}

#[test]
fn javascript_string_forms() {
    assert_eq!(
        significant(&JAVASCRIPT, "const s = 'a' + \"b\" + `t ${x + 1}`;\n"),
        ["const", "identifier", "=", "string", "+", "string", "+", "template_string", ";"]
    );
    let mut workspace = Workspace::default();
    workspace.parse(b"const t = `a ${`b ${c}`} d`;", &JAVASCRIPT, &Budget::default()).unwrap();
    let templates = (0..workspace.tokens.len())
        .filter(|&at| workspace.tokens.kind[at] == deep_grammars::javascript::TEMPLATE)
        .count();
    // The nested `${}` parts are scanned with their braces balanced: the outer
    // template is one token, and the inner one is not a token of its own.
    assert_eq!(templates, 1);
}

#[test]
fn javascript_comments_and_numbers() {
    assert_eq!(
        significant(&JAVASCRIPT, "/* block */ x = 0x1f + 1_000.5e3 + 0b1010n;\n"),
        ["identifier", "=", "number", "+", "number", "+", "number", ";"]
    );
    assert_eq!(
        tokens(&JAVASCRIPT, "// line\nx = 1;\n"),
        ["comment", "whitespace", "identifier", "whitespace", "=", "whitespace", "number", ";", "whitespace"]
    );
}

#[test]
fn javascript_a_regex_is_a_token_only_where_an_operand_starts() {
    assert_eq!(significant(&JAVASCRIPT, "const re = /ab+c/gi;\n"), ["const", "identifier", "=", "regex", ";"]);
    // After an operand `/` is division, not the start of a regular expression.
    assert_eq!(significant(&JAVASCRIPT, "x = a / b / c;\n"), ["identifier", "=", "identifier", "/", "identifier", "/", "identifier", ";"]);
}

#[test]
fn typescript_keeps_nested_generic_closers_apart() {
    // `>>` closes two type argument lists; `>joined` is the first of the two.
    assert_eq!(
        significant(&TYPESCRIPT, "let n: Array<Map<string, number>> = [];\n"),
        ["let", "identifier", ":", "identifier", "<", "identifier", "<", "identifier", ",", "identifier", ">joined", ">", "=", "[", "]", ";"]
    );
    assert_eq!(significant(&TYPESCRIPT, "x = a >> b;\n"), ["identifier", "=", "identifier", ">joined", ">", "identifier", ";"]);
}

#[test]
fn typescript_an_unterminated_string_is_a_bad_token() {
    assert_eq!(class_of(&TYPESCRIPT, "\"open\n"), Class::BadToken);
}
