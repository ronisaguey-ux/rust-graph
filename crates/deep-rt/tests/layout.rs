//! Layout as tokens, inferred by the runtime instead of by a grammar.
//!
//! Python and Nim are the reason [`Layout`] exists: indentation is syntax, and a
//! grammar written in `grammar!` cannot say "a block is a deeper line". The
//! runtime's lexer therefore invents three zero-width tokens — `NEWLINE` after the
//! last token of a logical line, `INDENT` before the first token of a deeper line,
//! `DEDENT` just after the `NEWLINE` that ends a block — and the grammar consumes
//! them like any other token.
//!
//! These tests use a four-line language written by hand, not Python, so a failure
//! points at the layout pass and nothing else. The shipped Python grammar is
//! covered token for token by `grammar_parity.rs`.

use deep_rt::lexer::Cursor;
use deep_rt::syntax::{Class, Kind, KindInfo, Language, Layout, COMMENT, FIRST_LANGUAGE_KIND, UNKNOWN, WHITESPACE};
use deep_rt::{Budget, Parser, Workspace};

const NEWLINE: Kind = FIRST_LANGUAGE_KIND;
const INDENT: Kind = NEWLINE + 1;
const DEDENT: Kind = NEWLINE + 2;
const IDENT: Kind = NEWLINE + 3;
const EQ: Kind = NEWLINE + 4;
const NUMBER: Kind = NEWLINE + 5;
/// A token that cannot continue a bracketed expression: a reset.
const DEF: Kind = NEWLINE + 6;
const LPAREN: Kind = NEWLINE + 7;
const RPAREN: Kind = NEWLINE + 8;
const PLUS: Kind = NEWLINE + 12;
const MODULE: Kind = NEWLINE + 9;
const BLOCK: Kind = NEWLINE + 10;
const STATEMENT: Kind = NEWLINE + 11;

const KINDS: &[KindInfo] = &[
    KindInfo { name: "newline", class: Class::Token },
    KindInfo { name: "indent", class: Class::Open(0) },
    KindInfo { name: "dedent", class: Class::Close(0) },
    KindInfo { name: "identifier", class: Class::Token },
    KindInfo { name: "=", class: Class::Token },
    KindInfo { name: "number", class: Class::Token },
    KindInfo { name: "def", class: Class::Token },
    KindInfo { name: "(", class: Class::Open(1) },
    KindInfo { name: ")", class: Class::Close(1) },
    KindInfo { name: "module", class: Class::Node },
    KindInfo { name: "block", class: Class::Node },
    KindInfo { name: "statement", class: Class::Node },
    KindInfo { name: "+", class: Class::Token },
];

/// `name = number`, each on its own line, nested by indentation:
///
/// ```text
/// module:    _statement*
/// statement: IDENT "=" NUMBER NEWLINE | INDENT _statement* DEDENT
/// ```
pub static MINI: Language = Language::new("mini", MODULE, KINDS, lex, module).with_layout(&Layout {
    newline: NEWLINE,
    indent: INDENT,
    dedent: DEDENT,
    resets: &[DEF],
});

fn module(p: &mut Parser) {
    while !p.at_end() {
        statement(p);
    }
}

fn statement(p: &mut Parser) {
    // A stray `NEWLINE` (a blank line inside a block) is its own small node, so a
    // body of statements is a list and never a sibling of its parent's `DEDENT`.
    if p.at(NEWLINE) {
        let marker = p.open();
        p.bump();
        p.close(marker, MODULE);
        return;
    }
    if p.at(INDENT) {
        // `indent` and `dedent` are an Open/Close pair, so a group over the body
        // ends at the `dedent` and cannot see past it: the same firewall as `{...}`.
        p.group(BLOCK, body);
        return;
    }
    let marker = p.open();
    p.bump();
    p.expect(EQ);
    p.expect(NUMBER);
    p.expect(NEWLINE);
    p.close(marker, STATEMENT);
}

fn body(p: &mut Parser) {
    while !p.at_end() {
        statement(p);
    }
}

fn lex(c: &mut Cursor) -> Kind {
    match c.peek() {
        b'\n' => {
            c.bump();
            NEWLINE
        }
        b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c'));
            WHITESPACE
        }
        b'#' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'(' => {
            c.bump();
            LPAREN
        }
        b')' => {
            c.bump();
            RPAREN
        }
        b'=' => {
            c.bump();
            EQ
        }
        b'+' => {
            c.bump();
            PLUS
        }
        b'0'..=b'9' => {
            c.eat_while(|byte| byte.is_ascii_digit());
            NUMBER
        }
        byte if byte.is_ascii_alphabetic() || byte == b'_' => {
            let rest = c.rest();
            let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
            if &rest[..length] == b"def" {
                DEF
            } else {
                IDENT
            }
        }
        _ => UNKNOWN,
    }
}

