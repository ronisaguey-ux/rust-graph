//! Hand-written scanners: the whole input is tokenised before the parser runs,
//! so a version never has to rewind a lexer.

#![allow(dead_code)]

use deep_rt::{Lexer, StateId, Token};

use super::{BAD, COLON, DEF, EQ, LBRACE, LP, NAME, NEST_LP, NEST_NUM, NEST_RP, NUM, RBRACE, RP, SEMI};

pub struct SliceLexer {
    tokens: Vec<Token>,
    at: usize,
}

impl SliceLexer {
    pub fn new(tokens: Vec<Token>) -> SliceLexer {
        SliceLexer { tokens, at: 0 }
    }
}

impl Lexer for SliceLexer {
    fn next_token(&mut self, _state: StateId) -> Option<Token> {
        let token = self.tokens.get(self.at).copied();
        if token.is_some() {
            self.at += 1;
        }
        token
    }
}

/// The mini language's hand-written scanner.
pub fn lex_mini(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        let start = at;
        let symbol = match byte {
            b'(' => {
                at += 1;
                LP
            }
            b')' => {
                at += 1;
                RP
            }
            b'{' => {
                at += 1;
                LBRACE
            }
            b'}' => {
                at += 1;
                RBRACE
            }
            b'=' => {
                at += 1;
                EQ
            }
            b';' => {
                at += 1;
                SEMI
            }
            b':' => {
                at += 1;
                COLON
            }
            b'0'..=b'9' => {
                while at < bytes.len() && bytes[at].is_ascii_digit() {
                    at += 1;
                }
                NUM
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while at < bytes.len()
                    && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_')
                {
                    at += 1;
                }
                if &source[start..at] == "def" {
                    DEF
                } else {
                    NAME
                }
            }
            _ => {
                at += 1;
                BAD
            }
        };
        out.push(Token {
            symbol,
            start: start as u32,
            end: at as u32,
        });
    }
    out
}

/// The nesting language's scanner: `(` `)` and runs of digits, with `?` as the
/// deliberately invalid token.
pub fn lex_nest(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        let start = at;
        let symbol = match byte {
            b'(' => {
                at += 1;
                NEST_LP
            }
            b')' => {
                at += 1;
                NEST_RP
            }
            b'0'..=b'9' => {
                while at < bytes.len() && bytes[at].is_ascii_digit() {
                    at += 1;
                }
                NEST_NUM
            }
            _ => {
                at += 1;
                BAD
            }
        };
        out.push(Token {
            symbol,
            start: start as u32,
            end: at as u32,
        });
    }
    out
}
