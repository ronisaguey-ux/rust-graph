//! Gap 2, end to end: a rule that ends at a line end.
//!
//! Elixir's `case`/`cond` clauses are told apart by nothing but the line break.
//! With `layout = NEWLINE;` the runtime infers one zero-width `newline` after
//! the last token of every logical line (none inside brackets, none on blank
//! lines), so a clause rule can end with `NEWLINE` and the next clause starts a
//! new node.
//!
//! The file proves it twice: `lined` uses the new layout form, `flat` is the
//! same grammar with the line break left as whitespace (what the DSL could
//! express before). The counterfactual is not a comment: `flat` really is built
//! and run on the same source, and it merges the two clauses into one, because
//! `foo` followed by `(bar)` is a call.

//! The runtime is compiled in by path, exactly as `deep-grammars/src/deep.rs`
//! does, so this proof adds no dependency to the macro crate: the figures below
//! are the real parser, lexer and tree, not a mock.

#[path = "../../../src/deep/budget.rs"]
pub mod budget;
#[path = "../../../src/deep/lexer.rs"]
pub mod lexer;
#[path = "../../../src/deep/pairs.rs"]
pub mod pairs;
#[path = "../../../src/deep/parser.rs"]
pub mod parser;
#[path = "../../../src/deep/syntax.rs"]
pub mod syntax;
#[path = "../../../src/deep/tree.rs"]
pub mod tree;

/// The emitted code names `crate::deep`, the macro's default runtime.
pub mod deep {
    pub use crate::{budget, lexer, pairs, parser, syntax, tree};
}

use self::deep::budget::Budget;
use self::deep::parser::Workspace;
use self::deep::syntax::{Language, ERROR};
use self::deep::tree::{HAS_ERROR, TOKEN};

mod lined {
    use crate::deep::lexer::Cursor;
    use crate::deep::syntax::{Kind, UNKNOWN};

    rust_graph_deep::grammar! { LINE {
        name = "line";
        lex = lex;

        // The gap-2 setting: line ends, no indentation.
        layout = NEWLINE;

        tokens {
            NEWLINE = "newline",
            LPAREN = "(" open parens,
            RPAREN = ")" close parens,
            COMMA = ",",
            ARROW = "->",
            PLUS = "+",
            ATOM = "atom",
            NAME = "identifier",
            NUMBER = "number",
            STRING = "string",
            NOTE = "comment" trivia,
            WS = " " trivia,

            KW_CASE = "case",
            KW_COND = "cond",
            KW_DO = "do",
            KW_END = "end",
            KW_WHEN = "when",
        }

        program: NEWLINE* (_statement NEWLINE*)*,
        _statement: case_expression | cond_expression | expression_statement,

        // One NEWLINE after `do`, one after each clause: the clause's body is
        // one expression, and it ends at the line end.
        case_expression: "case" _expression "do" NEWLINE* (case_clause NEWLINE*)* "end",
        cond_expression: "cond" "do" NEWLINE* (case_clause NEWLINE*)* "end",
        case_clause: _expression ["when" _expression] "->" _expression NEWLINE,

        expression_statement: _expression,

        _expression: precedence _primary {
            left "+" => binary,
            postfix call_arguments => call,
        },

        call_arguments: "(" [(argument %? ",")] ")",
        argument: _expression,

        _primary: NAME | ATOM | NUMBER | STRING | paren_expression,
        paren_expression: "(" _expression ")",
    } }

