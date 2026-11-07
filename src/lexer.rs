//! A pull lexer driven by the language table. It walks the bytes once and
//! hands out tokens; nothing downstream looks at a byte of source twice,
//! except the inside of an interpolated string, which is lexed when reached.

use crate::langs::{Imports, Interp, Language};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Ident,
    Str,
    Num,
    Op,
    Newline,
    /// `#include` and friends: the whole line.
    Directive,
    /// A function body that has already been read into its own symbol.
    Lambda,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: Kind,
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
    /// Byte ranges of the expressions inside an interpolated string.
    pub interp: Option<Box<Vec<(u32, u32)>>>,
}

const OPS3: [&[u8]; 10] = [b"===", b"!==", b"...", b"**=", b"//=", b">>=", b"<<=", b"&&=", b"||=", b"??="];
const OPS2: [&[u8]; 28] = [
    b"==", b"!=", b"<=", b">=", b"=>", b"->", b"::", b":=", b"+=", b"-=", b"*=", b"/=", b"%=", b".=", b"|=", b"&=",
    b"^=", b"?.", b"&&", b"||", b"<<", b">>", b"**", b"//", b"=~", b"<-", b"++", b"--",
];
/// After these words a `/` opens a regex, not a division.
const BEFORE_REGEX: [&[u8]; 12] =
    [b"return", b"typeof", b"split", b"grep", b"map", b"if", b"unless", b"and", b"or", b"not", b"when", b"case"];

