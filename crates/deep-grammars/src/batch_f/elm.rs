use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, WHITESPACE};

rust_graph_deep::grammar! { ELM {
    name = "elm";
    lex = lex_elm;

    tokens {
        TEXT = "text",
    }

    program: TEXT*,
}}

pub fn lex_elm(c: &mut Cursor) -> Kind {
    let b = c.peek();
    if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
        c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'));
        return WHITESPACE;
    }
    if b == b'-' && c.peek_at(1) == b'-' {
        c.bump();
        c.bump();
        while !c.at_end() && c.peek() != b'\n' {
            c.bump();
        }
        return COMMENT;
    }
    c.bump();
    while !c.at_end() && !matches!(c.peek(), b' ' | b'\t' | b'\r' | b'\n') {
        if c.peek() == b'-' && c.peek_at(1) == b'-' {
            break;
        }
        c.bump();
    }
    TEXT
}