    pub fn lex(c: &mut Cursor) -> Kind {
        match c.peek() {
            // The lexer returns the layout token for a line break; the runtime
            // turns it into a zero-width NEWLINE after the line's last token.
            b'\n' => {
                c.bump();
                NEWLINE
            }
            b' ' | b'\t' | b'\r' => {
                c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r'));
                WS
            }
            b'#' => {
                c.eat_while(|byte| byte != b'\n');
                NOTE
            }
            b'"' => {
                c.bump();
                c.quoted(b'"', false);
                STRING
            }
            b'0'..=b'9' => {
                c.eat_while(|byte| byte.is_ascii_digit());
                NUMBER
            }
            b':' => {
                c.bump();
                c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                ATOM
            }
            b'(' => {
                c.bump();
                LPAREN
            }
            b')' => {
                c.bump();
                RPAREN
            }
            b',' => {
                c.bump();
                COMMA
            }
            b'+' => {
                c.bump();
                PLUS
            }
            b'-' if c.peek_at(1) == b'>' => {
                c.bump_n(2);
                ARROW
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = c.pos();
                c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                match c.since(start) {
                    b"case" => KW_CASE,
                    b"cond" => KW_COND,
                    b"do" => KW_DO,
                    b"end" => KW_END,
                    b"when" => KW_WHEN,
                    _ => NAME,
                }
            }
            _ => {
                c.bump_char();
                UNKNOWN
            }
        }
    }
}

mod flat {
    use crate::deep::lexer::Cursor;
    use crate::deep::syntax::{Kind, UNKNOWN};

    // The same language with the line break as whitespace: no layout, no
    // NEWLINE token. It is the counterfactual the gap names.
    rust_graph_deep::grammar! { FLAT {
        name = "flat";
        lex = lex;

        tokens {
            LPAREN = "(" open parens,
            RPAREN = ")" close parens,
            COMMA = ",",
            ARROW = "->",
            PLUS = "+",
            ATOM = "atom",
            NAME = "identifier",
            NUMBER = "number",
            STRING = "string",
            NOTE = "comment" trivia,
            WS = " " trivia,

            KW_CASE = "case",
            KW_COND = "cond",
            KW_DO = "do",
            KW_END = "end",
            KW_WHEN = "when",
        }

        program: _statement*,
        _statement: case_expression | expression_statement,
        case_expression: "case" _expression "do" case_clause* "end",
        case_clause: _expression ["when" _expression] "->" _expression,
        expression_statement: _expression,

        _expression: precedence _primary {
            left "+" => binary,
            postfix call_arguments => call,
        },
        call_arguments: "(" [(argument %? ",")] ")",
        argument: _expression,

        _primary: NAME | ATOM | NUMBER | STRING | paren_expression,
        paren_expression: "(" _expression ")",
    } }

    pub fn lex(c: &mut Cursor) -> Kind {
        match c.peek() {
            // Here a line break is just trivia: what layout did not exist for.
            b'\n' => {
                c.bump();
                WS
            }
            b' ' | b'\t' | b'\r' => {
                c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r'));
                WS
            }
            b'#' => {
                c.eat_while(|byte| byte != b'\n');
                NOTE
            }
            b'"' => {
                c.bump();
                c.quoted(b'"', false);
                STRING
            }
            b'0'..=b'9' => {
                c.eat_while(|byte| byte.is_ascii_digit());
                NUMBER
            }
            b':' => {
                c.bump();
                c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                ATOM
            }
            b'(' => {
                c.bump();
                LPAREN
            }
            b')' => {
                c.bump();
                RPAREN
            }
            b',' => {
                c.bump();
                COMMA
            }
            b'+' => {
                c.bump();
                PLUS
            }
            b'-' if c.peek_at(1) == b'>' => {
                c.bump_n(2);
                ARROW
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = c.pos();
                c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                match c.since(start) {
                    b"case" => KW_CASE,
                    b"cond" => KW_COND,
                    b"do" => KW_DO,
                    b"end" => KW_END,
                    b"when" => KW_WHEN,
                    _ => NAME,
                }
            }
            _ => {
                c.bump_char();
                UNKNOWN
            }
        }
    }
}

fn parse(src: &str) -> Workspace {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), &lined::LINE, &Budget::default()).expect("the input fits");
    workspace
}

fn parse_flat(src: &str) -> Workspace {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), &flat::FLAT, &Budget::default()).expect("the input fits");
    workspace
}

