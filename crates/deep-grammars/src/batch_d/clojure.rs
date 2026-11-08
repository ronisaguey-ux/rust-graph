//! Clojure: forms, the special forms a scanner needs to see, and the literal
//! shapes (lists, vectors, maps, atoms, quoted forms).
//!
//! Clojure is data: `(f x)` is a list whose first element happens to be a
//! symbol. A scanner still wants `defn`, `def`, `defmacro` and `ns` as
//! definitions, so those names are keyword tokens and a list that starts with
//! one is a `definition_form` or `ns_form` rather than an anonymous `list`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A special form used as data is an error.** `(quote defn)` does not parse,
//!   because `defn` is a token only a definition may start with.
//! - **Reader macros** `#'`, `@`, `^`, `~`, `` ` `` are not modelled; `'` (quote)
//!   and `#` (a dispatch prefix kept with its form) are.
//! - **Sets and anonymous function literals** (`#{...}`, `#(...)`) are not
//!   modelled: a set's `#{` is one `hash_prefix` token followed by a map, which
//!   is not the same tree.
//! - **Metadata maps and reader conditionals** are not modelled.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { CLOJURE {
    name = "clojure";
    lex = lex_clojure;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        QUOTE = "'",
        TILDE = "~",
        TILDE_AT = "~@",
        AT = "@",
        BACKTICK = "`",
        HASH_PREFIX = "#",
        SET_START = "#{",
        SYMBOL = "symbol",
        KEYWORD = "keyword",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,
        CHAR = "char_literal",

        KW_NS = "ns",
        KW_DEF = "def",
        KW_DEFN = "defn",
        KW_DEFN_PRIVATE = "defn-",
        KW_DEFMACRO = "defmacro",
        KW_NIL = "nil",
        KW_TRUE = "true",
        KW_FALSE = "false",
    }

    program: _form*,

    _form: ns_form | definition_form | quoted_form | hash_form | reader_form | list | vector | map | atom,

    ns_form: "(" "ns" SYMBOL _form* ")",
    definition_form: "(" definition_head SYMBOL _form* ")",
    definition_head: "def" | "defn" | "defn-" | "defmacro",

    quoted_form: "'" _form,
    hash_form: "#" _form,
    reader_form: reader_prefix _form,
    reader_prefix: "~" | "~@" | "@" | "`",

    list: "(" _form* ")",
    vector: "[" _form* "]",
    map: "{" _form* "}",

    atom: SYMBOL | KEYWORD | NUMBER | STRING | CHAR | "nil" | "true" | "false",
} }

pub fn lex_clojure(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b',' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b',' | b'\x0c'));
            WHITESPACE
        }
        b';' => { lex::line_comment(c); COMMENT }
        b'"' => if lex::quoted(c, b'"') { STRING } else { BAD_STRING },
        b'\\' => {
            c.bump();
            let start = c.rest();
            let length = c.eat_while(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'"' | b';'));
            let _ = start;
            if length == 0 { BAD_STRING } else { CHAR }
        }
        b'#' if c.peek_at(1) == b'{' => { c.bump_n(2); SET_START }
        b'#' => { c.bump(); HASH_PREFIX }
        b'\'' => { c.bump(); QUOTE }
        b'~' if c.peek_at(1) == b'@' => { c.bump_n(2); TILDE_AT }
        b'~' => { c.bump(); TILDE }
        b'@' => { c.bump(); AT }
        b'`' => { c.bump(); BACKTICK }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b':' => {
            c.bump();
            c.eat_while(symbol_byte);
            KEYWORD
        }
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'-' if c.peek_at(1).is_ascii_digit() => { c.bump(); lex::number(c); NUMBER }
        b'+' if c.peek_at(1).is_ascii_digit() => { c.bump(); lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'*' | b'!' | b'?' | b'<' | b'>' | b'=' | b'/' | b'&' | b'%' | b'|' | b'+' | b'-' | b'.' => {
            let start = c.rest();
            let length = c.eat_while(symbol_byte);
            let text = &start[..length];
            match text {
                b"ns" => KW_NS,
                b"def" => KW_DEF,
                b"defn" => KW_DEFN,
                b"defn-" => KW_DEFN_PRIVATE,
                b"defmacro" => KW_DEFMACRO,
                b"nil" => KW_NIL,
                b"true" => KW_TRUE,
                b"false" => KW_FALSE,
                _ => SYMBOL,
            }
        }
        _ => UNKNOWN,
    }
}

fn symbol_byte(byte: u8) -> bool {
    !byte.is_ascii_whitespace()
        && !matches!(byte, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'"' | b';' | b'\'' | b'`' | b'~' | b'@' | b'^' | b',')
}
