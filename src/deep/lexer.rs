//! The lexer core: struct-of-arrays tokens over `&[u8]`, no allocation per token.
//!
//! Tokens cover the input with no gaps, trivia included, so a token's start is the
//! previous token's end and the token texts concatenate back to the input. Tokens
//! are final before parsing starts: recovery can never move a token boundary.

use super::syntax::{Kind, Language, UNKNOWN};

/// Tokens as two parallel arrays. Start of token `i` is the end of token `i - 1`.
#[derive(Default)]
pub struct Tokens {
    pub kind: Vec<Kind>,
    pub end: Vec<u32>,
}

impl Tokens {
    pub fn clear(&mut self) {
        self.kind.clear();
        self.end.clear();
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.kind.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    #[inline]
    pub fn start(&self, index: usize) -> u32 {
        if index == 0 { 0 } else { self.end[index - 1] }
    }

    #[inline]
    pub fn text<'s>(&self, src: &'s [u8], index: usize) -> &'s [u8] {
        &src[self.start(index) as usize..self.end[index] as usize]
    }

    #[inline]
    fn push(&mut self, kind: Kind, end: u32) {
        self.kind.push(kind);
        self.end.push(end);
    }
}

/// A position in the input, with the small set of moves every lexer needs.
pub struct Cursor<'s> {
    src: &'s [u8],
    pos: usize,
}

impl<'s> Cursor<'s> {
    pub fn new(src: &'s [u8]) -> Self {
        Cursor { src, pos: 0 }
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    pub fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// The byte at the cursor, or 0 at the end.
    #[inline]
    pub fn peek(&self) -> u8 {
        self.peek_at(0)
    }

    #[inline]
    pub fn peek_at(&self, ahead: usize) -> u8 {
        self.src.get(self.pos + ahead).copied().unwrap_or(0)
    }

    #[inline]
    pub fn rest(&self) -> &'s [u8] {
        &self.src[self.pos.min(self.src.len())..]
    }

    #[inline]
    pub fn starts_with(&self, prefix: &[u8]) -> bool {
        self.rest().starts_with(prefix)
    }

    #[inline]
    pub fn bump(&mut self) {
        if self.pos < self.src.len() {
            self.pos += 1;
        }
    }

    #[inline]
    pub fn bump_n(&mut self, count: usize) {
        self.pos = (self.pos + count).min(self.src.len());
    }

    #[inline]
    pub fn eat(&mut self, byte: u8) -> bool {
        if !self.at_end() && self.src[self.pos] == byte {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn eat_while(&mut self, mut accept: impl FnMut(u8) -> bool) -> usize {
        let start = self.pos;
        while self.pos < self.src.len() && accept(self.src[self.pos]) {
            self.pos += 1;
        }
        self.pos - start
    }

    /// One whole UTF-8 character, or one byte if the input is not UTF-8 here.
    pub fn bump_char(&mut self) {
        let width = match self.peek() {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        let mut taken = 1;
        while taken < width && matches!(self.peek_at(taken), 0x80..=0xbf) {
            taken += 1;
        }
        self.bump_n(taken);
    }

    /// The rest of a quoted literal, after its opening quote. A backslash escapes the
    /// next byte. With `stop_at_newline` an unterminated literal ends before the newline,
    /// so one stray quote cannot swallow the rest of the file. True if it closed.
    pub fn quoted(&mut self, quote: u8, stop_at_newline: bool) -> bool {
        while let Some(&byte) = self.src.get(self.pos) {
            match byte {
                b'\\' => self.bump_n(2),
                b'\n' | b'\r' if stop_at_newline => return false,
                _ if byte == quote => {
                    self.pos += 1;
                    return true;
                }
                _ => self.pos += 1,
            }
        }
        false
    }
}

/// Inputs this large cannot be addressed with `u32` offsets.
pub const MAX_INPUT: usize = u32::MAX as usize - 1;

/// Lexes `src` into `tokens` with the language's step function. Every step must
/// advance; a step that does not costs one character as [`UNKNOWN`], so lexing
/// always terminates. Runs of unknown characters merge into one token.
pub fn lex(src: &[u8], language: &Language, tokens: &mut Tokens) {
    assert!(src.len() <= MAX_INPUT, "input over 4 GiB reached the lexer");
    tokens.clear();
    let mut cursor = Cursor::new(src);
    while !cursor.at_end() {
        let start = cursor.pos;
        let mut kind = (language.lex)(&mut cursor);
        if cursor.pos <= start {
            cursor.pos = start;
            cursor.bump_char();
            kind = UNKNOWN;
        }
        let end = cursor.pos as u32;
        if kind == UNKNOWN && tokens.kind.last() == Some(&UNKNOWN) {
            *tokens.end.last_mut().expect("a last kind has a last end") = end;
        } else {
            tokens.push(kind, end);
        }
    }
}
