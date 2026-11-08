//! Tcl: commands made of words, terminated by a newline or `;`.
//!
//! Everything in Tcl is a command whose arguments are words; `{...}` is one
//! braced word and `[...]` is a command substitution, so a `proc` body is a
//! single word here and is not parsed as code. That is Tcl's own model, not a
//! shortcut: the interpreter does not see the body as commands either until it
//! evaluates the `proc`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A script inside a brace is one token.** `proc p {} { set x 1 }` gives
//!   `proc`, its name, `{}` and the body as a braced word; the body's commands
//!   are not nodes. Only `[...]` substitutions are separate tokens, and their
//!   contents are one token too.
//! - **`if`/`while`/`for` bodies** have the same shape: their braces are words,
//!   so the control flow is a command, not a block node.
//! - **Variable substitution** `$x`, `$x(idx)` is one `variable` token.
//! - **A missing final newline** is an error: every command needs its
//!   terminator.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { TCL {
    name = "tcl";
    lex = lex_tcl;

    tokens {
        NEWLINE = "newline",
        SEMI = ";",
        /// One word with no quoting: `set`, `puts`, `-nonewline`.
        WORD = "word",
        /// `{ ... }` including its braces.
        BRACED = "braced_word",
        /// `" ... "` including its quotes.
        QUOTED = "quoted_word",
        /// `[ ... ]`, a command substitution.
        BRACKET = "command_substitution",
        /// `$name`, `${name}`, `$name(index)`.
        VARIABLE = "variable",
        BAD_STRING = "bad_string" bad,
        KW_PROC = "proc",
    }

    program: (NEWLINE | ";" | _command)*,

    _command: proc_command | call_command,

    proc_command: "proc" WORD _word* command_end,
    call_command: _word+ command_end,

    command_end: NEWLINE | ";",
    _word: WORD | BRACED | QUOTED | BRACKET | VARIABLE,
} }

pub fn lex_tcl(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r'));
            WHITESPACE
        }
        b'\\' if c.peek_at(1) == b'\n' => { c.bump_n(2); WHITESPACE }
        b'\n' => { c.bump(); NEWLINE }
        b';' => { c.bump(); SEMI }
        b'#' => {
            // A comment is only a comment where a command may start.
            if matches!(c.previous(), crate::deep::syntax::UNKNOWN | NEWLINE | SEMI) {
                lex::line_comment(c);
                COMMENT
            } else {
                c.bump();
                WORD
            }
        }
        b'{' => {
            c.bump();
            let mut depth = 1;
            while !c.at_end() {
                match c.peek() {
                    b'\\' => c.bump_n(2),
                    b'{' => { depth += 1; c.bump(); }
                    b'}' => {
                        c.bump();
                        depth -= 1;
                        if depth == 0 {
                            return BRACED;
                        }
                    }
                    _ => c.bump_char(),
                }
            }
            BAD_STRING
        }
        b'"' => {
            c.bump();
            loop {
                if c.at_end() {
                    return BAD_STRING;
                }
                match c.peek() {
                    b'\\' => c.bump_n(2),
                    b'"' => { c.bump(); return QUOTED; }
                    _ => c.bump_char(),
                }
            }
        }
        b'[' => {
            c.bump();
            let mut depth = 1;
            while !c.at_end() {
                match c.peek() {
                    b'\\' => c.bump_n(2),
                    b'[' => { depth += 1; c.bump(); }
                    b']' => {
                        c.bump();
                        depth -= 1;
                        if depth == 0 {
                            return BRACKET;
                        }
                    }
                    _ => c.bump_char(),
                }
            }
            BAD_STRING
        }
        b'$' => {
            c.bump();
            if c.peek() == b'{' {
                c.eat_while(|byte| byte != b'}' && byte != b'\n');
                c.eat(b'}');
            } else {
                c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'));
                if c.peek() == b'(' {
                    c.bump();
                    c.eat_while(|byte| byte != b')' && byte != b'\n');
                    c.eat(b')');
                }
            }
            VARIABLE
        }
        _ => {
            let start = c.rest();
            let length = c.eat_while(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b';' | b'{' | b'"' | b'[' | b'$' | b'\\'));
            if length == 0 {
                c.bump_char();
                return UNKNOWN;
            }
            if &start[..length] == b"proc" {
                return KW_PROC;
            }
            WORD
        }
    }
}
