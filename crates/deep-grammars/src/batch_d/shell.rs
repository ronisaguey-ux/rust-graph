//! POSIX shell and bash: commands, pipelines, and/or lists, redirections,
//! assignments, functions, `if`, `for` and `while`/`until`.
//!
//! The lexer decides what is a reserved word from the token before it, because
//! shell reserved words are only reserved at the start of a command: `echo if`
//! is two words, `if x; then` is a clause. A word is a keyword when it follows
//! a command boundary (start of file, newline, `;`, `&&`, `||`, `|`, `&`, `(`,
//! `{`, `then`, `do`, `else`, `elif`), which is where bash recognises one too.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **`in` is not a reserved word.** It is only reserved after the `for`/`case`
//!   word, and the lexer cannot see two tokens back, so it is lexed as a word.
//!   `for x in a b` therefore models its word list with `in` inside it; the loop
//!   variable and every word are still nodes of the tree.
//! - **Here-documents.** `<<EOF` is one redirect token and the body would be
//!   read as commands, so a heredoc is not modelled. So are process
//!   substitution's internals: `$( ... )`, `${ ... }` and `$(( ... ))` are one
//!   `variable` token scanned with brackets balanced, not code.
//! - **`[[ ... ]]` test expressions** are a command word list here; the pattern
//!   operators inside are not modelled.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { SHELL {
    name = "shell";
    lex = lex_sh;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        SEMI = ";",
        NEWLINE = "newline",
        PIPE = "|",
        OR = "||",
        AND = "&&",
        AMP = "&",
        BANG = "!",
        /// `>file`, `2>&1`, `<<EOF`: one token, the whole redirection.
        REDIRECT = "redirect",
        /// `$name`, `${...}`, `$(...)`, `$1`: one token.
        VARIABLE = "variable",
        /// A quoted string, single, double or `$'...'`.
        STRING = "string",
        /// A word with no closing quote.
        BAD_STRING = "bad_string" bad,
        /// `name=value` at the start of a word.
        ASSIGNMENT = "assignment",
        WORD = "word",
        /// A word directly followed by `()` at the start of a command: a
        /// function's name, which the shell's grammar does not read as an
        /// argument.
        FUNCTION_NAME = "function_name",

        KW_IF = "if",
        KW_THEN = "then",
        KW_ELIF = "elif",
        KW_ELSE = "else",
        KW_FI = "fi",
        KW_FOR = "for",
        KW_IN = "in",
        KW_WHILE = "while",
        KW_UNTIL = "until",
        KW_DO = "do",
        KW_DONE = "done",
        KW_FUNCTION = "function",
    }

    program: _body,

    _command: (_and_or | _compound) _terminator*,
    _terminator: ";" | NEWLINE | "&",
    /// A body: leading line breaks, then commands, each with its own trailing
    /// separators.
    _body: _terminator* _command*,

    _compound:
        if_clause
        | for_clause
        | while_clause
        | until_clause
        | function_definition
        | paren_function
        | brace_group
        | subshell,

    if_clause: "if" _body "then" _body elif_clause* [else_clause] "fi",
    elif_clause: "elif" _body,
    else_clause: "else" _body,

    for_clause: "for" _name ["in"] _word* _terminator* "do" _body "done",
    while_clause: "while" _body "do" _body "done",
    until_clause: "until" _body "do" _body "done",

    function_definition: "function" _name [empty_parens] _compound,
    paren_function: FUNCTION_NAME empty_parens _compound,
    empty_parens: "(" ")",

    brace_group: "{" _body "}",
    subshell: "(" _body ")",

    _and_or: _pipeline (("&&" | "||") _operand)*,
    _operand: _pipeline | _compound,
    _pipeline: ["!"] simple_command _pipe_next*,
    _pipe_next: "|" simple_command,
    simple_command: _simple_item+,

    _simple_item: WORD | STRING | VARIABLE | ASSIGNMENT | REDIRECT | "in",

    _name: WORD,
    _word: WORD | STRING | VARIABLE | ASSIGNMENT,
} }

/// The tokens after which a word is in command position, and so a keyword.
fn command_start(previous: Kind) -> bool {
    matches!(
        previous,
        crate::deep::syntax::UNKNOWN | NEWLINE | SEMI | PIPE | OR | AND | AMP | LPAREN | LBRACE | KW_THEN | KW_DO | KW_ELSE | KW_ELIF
    )
}

