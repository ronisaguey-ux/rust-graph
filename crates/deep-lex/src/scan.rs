//! Token production for the ordinary (non-layout) cases: identifiers, numbers,
//! string literals and template literals. Everything here is offsets and byte
//! scans; nothing allocates.

use crate::cursor::{is_ident_continue, is_ident_start, is_line_terminator};
use crate::kind::OPERATORS;
use crate::mode::Mode;
use crate::python;
use crate::{string_prefix, DeepLexer, Kind, Language, Span, Token};

impl<'a, S: crate::ExternalScanner> DeepLexer<'a, S> {
    pub(crate) fn lex_code(&mut self) -> Token {
        let start = self.cur.pos();
        let b = match self.cur.peek() {
            Some(b) => b,
            None => return self.layout_token_at(Kind::Error, start),
        };

        // String literals, with the optional Python/C prefix included in the span.
        if let Some((prefix, quote, triple, raw)) = string_prefix(&self.cur, self.lang) {
            self.cur.advance(prefix);
            return self.lex_string(start, quote, triple, raw);
        }

        // JavaScript template literal: the only construct that needs a real,
        // multi-token mode stack.
        if b == b'`' && self.lang == Language::JavaScript {
            self.cur.bump();
            if self.modes.push(Mode::Template, 0) {
                return self.lex_template(start);
            }
            return self.emit(Kind::Error, Span::new(start, self.cur.pos()));
        }

        // Template substitution: `}` closes the expression only at depth 0.
        if self.modes.top() == Mode::TemplateExpr {
            if b == b'{' {
                self.modes.set_top_depth(self.modes.top_depth().saturating_add(1));
            } else if b == b'}' {
                let depth = self.modes.top_depth();
                if depth == 0 {
                    self.cur.bump();
                    self.modes.pop();
                    // The frame below is the Template we return to; its depth
                    // records that the next chunk follows a substitution.
                    self.modes.set_top_depth(1);
                    return self.emit(Kind::RBrace, Span::new(start, self.cur.pos()));
                }
                self.modes.set_top_depth(depth - 1);
            }
        }

        // JSON's `-` is part of the number, not an operator: `-12` is one token.
        if b == b'-'
            && self.lang.is_json()
            && self.cur.peek_n(1).map_or(false, |d| d.is_ascii_digit())
        {
            self.cur.bump();
            return self.lex_number(start);
        }

        if b.is_ascii_digit() || (b == b'.' && self.cur.peek_n(1).map_or(false, |d| d.is_ascii_digit()))
        {
            return self.lex_number(start);
        }

        if b < 0x80 {
            if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
                return self.lex_ident(start);
            }
        } else if let Some((c, len)) = self.cur.decode() {
            if is_ident_start(c) {
                self.cur.advance(len);
                return self.lex_ident(start);
            }
        }

        for &(bytes, kind) in OPERATORS {
            if self.cur.starts_with(bytes) {
                self.cur.advance(bytes.len());
                return self.emit(kind, Span::new(start, self.cur.pos()));
            }
        }