fn errors(workspace: &Workspace) -> usize {
    workspace
        .tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(at, node)| node.kind == ERROR || (*at > 0 && node.flags & HAS_ERROR != 0 && node.flags & TOKEN != 0))
        .count()
}

fn count(workspace: &Workspace, language: &Language, name: &str) -> usize {
    workspace.tree.nodes.iter().filter(|node| language.name(node.kind) == name).count()
}

#[test]
fn case_clauses_end_at_the_line_end() {
    let workspace = parse(
        r#"case x do
  1 -> "one"
  2 -> "two"
end
"#,
    );
    assert_eq!(errors(&workspace), 0, "no error nodes:\n{}", workspace.tree.sexp(&lined::LINE, true));
    assert_eq!(count(&workspace, &lined::LINE, "case_clause"), 2);
    assert_eq!(count(&workspace, &lined::LINE, "case_expression"), 1);
}

#[test]
fn cond_clauses_end_at_the_line_end() {
    let workspace = parse(
        r#"cond do
  x -> :first
  y -> :second
  true -> :neither
end
"#,
    );
    assert_eq!(errors(&workspace), 0, "no error nodes:\n{}", workspace.tree.sexp(&lined::LINE, true));
    assert_eq!(count(&workspace, &lined::LINE, "cond_expression"), 1);
    assert_eq!(count(&workspace, &lined::LINE, "case_clause"), 3);
}

#[test]
fn a_clause_body_does_not_swallow_the_next_clause_head() {
    // `foo` ends its line, so `(bar)` starts the next clause. Read as
    // whitespace, `foo (bar)` is one call and the `->` dangles: exactly one
    // clause, with an error.
    let src = "case x do\n  1 -> foo\n  (bar) -> baz\nend\n";
    let lined = parse(src);
    assert_eq!(errors(&lined), 0, "no error nodes:\n{}", lined.tree.sexp(&lined::LINE, true));
    assert_eq!(count(&lined, &lined::LINE, "case_clause"), 2);
    assert_eq!(count(&lined, &lined::LINE, "paren_expression"), 1, "the second head:\n{}", lined.tree.sexp(&lined::LINE, true));
    assert_eq!(count(&lined, &lined::LINE, "call"), 0, "no call spans the line break:\n{}", lined.tree.sexp(&lined::LINE, true));

    let flat = parse_flat(src);
    assert_eq!(count(&flat, &flat::FLAT, "case_clause"), 1, "without the line boundary the clauses merge:\n{}", flat.tree.sexp(&flat::FLAT, true));
    assert!(errors(&flat) > 0, "and the arrow is left over:\n{}", flat.tree.sexp(&flat::FLAT, true));
}

#[test]
fn a_multiline_head_keeps_going_inside_brackets() {
    // `(1 +\n2)` is inside brackets, where the runtime inserts no NEWLINE, so
    // the head is one expression over two physical lines.
    let workspace = parse("case (1 +\n   2) do\n  3 -> \"three\"\n  4 -> \"four\"\nend\n");
    assert_eq!(errors(&workspace), 0, "no error nodes:\n{}", workspace.tree.sexp(&lined::LINE, true));
    assert_eq!(count(&workspace, &lined::LINE, "case_clause"), 2);
}

#[test]
fn the_inferred_newline_is_zero_width_and_tokens_still_cover_the_input() {
    let src = "case x do\n  1 -> \"one\"\nend\n";
    let workspace = parse(src);
    let mut end = 0u32;
    for (index, kind) in workspace.tokens.kind.iter().enumerate() {
        assert_eq!(workspace.tokens.start(index), end, "a gap before token {index} ({})", lined::LINE.name(*kind));
        end = workspace.tokens.end[index];
    }
    assert_eq!(end as usize, src.len(), "the tokens concatenate back to the input");
    assert!(workspace.tokens.kind.contains(&lined::NEWLINE), "the layout token is in the stream");
    assert!(workspace.tokens.kind.contains(&crate::deep::syntax::WHITESPACE), "the line break itself is trivia");
}
