//! Gap 3, end to end: a lexer mode declared by the grammar.
//!
//! A heredoc's body cannot be lexed by a stateless step function: at the line
//! break after `<<EOS` nothing in the next bytes says a heredoc is open, so
//! `hello` and `world` would come back as identifiers. `modes { BODY = lex_body, }`
//! names a second step function, and the two `switch` lines say when it runs:
//! after the `<<EOS` token and after the body it returns. The macro generates the
//! stack, clears it at byte 0 of every file, and dispatches on its top frame; the
//! runtime still calls a plain `fn(&mut Cursor) -> Kind`.
//!
//! The terminator is a fixed `EOS` because a step function cannot see the token
//! that came before it (`Cursor` exposes the bytes from the cursor on, not the
//! previous token's text); remembering the delimiter is per-mode state, which
//! stays in the step function. What the DSL adds is the mode itself.
//!
//! The runtime is compiled in by path, as `deep-grammars/src/deep.rs` does, so
//! this proof adds no dependency to the macro crate.

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

mod heredoc {
    use crate::deep::lexer::Cursor;
    use crate::deep::syntax::{Kind, UNKNOWN};

    rust_graph_deep::grammar! { HERE {
        name = "here";
        lex = lex_code;

        modes {
            BODY = lex_body,
        }
        switch HEREDOC_START => BODY;
        switch HEREDOC_BODY => pop;

        tokens {
            ASSIGN = "=",
            HEREDOC_START = "heredoc_start",
            HEREDOC_BODY = "heredoc_body",
            NAME = "identifier",
            NUMBER = "number",
            WS = " " trivia,
        }

        program: _statement*,
        _statement: assignment,
        assignment: NAME "=" (NUMBER | heredoc),
        heredoc: HEREDOC_START HEREDOC_BODY,
    } }

    /// Mode 0: code. `<<EOS` is one token; the next step runs in mode 1.
    pub fn lex_code(c: &mut Cursor) -> Kind {
        match c.peek() {
            b' ' | b'\t' | b'\n' | b'\r' => {
                c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
                WS
            }
            b'<' if c.peek_at(1) == b'<' => {
                c.bump_n(2);
                c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                HEREDOC_START
            }
            b'=' => {
                c.bump();
                ASSIGN
            }
            b'0'..=b'9' => {
                c.eat_while(|byte| byte.is_ascii_digit());
                NUMBER
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                NAME
            }
            _ => {
                c.bump_char();
                UNKNOWN
            }
        }
    }

    /// Mode 1: the body. Called first at the line break that ends the `<<EOS`
    /// line, and runs to the line that is exactly `EOS`.
    pub fn lex_body(c: &mut Cursor) -> Kind {
        while !c.at_end() {
            let start = c.pos();
            c.eat_while(|byte| byte != b'\n');
            if c.since(start) == b"EOS" {
                return HEREDOC_BODY;
            }
            if c.peek() == b'\n' {
                c.bump();
            }
        }
        HEREDOC_BODY
    }
}

use self::deep::budget::Budget;
use self::deep::parser::Workspace;
use self::deep::syntax::{Language, ERROR};
use self::deep::tree::{HAS_ERROR, TOKEN};

fn parse(src: &str) -> Workspace {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), &heredoc::HERE, &Budget::default()).expect("the input fits");
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
fn a_heredoc_body_is_one_token_lexed_in_the_declared_mode() {
    let src = "greeting = <<EOS\nhello\nworld\nEOS\ncount = 3\n";
    let workspace = parse(src);
    assert_eq!(errors(&workspace), 0, "no error nodes:\n{}", workspace.tree.sexp(&heredoc::HERE, true));
    assert_eq!(count(&workspace, &heredoc::HERE, "assignment"), 2);
    assert_eq!(count(&workspace, &heredoc::HERE, "heredoc"), 1);
    // `hello` and `world` are not identifiers: they never reach the code mode.
    assert_eq!(count(&workspace, &heredoc::HERE, "identifier"), 2, "`greeting` and `count` only:\n{}", workspace.tree.sexp(&heredoc::HERE, true));

    // The body token is the bytes from the line break to the terminator line.
    let body = workspace
        .tree
        .nodes
        .iter()
        .find(|node| node.flags & TOKEN != 0 && heredoc::HERE.name(node.kind) == "heredoc_body")
        .expect("a heredoc body token");
    let text = &src[body.start as usize..body.end as usize];
    assert_eq!(text, "\nhello\nworld\nEOS", "the body is one token, terminator included");
}

#[test]
fn the_mode_pops_after_the_body_and_code_resumes() {
    // `count = 3` follows the heredoc: if the pop did not happen, the `=` and
    // the number would be lexed by the body lexer.
    let workspace = parse("a = <<EOS\nbody\nEOS\nb = 2\nc = 3\n");
    assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&heredoc::HERE, true));
    assert_eq!(count(&workspace, &heredoc::HERE, "assignment"), 3);
    assert_eq!(count(&workspace, &heredoc::HERE, "number"), 2);
}

#[test]
fn two_heredocs_in_one_file_each_enter_and_leave_the_mode() {
    let workspace = parse("a = <<EOS\none\nEOS\nb = <<EOS\ntwo\nEOS\n");
    assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&heredoc::HERE, true));
    assert_eq!(count(&workspace, &heredoc::HERE, "heredoc"), 2);
}

#[test]
fn the_mode_stack_is_cleared_between_files() {
    // The stack is a thread local, and a file that ends inside the body mode
    // must not leave it set for the next file.
    let first = parse("a = <<EOS\nunterminated\n");
    assert!(errors(&first) > 0 || count(&first, &heredoc::HERE, "heredoc") == 1);
    let second = parse("b = 2\n");
    assert_eq!(errors(&second), 0, "the second file starts in mode 0:\n{}", second.tree.sexp(&heredoc::HERE, true));
    assert_eq!(count(&second, &heredoc::HERE, "assignment"), 1);
}