/// Every token, trivia and layout included, by kind name.
fn all(src: &str) -> Vec<&'static str> {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), &MINI, &Budget::default()).unwrap();
    (0..workspace.tokens.len()).map(|at| MINI.name(workspace.tokens.kind[at])).collect()
}

/// The tokens the grammar sees: trivia dropped, layout kept.
fn significant(src: &str) -> Vec<&'static str> {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), &MINI, &Budget::default()).unwrap();
    (0..workspace.tokens.len())
        .filter(|&at| !MINI.is_trivia(workspace.tokens.kind[at]))
        .map(|at| MINI.name(workspace.tokens.kind[at]))
        .collect()
}

/// Tokens concatenate back to the input, and only layout tokens are empty.
fn check_lossless(src: &str) {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), &MINI, &Budget::default()).unwrap();
    let tokens = &workspace.tokens;
    let mut joined = Vec::with_capacity(src.len());
    for at in 0..tokens.len() {
        let empty = tokens.start(at) == tokens.end[at];
        let layout = matches!(tokens.kind[at], NEWLINE | INDENT | DEDENT);
        assert!(!empty || layout, "token {at} is empty and is not layout");
        joined.extend_from_slice(tokens.text(src.as_bytes(), at));
    }
    assert_eq!(joined, src.as_bytes(), "tokens are not lossless");
    // The root spans the file whatever the grammar did with it.
    assert_eq!(workspace.tree.nodes[0].kind, MODULE);
    assert_eq!((workspace.tree.nodes[0].start, workspace.tree.nodes[0].end), (0, src.len() as u32));
}

#[test]
fn a_line_break_becomes_a_zero_width_newline_after_the_last_token() {
    assert_eq!(all("a = 1\n"), ["identifier", "whitespace", "=", "whitespace", "number", "newline", "whitespace"]);
    assert_eq!(significant("a = 1\n"), ["identifier", "=", "number", "newline"]);
    check_lossless("a = 1\n");
    // The blank line adds no `NEWLINE`: there was no token to end.
    assert_eq!(significant("a = 1\n\n\nb = 2\n"), ["identifier", "=", "number", "newline", "identifier", "=", "number", "newline"]);
    // A comment-only line is trivia, so it ends nothing either.
    assert_eq!(significant("# note\na = 1 # note\n"), ["identifier", "=", "number", "newline"]);
}