pub struct Lexer<'a> {
    pub src: &'a [u8],
    lang: &'static Language,
    pos: usize,
    line: u32,
    line_start: usize,
    /// The previous token could end an expression.
    after_value: bool,
    at_line_start: bool,
    heredocs: Vec<Vec<u8>>,
    in_html: bool,
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8], lang: &'static Language) -> Self {
        let mut lexer = Lexer {
            src,
            lang,
            pos: 0,
            line: 1,
            line_start: 0,
            after_value: false,
            at_line_start: true,
            heredocs: Vec::new(),
            in_html: lang.tag == "PHP",
        };
        if src.starts_with(b"#!") {
            lexer.skip_to_newline();
        }
        lexer
    }

    /// Lex a slice of an outer file, keeping its line number.
    pub fn nested(src: &'a [u8], lang: &'static Language, line: u32) -> Self {
        let mut lexer = Lexer::new(src, lang);
        lexer.line = line;
        lexer.in_html = false;
        lexer
    }

    fn peek(&self, offset: usize) -> u8 {
        self.src.get(self.pos + offset).copied().unwrap_or(0)
    }

    fn is_ident(&self, byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80 || self.lang.ident_extra.contains(&byte)
    }

    fn skip_to_newline(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
            self.pos += 1;
        }
    }

    fn newline(&mut self) {
        self.pos += 1;
        self.line += 1;
        self.line_start = self.pos;
        self.at_line_start = true;
    }

    fn skip_until(&mut self, close: &[u8]) {
        while self.pos < self.src.len() {
            if self.src[self.pos..].starts_with(close) {
                self.pos += close.len();
                return;
            }
            if self.src[self.pos] == b'\n' {
                self.newline();
            } else {
                self.pos += 1;
            }
        }
    }

    fn skip_html(&mut self) {
        while self.pos < self.src.len() {
            let rest = &self.src[self.pos..];
            if rest.starts_with(b"<?php") {
                self.pos += 5;
                self.in_html = false;
                return;
            }
            if rest.starts_with(b"<?=") {
                self.pos += 3;
                self.in_html = false;
                return;
            }
            if rest[0] == b'\n' {
                self.newline();
            } else {
                self.pos += 1;
            }
        }
    }

    /// Skips blanks and comments. True when a comment ran to a line end.
    fn skip_trivia(&mut self) {
        loop {
            if self.in_html {
                self.skip_html();
            }
            let byte = self.peek(0);
            if self.pos >= self.src.len() {
                return;
            }
            if byte == b' ' || byte == b'\t' || byte == b'\r' || byte == 0x0c {
                self.pos += 1;
                continue;
            }
            if byte == b'\\' && matches!(self.peek(1), b'\n' | b'\r') {
                self.pos += 1;
                if self.peek(0) == b'\r' {
                    self.pos += 1;
                }
                let at_start = self.at_line_start;
                self.newline();
                self.at_line_start = at_start;
                continue;
            }
            let rest = &self.src[self.pos..];
            if self.lang.tag == "PHP" && rest.starts_with(b"?>") {
                self.pos += 2;
                self.in_html = true;
                continue;
            }
            if let Some((open, close)) = self.lang.block_comments.iter().find(|pair| rest.starts_with(pair.0.as_bytes())) {
                let needs_line_start = open.starts_with('=');
                if !needs_line_start || self.pos == self.line_start {
                    self.pos += open.len();
                    self.skip_until(close.as_bytes());
                    self.at_line_start = false;
                    continue;
                }
            }
            if self.lang.line_comments.iter().any(|comment| rest.starts_with(comment.as_bytes())) {
                self.skip_to_newline();
                continue;
            }
            return;
        }
    }

    fn token(&mut self, kind: Kind, start: usize, line: u32, col: u32) -> Token {
        self.after_value = match kind {
            Kind::Ident | Kind::Num | Kind::Str => true,
            Kind::Op => matches!(self.src[self.pos - 1], b')' | b']' | b'}'),
            _ => false,
        };
        if kind == Kind::Ident && BEFORE_REGEX.contains(&&self.src[start..self.pos]) {
            self.after_value = false;
        }
        Token { kind, start: start as u32, end: self.pos as u32, line, col, interp: None }
    }

    fn read_heredoc_bodies(&mut self) {
        let terminators = std::mem::take(&mut self.heredocs);
        for terminator in terminators {
            while self.pos < self.src.len() {
                let start = self.pos;
                self.skip_to_newline();
                let line = &self.src[start..self.pos];
                let done = line.trim_ascii() == terminator.as_slice();
                if self.pos < self.src.len() {
                    self.newline();
                }
                if done {
                    break;
                }
            }
        }
    }

    /// `<<EOF`, `<<-EOF`, `<<~EOF`, `<<'EOF'`, `<<<EOF`. Leaves pos alone if not one.
    fn try_heredoc(&mut self) -> bool {
        let mut at = self.pos + 2;
        let bytes = self.src;
        if bytes.get(at) == Some(&b'<') {
            at += 1;
        }
        if matches!(bytes.get(at), Some(b'-' | b'~')) {
            at += 1;
        }
        let quote = matches!(bytes.get(at), Some(b'"' | b'\''));
        if quote {
            at += 1;
        }
        let start = at;
        while bytes.get(at).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
            at += 1;
        }
        let word = &bytes[start..at];
        if word.is_empty() || !word[0].is_ascii_uppercase() {
            return false;
        }
        self.heredocs.push(word.to_vec());
        self.pos = if quote { at + 1 } else { at };
        true
    }

    fn scan_interpolated(&mut self, close: &[u8], interp: Interp, multi_line: bool) -> Vec<(u32, u32)> {
        let mut parts = Vec::new();
        while self.pos < self.src.len() {
            let rest = &self.src[self.pos..];
            if rest.starts_with(close) {
                self.pos += close.len();
                return parts;
            }
            let byte = rest[0];
            match byte {
                b'\\' => {
                    if interp == Interp::Swift && self.peek(1) == b'(' {
                        self.pos += 2;
                        parts.push(self.balanced(b'(', b')'));
                        continue;
                    }
                    self.pos += 1;
                    if self.peek(0) == b'\n' {
                        self.newline();
                    } else {
                        self.pos += 1;
                    }
                }
                b'\n' => {
                    if !multi_line {
                        return parts;
                    }
                    self.newline();
                }
                b'{' if interp == Interp::PyFormat => {
                    if self.peek(1) == b'{' {
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        parts.push(self.balanced(b'{', b'}'));
                    }
                }
                b'$' if interp == Interp::Template && self.peek(1) == b'{' => {
                    self.pos += 2;
                    parts.push(self.balanced(b'{', b'}'));
                }
                b'#' if interp == Interp::Hash && self.peek(1) == b'{' => {
                    self.pos += 2;
                    parts.push(self.balanced(b'{', b'}'));
                }
                b'$' if interp == Interp::Dollar => {
                    if self.peek(1) == b'{' {
                        self.pos += 2;
                        parts.push(self.balanced(b'{', b'}'));
                    } else if is_ident_start(self.peek(1)) || self.peek(1).is_ascii_digit() {
                        self.pos += 1;
                        let start = self.pos;
                        while self.is_ident(self.peek(0)) {
                            self.pos += 1;
                        }
                        parts.push((start as u32, self.pos as u32));
                    } else {
                        self.pos += 1;
                    }
                }
                _ => self.pos += 1,
            }
        }
        parts
    }

    /// Past an opener: the range up to its partner, and pos after the partner.
    fn balanced(&mut self, open: u8, close: u8) -> (u32, u32) {
        let start = self.pos;
        let mut depth = 1;
        while self.pos < self.src.len() {
            let byte = self.src[self.pos];
            if byte == open {
                depth += 1;
            } else if byte == close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    return (start as u32, (self.pos - 1) as u32);
                }
            } else if byte == b'\n' {
                self.newline();
                continue;
            }
            self.pos += 1;
        }
        (start as u32, self.pos as u32)
    }

    fn string(&mut self, start: usize, line: u32, col: u32, formatted: bool) -> Token {
        let quote = self.peek(0);
        let triple = self.lang.triple_quotes && self.peek(1) == quote && self.peek(2) == quote;
        let closer = [quote; 3];
        let close: &[u8] = if triple { &closer } else { &closer[..1] };
        self.pos += close.len();
        let interp = match self.lang.interp {
            Interp::PyFormat if formatted => Interp::PyFormat,
            Interp::Template if quote == b'`' => Interp::Template,
            Interp::Hash | Interp::Dollar | Interp::Swift if quote == b'"' => self.lang.interp,
            _ => Interp::None,
        };
        let multi_line = triple || quote == b'`' || self.lang.sigils || self.lang.interp == Interp::Hash;
        let parts = self.scan_interpolated(close, interp, multi_line);
        let mut token = self.token(Kind::Str, start, line, col);
        if !parts.is_empty() {
            token.interp = Some(Box::new(parts));
        }
        token
    }

    /// A `/` where a value is expected, closed on the same line.
    fn try_regex(&mut self) -> bool {
        let mut at = self.pos + 1;
        let mut in_class = false;
        while let Some(&byte) = self.src.get(at) {
            match byte {
                b'\n' => return false,
                b'\\' => at += 1,
                b'[' => in_class = true,
                b']' => in_class = false,
                b'/' if !in_class => {
                    at += 1;
                    while self.src.get(at).is_some_and(|b| b.is_ascii_alphabetic()) {
                        at += 1;
                    }
                    self.pos = at;
                    return true;
                }
                _ => {}
            }
            at += 1;
        }
        false
    }

    /// Perl's `q(...)`, `qq{...}`, `m/.../`, `s/.../.../` and kin.
    fn try_perl_quote(&mut self, word: &[u8]) -> bool {
        let sections = match word {
            b"q" | b"qq" | b"qw" | b"qx" | b"qr" | b"m" => 1,
            b"s" | b"tr" | b"y" => 2,
            _ => return false,
        };
        let open = self.peek(0);
        if !matches!(open, b'(' | b'{' | b'[' | b'/' | b'|' | b'!' | b'#') || (word != b"m" && word != b"s" && open == b'/' && word.len() == 1) {
            return false;
        }
        let close = match open {
            b'(' => b')',
            b'{' => b'}',
            b'[' => b']',
            other => other,
        };
        for section in 0..sections {
            if section == 0 || open == close {
                self.pos += 1;
            } else {
                while self.peek(0).is_ascii_whitespace() {
                    self.pos += 1;
                }
                self.pos += 1;
            }
            if open == close {
                while self.pos < self.src.len() && self.src[self.pos] != close {
                    self.pos += if self.src[self.pos] == b'\\' { 2 } else { 1 };
                }
            } else {
                self.balanced(open, close);
                self.pos -= 1;
            }
            if open != close {
                self.pos += 1;
            }
        }
        if open == close {
            self.pos += 1;
        }
        while self.peek(0).is_ascii_alphabetic() {
            self.pos += 1;
        }
        true
    }

    pub fn next(&mut self) -> Token {
        self.skip_trivia();
        let start = self.pos;
        let line = self.line;
        let col = (start - self.line_start) as u32;
        if start >= self.src.len() {
            return Token { kind: Kind::Eof, start: start as u32, end: start as u32, line, col, interp: None };
        }
        let byte = self.src[start];
        let first_on_line = self.at_line_start;
        self.at_line_start = false;
        if byte == b'\n' {
            self.pos += 1;
            let token = self.token(Kind::Newline, start, line, col);
            self.pos -= 1;
            self.newline();
            self.after_value = false;
            if !self.heredocs.is_empty() {
                self.read_heredoc_bodies();
            }
            return token;
        }
        if byte == b'#' && first_on_line && self.lang.imports == Imports::Include {
            self.skip_to_newline();
            return self.token(Kind::Directive, start, line, col);
        }
        if self.lang.sigils && matches!(byte, b'$' | b'@') {
            let next = self.peek(1);
            if is_ident_start(next) || (byte == b'$' && next.is_ascii_digit()) {
                self.pos += 1;
                let name = self.pos;
                while self.is_ident(self.peek(0)) {
                    self.pos += 1;
                }
                return self.token(Kind::Ident, name, line, col);
            }
        }
        if is_ident_start(byte) {
            while self.is_ident(self.peek(0)) {
                self.pos += 1;
            }
            let word = &self.src[start..self.pos];
            let next = self.peek(0);
            if self.lang.tag == "PL" && self.try_perl_quote(word) {
                return self.token(Kind::Str, start, line, col);
            }
            if self.lang.quotes.contains(&next) && word.len() <= 2 && word.iter().all(|b| b"rRbBuUfF".contains(b)) {
                let formatted = word.iter().any(|b| matches!(b, b'f' | b'F'));
                return self.string(start, line, col, formatted);
            }
            if self.lang.tag == "RS" && word == b"r" && next == b'#' {
                let mut hashes = 0;
                while self.peek(hashes) == b'#' {
                    hashes += 1;
                }
                if self.peek(hashes) == b'"' {
                    self.pos += hashes + 1;
                    let mut close = vec![b'"'];
                    close.resize(hashes + 1, b'#');
                    self.skip_until(&close);
                    return self.token(Kind::Str, start, line, col);
                }
            }
            return self.token(Kind::Ident, start, line, col);
        }
        if byte.is_ascii_digit() {
            while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' || (self.peek(0) == b'.' && self.peek(1).is_ascii_digit()) {
                self.pos += 1;
            }
            return self.token(Kind::Num, start, line, col);
        }
        if self.lang.quotes.contains(&byte) {
            return self.string(start, line, col, false);
        }
        if self.lang.tag == "M" && byte == b'\'' && !self.after_value {
            self.pos += 1;
            self.scan_interpolated(b"'", Interp::None, false);
            return self.token(Kind::Str, start, line, col);
        }
        if byte == b'\'' {
            let width = if self.peek(1) == b'\\' { 3 } else { 2 };
            if self.peek(width) == b'\'' {
                self.pos += width + 1;
                return self.token(Kind::Str, start, line, col);
            }
        }
        if matches!(byte, b'$' | b'@') && self.lang.quotes.contains(&self.peek(1)) {
            self.pos += 1;
            return self.string(start, line, col, byte == b'$');
        }
        if self.lang.tag == "LUA" && byte == b'[' && self.peek(1) == b'[' {
            self.pos += 2;
            self.skip_until(b"]]");
            return self.token(Kind::Str, start, line, col);
        }
        if byte == b'/' && self.lang.regex_literals && !self.after_value && self.try_regex() {
            return self.token(Kind::Str, start, line, col);
        }
        if self.lang.heredocs && byte == b'<' && self.peek(1) == b'<' && self.try_heredoc() {
            return self.token(Kind::Str, start, line, col);
        }
        let rest = &self.src[start..];
        let width = if OPS3.iter().any(|op| rest.starts_with(op)) {
            3
        } else if OPS2.iter().any(|op| rest.starts_with(op)) {
            2
        } else {
            1
        };
        self.pos += width;
        self.token(Kind::Op, start, line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs::by_content;

    fn words(suffix: &str, body: &str) -> Vec<String> {
        let lang = by_content(suffix, body.as_bytes()).unwrap();
        let mut lexer = Lexer::new(body.as_bytes(), lang);
        let mut out = Vec::new();
        loop {
            let token = lexer.next();
            match token.kind {
                Kind::Eof => return out,
                Kind::Ident => out.push(String::from_utf8_lossy(&body.as_bytes()[token.start as usize..token.end as usize]).into_owned()),
                _ => {}
            }
        }
    }

    #[test]
    fn comments_and_strings_hold_no_identifiers() {
        assert_eq!(words("py", "a # b\n'c'\n\"\"\"d\ne\"\"\"\nf\n"), ["a", "f"]);
        assert_eq!(words("js", "a // b\n/* c */ 'd' e\n"), ["a", "e"]);
        assert_eq!(words("rb", "a\n=begin\nb\n=end\nc\n"), ["a", "c"]);
    }

    #[test]
    fn a_regex_is_not_division() {
        assert_eq!(words("js", "x = /a b/.exec(s)\n"), ["x", "exec", "s"]);
        assert_eq!(words("js", "x = a / b / c\n"), ["x", "a", "b", "c"]);
    }

    #[test]
    fn heredoc_bodies_are_data() {
        assert_eq!(words("rb", "x = <<~SQL\n  eval(y)\nSQL\nz\n"), ["x", "z"]);
    }

    #[test]
    fn php_outside_the_tags_is_html() {
        assert_eq!(words("php", "<b>eval</b><?php $a; ?>eval<?php $b;"), ["a", "b"]);
    }

    #[test]
    fn a_rust_char_with_a_quote_does_not_open_a_string() {
        assert_eq!(words("rs", "let c = '\"'; let d = 1;\n"), ["let", "c", "let", "d"]);
    }

    #[test]
    fn interpolation_ranges_are_recorded() {
        let body = b"f\"a {x} b\"";
        let lang = by_content("py", body).unwrap();
        let token = Lexer::new(body, lang).next();
        let parts = token.interp.unwrap();
        assert_eq!(&body[parts[0].0 as usize..parts[0].1 as usize], b"x");
    }
}
