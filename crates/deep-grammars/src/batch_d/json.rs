//! JSON: objects, arrays, strings, numbers and the three literals.
//!
//! What is missing on purpose, and named here rather than silently accepted:
//! comments and trailing commas are extensions, not JSON, so they are errors.
//! A duplicate key is a tree with two `pair` nodes: JSON has no rule against it,
//! and a scanner may care.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;

rust_graph_deep::grammar! { JSON {
    name = "json";
    lex = lex_json;

    tokens {
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        COMMA = ",",
        COLON = ":",
        STRING = "string",
        /// A string with no closing quote.
        BAD_STRING = "bad_string" bad,
        NUMBER = "number",
        TRUE = "true",
        FALSE = "false",
        NULL = "null",
    }

    document: _value,

    _value: object | array | STRING | NUMBER | "true" | "false" | "null",

    object: "{" (_pair %? ",") "}",
    _pair: STRING ":" _value,

    array: "[" (_value %? ",") "]",
} }

pub fn lex_json(c: &mut Cursor) -> crate::deep::syntax::Kind {
    use crate::deep::syntax::{UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'));
            WHITESPACE
        }
        0xef if c.pos() == 0 && c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'"' => {
            if lex::quoted(c, b'"') {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'-' | b'0'..=b'9' => {
            c.eat(b'-');
            lex::number(c);
            NUMBER
        }
        b'{' => {
            c.bump();
            LBRACE
        }
        b'}' => {
            c.bump();
            RBRACE
        }
        b'[' => {
            c.bump();
            LBRACKET
        }
        b']' => {
            c.bump();
            RBRACKET
        }
        b',' => {
            c.bump();
            COMMA
        }
        b':' => {
            c.bump();
            COLON
        }
        b't' | b'f' | b'n' => {
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
            match &start[..length] {
                b"true" => TRUE,
                b"false" => FALSE,
                b"null" => NULL,
                _ => UNKNOWN,
            }
        }
        _ => UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lexes_the_punctuation() {
        use crate::deep::lexer::{lex, Tokens};
        let src = br#"{"a": 1, "b": [2]}"#;
        let mut tokens = Tokens::default();
        lex(src, &super::JSON, &mut tokens);
        let names: Vec<String> = (0..tokens.len()).map(|index| format!("{}={:?}", super::JSON.name(tokens.kind[index]), String::from_utf8_lossy(tokens.text(src, index)))).collect();
        println!("{:?}", names);
        
    }
}