        // Unknown byte: one Error token, so the parser can skip exactly it.
        self.cur.bump();
        self.emit(Kind::Error, Span::new(start, self.cur.pos()))
    }

    pub(crate) fn lex_ident(&mut self, start: u32) -> Token {
        loop {
            match self.cur.peek() {
                None => break,
                Some(b) if b < 0x80 => {
                    if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
                        self.cur.bump();
                    } else {
                        break;
                    }
                }
                Some(_) => match self.cur.decode() {
                    Some((c, len)) if is_ident_continue(c) => self.cur.advance(len),
                    _ => break,
                },
            }
        }
        self.emit(Kind::Ident, Span::new(start, self.cur.pos()))
    }

    pub(crate) fn lex_number(&mut self, start: u32) -> Token {
        let radix = if self.cur.peek() == Some(b'0') {
            match self.cur.peek_n(1) {
                Some(b'x') | Some(b'X') => 16,
                Some(b'b') | Some(b'B') => 2,
                Some(b'o') | Some(b'O') => 8,
                _ => 10,
            }
        } else {
            10
        };
        if radix != 10 {
            self.cur.advance(2);
            while let Some(b) = self.cur.peek() {
                if b.is_ascii_hexdigit() || b == b'_' {
                    self.cur.bump();
                } else {
                    break;
                }
            }
            return self.emit(Kind::Number, Span::new(start, self.cur.pos()));
        }
        self.cur.skip_while(|b| b.is_ascii_digit() || b == b'_');
        if self.cur.peek() == Some(b'.') && self.cur.peek_n(1) != Some(b'.') {
            self.cur.bump();
            self.cur.skip_while(|b| b.is_ascii_digit() || b == b'_');
        }
        if matches!(self.cur.peek(), Some(b'e') | Some(b'E')) {
            let next = self.cur.peek_n(1);
            let signed = matches!(next, Some(b'+') | Some(b'-'))
                && self.cur.peek_n(2).map_or(false, |d| d.is_ascii_digit());
            if next.map_or(false, |d| d.is_ascii_digit()) || signed {
                self.cur.bump();
                if signed {
                    self.cur.bump();
                }
                self.cur.skip_while(|b| b.is_ascii_digit() || b == b'_');
            }
        }
        self.emit(Kind::Number, Span::new(start, self.cur.pos()))
    }

    pub(crate) fn lex_string(&mut self, start: u32, quote: u8, triple: bool, raw: bool) -> Token {
        // The mode is real even though the literal is emitted as one token: it
        // is what makes the escape and triple-quote rules reachable, and it is
        // popped again however the literal ends — including at EOF.
        let pushed = self.modes.push(Mode::String, if triple { 3 } else { 1 });
        self.cur.advance(if triple { 3 } else { 1 });
        loop {
            match self.cur.peek() {
                None => break,
                Some(b'\\') => {
                    self.cur.bump();
                    if self.cur.peek().is_some() {
                        self.cur.bump();
                    }
                }
                Some(b) if b == quote => {
                    if triple {
                        if self.cur.peek_n(1) == Some(quote) && self.cur.peek_n(2) == Some(quote) {
                            self.cur.advance(3);
                            break;
                        }
                        self.cur.bump();
                    } else {
                        self.cur.bump();
                        break;
                    }
                }
                Some(b) if is_line_terminator(b) => {
                    if triple {
                        python::eat_terminator(&mut self.cur);
                    } else {
                        // Unterminated: end the token before the line break and
                        // let the parser see the string plus a NEWLINE.
                        break;
                    }
                }
                Some(_) => {
                    self.cur.bump();
                }
            }
        }
        if pushed {
            self.modes.pop();
        }
        let kind = if raw { Kind::RawString } else { Kind::String };
        self.emit(kind, Span::new(start, self.cur.pos()))
    }

    /// `span_start` covers the opening backtick when the template just opened;
    /// after a substitution it is the byte just past the closing `}`.
    pub(crate) fn lex_template(&mut self, span_start: u32) -> Token {
        let first = self.modes.top_depth() == 0;
        loop {
            match self.cur.peek() {
                None => {
                    let end = self.cur.pos();
                    self.modes.pop();
                    return self.emit(Kind::TemplateTail, Span::new(span_start, end));
                }
                Some(b'`') => {
                    self.cur.bump();
                    let end = self.cur.pos();
                    self.modes.pop();
                    return self.emit(Kind::TemplateTail, Span::new(span_start, end));
                }
                Some(b'\\') => {
                    self.cur.bump();
                    if self.cur.peek().is_some() {
                        self.cur.bump();
                    }
                }
                Some(b'$') if self.cur.peek_n(1) == Some(b'{') => {
                    self.cur.advance(2);
                    let end = self.cur.pos();
                    let kind = if first {
                        Kind::TemplateHead
                    } else {
                        Kind::TemplateMiddle
                    };
                    self.modes.push(Mode::TemplateExpr, 0);
                    return self.emit(kind, Span::new(span_start, end));
                }
                Some(_) => {
                    self.cur.bump();
                }
            }
        }
    }
}
