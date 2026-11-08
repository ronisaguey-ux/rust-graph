//! YAML: block mappings and sequences, flow collections, scalars, anchors and
//! aliases, with indentation handed to the runtime lexer.
//!
//! This is the second layout language of the crate (Python is the first): the
//! runtime infers `NEWLINE`, `INDENT` and `DEDENT` from the columns, and the
//! rules consume them like any other token.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Block scalars.** `|` and `>` would need the lexer to swallow every line
//!   more indented than the key, and the lexer has no column stack of its own.
//!   A `|` is a `bar` token with no rule, so such a file is an error, not a
//!   tree with the scalar lost.
//! - **Sequence items are their own block.** In `- name: a` followed by
//!   `  count: 1`, the continuation key is indented past the dash, so the
//!   runtime opens a block there and `count` becomes a `block` of the item.
//!   YAML reads both keys as one mapping; this grammar nests the second, which
//!   keeps every key and value in the tree with a different shape.
//! - **Multiple documents.** One `---` at the start and one `...` at the end are
//!   modelled; a second document is an error.
//! - **Tags (`!!str`), merge keys (`<<:`), complex keys (`? ...`)** and
//!   multi-line plain scalars are not modelled.
//! - **Colon handling.** `:` is a key separator only when a space, a newline or
//!   the end follows it, which is YAML's own rule; `a:b` is one scalar.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { YAML {
    name = "yaml";
    lex = lex_yaml;
    layout = NEWLINE INDENT DEDENT;

    tokens {
        NEWLINE = "newline",
        INDENT = "indent" open block,
        DEDENT = "dedent" close block,
        COLON = ":",
        DASH = "-",
        COMMA = ",",
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        STRING = "string",
        BAD_STRING = "bad_string" bad,
        NUMBER = "number",
        TRUE = "true",
        FALSE = "false",
        NULL = "null",
        WORD = "word",
        ANCHOR = "anchor",
        ALIAS = "alias",
        BAR = "bar",
        DOC_START = "---",
        DOC_END = "...",
    }

    stream: ["---" NEWLINE] _entry* ["..." NEWLINE],

    _entry: key_entry | sequence_entry,

    key_entry: _key ":" [flow_value] NEWLINE [block],
    sequence_entry: "-" [sequence_item] NEWLINE [block],

    /// A deeper run of entries: the runtime's `indent`/`dedent` pair.
    block: INDENT _entry+ DEDENT,

    sequence_item: flow_value | inline_pair,
    inline_pair: _key ":" [flow_value],


    flow_value: flow_sequence | flow_mapping | scalar,
    flow_sequence: "[" [flow_item ("," flow_item)*] "]",
    flow_mapping: "{" [flow_pair ("," flow_pair)*] "}",
    flow_pair: _key ":" [flow_value],
    flow_item: flow_value,

    scalar: STRING | NUMBER | "true" | "false" | "null" | WORD | ANCHOR | ALIAS,

    _key: WORD | STRING | NUMBER,
} }

pub fn lex_yaml(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r'));
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
        b'\'' => {
            c.bump();
            loop {
                if c.at_end() {
                    return BAD_STRING;
                }
                match c.peek() {
                    b'\'' if c.peek_at(1) == b'\'' => c.bump_n(2),
                    b'\'' => {
                        c.bump();
                        return STRING;
                    }
                    b'\n' => return BAD_STRING,
                    _ => c.bump_char(),
                }
            }
        }
        b'"' => {
            if lex::quoted(c, b'"') {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'&' => {
            c.bump();
            c.eat_while(lex::is_word_byte);
            ANCHOR
        }
        b'*' => {
            c.bump();
            c.eat_while(lex::is_word_byte);
            ALIAS
        }
        b'[' => {
            c.bump();
            LBRACKET
        }
        b']' => {
            c.bump();
            RBRACKET
        }
        b'{' => {
            c.bump();
            LBRACE
        }
        b'}' => {
            c.bump();
            RBRACE
        }
        b',' => {
            c.bump();
            COMMA
        }
        b'|' | b'>' => {
            c.bump();
            BAR
        }
        b'-' if c.peek_at(1) == b'-' && c.peek_at(2) == b'-' && c.peek_at(3) != b'-' => {
            c.bump_n(3);
            DOC_START
        }
        b'-' if matches!(c.peek_at(1), b' ' | b'\n' | b'\r' | 0) => {
            c.bump();
            DASH
        }
        b'.' if c.peek_at(1) == b'.' && c.peek_at(2) == b'.' => {
            c.bump_n(3);
            DOC_END
        }
        b':' if matches!(c.peek_at(1), b' ' | b'\n' | b'\r' | 0) => {
            c.bump();
            COLON
        }
        b'0'..=b'9' => plain(c),
        _ => plain(c),
    }
}

/// Whether a plain scalar is a number: `12`, `-1.5`, `1e9`, `0x1f`.
fn numeric(text: &[u8]) -> bool {
    let digits = text.strip_prefix(b"-").unwrap_or(text);
    !digits.is_empty()
        && digits.first().is_some_and(u8::is_ascii_digit)
        && digits.iter().all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'e' | b'E' | b'+' | b'-' | b'x' | b'a'..=b'f' | b'A'..=b'F'))
}

/// A plain scalar: everything up to the next structural character.
fn plain(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let mut length = 0;
    while length < start.len() {
        let byte = start[length];
        if matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b',' | b'[' | b']' | b'{' | b'}' | b'#' | b'"' | b'\'' | b'&' | b'*' | b'|' | b'>') {
            break;
        }
        if byte == b':' && matches!(start.get(length + 1), None | Some(b' ' | b'\t' | b'\n' | b'\r')) {
            break;
        }
        length += 1;
    }
    c.bump_n(length);
    let text = &start[..length];
    if text.is_empty() {
        c.bump_char();
        return crate::deep::syntax::UNKNOWN;
    }
    if numeric(text) {
        return NUMBER;
    }
    match text {
        b"true" | b"True" | b"TRUE" => TRUE,
        b"false" | b"False" | b"FALSE" => FALSE,
        b"null" | b"Null" | b"NULL" | b"~" => NULL,
        _ => WORD,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lexes_the_document_marker() {
        use crate::deep::lexer::{lex, Tokens};
        let src = b"---\na: 1\n";
        let mut tokens = Tokens::default();
        lex(src, &super::YAML, &mut tokens);
        let names: Vec<String> = (0..tokens.len())
            .map(|index| format!("{}={:?}", super::YAML.name(tokens.kind[index]), String::from_utf8_lossy(tokens.text(src, index))))
            .collect();
        println!("{:?}", names);
    }
}