#[test]
fn a_deeper_line_opens_a_block_and_a_shallower_one_closes_it() {
    assert_eq!(
        significant("a = 1\nb = 2\n  c = 3\nd = 4\n"),
        ["identifier", "=", "number", "newline", "identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "dedent", "identifier", "=", "number", "newline"]
    );
    // A block over several lines closes with one `dedent` per level left.
    assert_eq!(
        significant("a = 1\n  b = 2\n    c = 3\nd = 4\n"),
        ["identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "dedent", "dedent", "identifier", "=", "number", "newline"]
    );
    // The `dedent`s sit immediately after the `newline` that ended the block.
    let mut workspace = Workspace::default();
    let src = "a = 1\n  b = 2\nd = 4\n";
    workspace.parse(src.as_bytes(), &MINI, &Budget::default()).unwrap();
    let tokens = &workspace.tokens;
    let dedent = (0..tokens.len()).find(|&at| tokens.kind[at] == DEDENT).expect("a dedent");
    assert_eq!(tokens.kind[dedent - 1], NEWLINE, "a dedent follows the newline that ends the block");
    let at = (tokens.start(dedent), tokens.end[dedent]);
    assert_eq!(at, (tokens.start(dedent - 1), tokens.end[dedent - 1]));
    check_lossless(src);
}

#[test]
fn the_end_of_the_file_closes_every_open_block() {
    assert_eq!(
        significant("a = 1\n  b = 2\n    c = 3"),
        ["identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "dedent", "dedent"]
    );
    // With no final line break there is still a `newline` for the last line, and
    // the `dedent`s follow it.
    check_lossless("a = 1\n  b = 2\n    c = 3");
}

#[test]
fn a_line_break_inside_brackets_is_whitespace() {
    assert_eq!(
        significant("a = (1 +\n2)\nb = 3\n"),
        ["identifier", "=", "(", "number", "+", "number", ")", "newline", "identifier", "=", "number", "newline"]
    );
    check_lossless("a = (1 +\n2)\nb = 3\n");
}

#[test]
fn a_reset_token_at_the_start_of_a_line_ends_the_bracket() {
    // `def` cannot continue the `(`, so the bracket ends at the line break: one
    // line is one error, not the rest of the file.
    assert_eq!(
        significant("a = (1 +\ndef\nb = 2\n"),
        ["identifier", "=", "(", "number", "+", "newline", "def", "newline", "identifier", "=", "number", "newline"]
    );
    check_lossless("a = (1 +\ndef\nb = 2\n");
}

#[test]
fn indentation_is_columns_tabs_to_eight_and_a_form_feed_starts_over() {
    // A tab is a move to column 8, so it opens a block where four spaces did not.
    let tokens = significant("a = 1\n\tb = 2\n        c = 3\n");
    assert_eq!(tokens.iter().filter(|&&kind| kind == "indent").count(), 1);
    assert_eq!(tokens.iter().filter(|&&kind| kind == "dedent").count(), 1);
    // Eight spaces land on the same column as the tab: still one block.
    assert_eq!(
        significant("a = 1\n\tb = 2\n        c = 3\nd = 4\n"),
        ["identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "identifier", "=", "number", "newline", "dedent", "identifier", "=", "number", "newline"]
    );
    // A form feed resets the column to 0, so this is a dedent to the top level.
    assert_eq!(
        significant("a = 1\n    b = 2\n\x0cc = 3\n"),
        ["identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "dedent", "identifier", "=", "number", "newline"]
    );
    check_lossless("a = 1\n    b = 2\n\x0cc = 3\n");
}

#[test]
fn a_dedent_to_a_column_no_outer_line_used_reopens_a_block() {
    // 2 then 1: column 1 was never a level, so the dedent to 2 is followed by an
    // indent to 1, which is what Python's tokenizer does with a bad dedent.
    assert_eq!(
        significant("a = 1\n    b = 2\n  c = 3\nd = 4\n"),
        ["identifier", "=", "number", "newline", "indent", "identifier", "=", "number", "newline", "dedent", "indent", "identifier", "=", "number", "newline", "dedent", "identifier", "=", "number", "newline"]
    );
}

#[test]
fn every_block_pairs_its_indent_with_one_dedent() {
    // The pair pass pairs them like brackets, so `group` can jump a block whole.
    let mut workspace = Workspace::default();
    for src in ["a = 1\n  b = 2\n", "a = 1\n  b = 2\n    c = 3\n  d = 4\ne = 5\n", "a = 1\n  b = 2\n    c = 3"] {
        workspace.parse(src.as_bytes(), &MINI, &Budget::default()).unwrap();
        assert_eq!(workspace.pairs.unclosed, 0, "{src:?}");
        assert_eq!(workspace.pairs.stray, 0, "{src:?}");
        let indents = (0..workspace.tokens.len()).filter(|&at| workspace.tokens.kind[at] == INDENT).count();
        let dedents = (0..workspace.tokens.len()).filter(|&at| workspace.tokens.kind[at] == DEDENT).count();
        assert_eq!(indents, dedents, "{src:?}");
        assert!(!workspace.tree.sexp(&MINI, true).is_empty());
    }
}

#[test]
fn three_hundred_nested_blocks_do_not_overflow_the_stack() {
    // Layout is a `Vec` of columns and the pair pass is a loop, so depth costs no
    // stack there; the grammar's own recursion is what the thread size covers.
    let depth = 300;
    let mut src = String::new();
    for level in 0..depth {
        src.push_str(&" ".repeat(level * 2));
        src.push_str("a = 1\n");
    }
    let src = src;
    let parsed = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut workspace = Workspace::default();
            let outcome = workspace.parse(src.as_bytes(), &MINI, &Budget::default()).unwrap();
            let indents = (0..workspace.tokens.len()).filter(|&at| workspace.tokens.kind[at] == INDENT).count();
            let dedents = (0..workspace.tokens.len()).filter(|&at| workspace.tokens.kind[at] == DEDENT).count();
            (outcome.complete(), indents, dedents, workspace.tokens.len())
        })
        .unwrap()
        .join()
        .unwrap();
    let (complete, indents, dedents, _) = parsed;
    assert!(complete, "a deeply nested file did not parse");
    assert_eq!((indents, dedents), (depth - 1, depth - 1));
}
