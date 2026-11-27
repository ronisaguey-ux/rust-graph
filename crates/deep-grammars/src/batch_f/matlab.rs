use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, WHITESPACE};

rust_graph_deep::grammar! { MATLAB {
    name = "matlab";
    lex = lex_matlab;

    tokens {
        TEXT = "text",
    }

    program: TEXT*,
}}

pub fn lex_matlab(c: &mut Cursor) -> Kind {
    let b = c.peek();
    if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
        c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'));
        return WHITESPACE;
    }
    if b == b'%' {
        c.bump();
        while !c.at_end() && c.peek() != b'\n' {
            c.bump();
        }
        return COMMENT;
    }
    c.bump();
    while !c.at_end() && !matches!(c.peek(), b' ' | b'\t' | b'\r' | b'\n' | b'%') {
        c.bump();
    }
    TEXT
}