pub fn lex_sh(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\x0c'));
            WHITESPACE
        }
        b'\\' if c.peek_at(1) == b'\n' => {
            c.bump_n(2);
            WHITESPACE
        }
        b'\n' => {
            c.bump();
            NEWLINE
        }
        b'#' => {
            lex::line_comment(c);
            COMMENT
        }
        b'\'' | b'"' => {
            if lex::quoted(c, byte) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'$' if c.peek_at(1) == b'\'' => {
            c.bump();
            if lex::quoted(c, b'\'') {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'$' if c.peek_at(1) == b'"' => {
            c.bump();
            if lex::quoted(c, b'"') {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'$' => variable(c),
        b'(' => {
            c.bump();
            LPAREN
        }
        b')' => {
            c.bump();
            RPAREN
        }
        b'{' => {
            c.bump();
            LBRACE
        }
        b'}' => {
            c.bump();
            RBRACE
        }
        b';' => {
            c.bump();
            SEMI
        }
        b'|' if c.peek_at(1) == b'|' => {
            c.bump_n(2);
            OR
        }
        b'|' => {
            c.bump();
            PIPE
        }
        b'&' if c.peek_at(1) == b'&' => {
            c.bump_n(2);
            AND
        }
        b'&' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            redirect_target(c);
            REDIRECT
        }
        b'&' => {
            c.bump();
            AMP
        }
        b'!' if !c.peek_at(1).is_ascii_alphanumeric() && c.peek_at(1) != b'=' => {
            c.bump();
            BANG
        }
        b'>' | b'<' => {
            c.bump();
            if c.peek() == byte {
                c.bump();
            }
            if c.peek() == b'&' {
                c.bump();
            }
            redirect_target(c);
            REDIRECT
        }
        b'0'..=b'9'
            if matches!(c.peek_at(1), b'>' | b'<')
                || (c.peek_at(1).is_ascii_digit() && matches!(c.peek_at(2), b'>' | b'<')) =>
        {
            c.eat_while(|byte| byte.is_ascii_digit());
            c.bump();
            if c.peek() == b'>' && c.behind(1) == b'>' {
                c.bump();
            }
            if c.peek() == b'&' {
                c.bump();
            }
            redirect_target(c);
            REDIRECT
        }
        _ => word(c),
    }
}

/// The target of a redirection: a word, or an `&` and a file descriptor.
fn redirect_target(c: &mut Cursor) {
    if c.peek() == b'&' {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'-');
        return;
    }
    c.eat_while(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>'));
}

/// A `$` form: `$name`, `$1`, `$@`, `${...}` or `$(...)`, with brackets
/// balanced inside the last two.
fn variable(c: &mut Cursor) -> Kind {
    c.bump();
    match c.peek() {
        b'{' => balanced(c, b'{', b'}'),
        b'(' => balanced(c, b'(', b')'),
        _ => {
            c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@' | b'*' | b'#' | b'?' | b'$' | b'!' | b'-'));
        }
    }
    VARIABLE
}

/// From an opening bracket through its match, skipping quoted strings.
fn balanced(c: &mut Cursor, open: u8, close: u8) {
    let mut depth = 0u32;
    while !c.at_end() {
        match c.peek() {
            byte if byte == open => {
                depth += 1;
                c.bump();
            }
            byte if byte == close => {
                c.bump();
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            b'\'' | b'"' => {
                lex::quoted(c, c.peek());
            }
            b'\\' => c.bump_n(2),
            _ => c.bump_char(),
        }
    }
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>' | b'\'' | b'"' | b'#' | b'$'));
    let text = &start[..length];

    // `name=value` is one assignment word. The value may hold quotes and
    // expansions, so it continues to the end of the word.
    if let Some(eq) = text.iter().position(|&byte| byte == b'=') {
        let name = &text[..eq];
        if !name.is_empty() && name.iter().all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_') && !name[0].is_ascii_digit() {
            loop {
                match c.peek() {
                    b'\'' | b'"' => {
                        lex::quoted(c, c.peek());
                    }
                    b'$' => {
                        c.bump();
                        match c.peek() {
                            b'{' => balanced(c, b'{', b'}'),
                            b'(' => balanced(c, b'(', b')'),
                            _ => {
                                c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@' | b'*' | b'#' | b'?' | b'$' | b'!'));
                            }
                        }
                    }
                    b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>' | 0 => return ASSIGNMENT,
                    _ => c.bump_char(),
                }
            }
        }
    }

    if text.is_empty() {
        // A character the word scan stops on with nothing taken: one byte, so
        // the lexer always advances.
        c.bump_char();
        return WORD;
    }

    if command_start(c.previous()) && c.peek() == b'(' && c.peek_at(1) == b')' {
        return FUNCTION_NAME;
    }

    if command_start(c.previous()) {
        match text {
            b"if" => return KW_IF,
            b"then" => return KW_THEN,
            b"elif" => return KW_ELIF,
            b"else" => return KW_ELSE,
            b"fi" => return KW_FI,
            b"for" => return KW_FOR,
            b"while" => return KW_WHILE,
            b"until" => return KW_UNTIL,
            b"do" => return KW_DO,
            b"done" => return KW_DONE,
            b"function" => return KW_FUNCTION,
            _ => {}
        }
    }
    // `in` after the loop or case word, which is all the context there is.
    if text == b"in" && c.previous() == WORD {
        return KW_IN;
    }
    WORD
}
