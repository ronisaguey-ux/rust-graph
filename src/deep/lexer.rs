//! The lexer core: struct-of-arrays tokens over `&[u8]`, no allocation per token.
//!
//! Tokens cover the input with no gaps, trivia included, so a token's start is the
//! previous token's end and the token texts concatenate back to the input. Tokens
//! are final before parsing starts: recovery can never move a token boundary.

use super::syntax::{Class, Kind, Language, Layout, UNKNOWN, WHITESPACE};

/// Tokens as two parallel arrays. Start of token `i` is the end of token `i - 1`.
#[derive(Default)]
pub struct Tokens {
    pub kind: Vec<Kind>,
    pub end: Vec<u32>,
    /// The layout pass's stack of indentation columns, kept between files.
    levels: Vec<u32>,
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

    /// A zero-width token at `index`, where the token before it ends. Only ever
    /// moves the trivia after `index`, a few tokens.
    fn insert_empty(&mut self, index: usize, kind: Kind) {
        let at = self.start(index);
        self.kind.insert(index, kind);
        self.end.insert(index, at);
    }
}

/// A position in the input, with the small set of moves every lexer needs.
pub struct Cursor<'s> {
    src: &'s [u8],
    pos: usize,
    /// The kind of the last token that is not trivia, or `UNKNOWN` at the start.
    previous: Kind,
}

impl<'s> Cursor<'s> {
    pub fn new(src: &'s [u8]) -> Self {
        Cursor { src, pos: 0, previous: UNKNOWN }
    }

    /// The kind of the last significant token before the cursor, or `UNKNOWN`
    /// at the start of the file. Set by the lexer driver, so a step lexer can
    /// tell what a token may be from what came before it: in JavaScript a `/`
    /// after an operand divides, and anywhere else starts a regular expression.
    #[inline]
    pub fn previous(&self) -> Kind {
        self.previous
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

    /// The byte `back` places before the cursor, or 0 before the start. Lets a
    /// step lexer see what it just passed, such as the `.` before a tuple index.
    #[inline]
    pub fn behind(&self, back: usize) -> u8 {
        self.pos.checked_sub(back).and_then(|at| self.src.get(at)).copied().unwrap_or(0)
    }

    /// The input from `start` to the cursor: the text of the token so far.
    #[inline]
    pub fn since(&self, start: usize) -> &'s [u8] {
        &self.src[start.min(self.pos)..self.pos.min(self.src.len())]
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
    if let Some(layout) = language.layout {
        return lex_layout(src, language, layout, tokens);
    }
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
        if !language.is_trivia(kind) {
            cursor.previous = kind;
        }
        if kind == UNKNOWN && tokens.kind.last() == Some(&UNKNOWN) {
            *tokens.end.last_mut().expect("a last kind has a last end") = end;
        } else {
            tokens.push(kind, end);
        }
    }
}

/// The width of an indentation, as Python counts it: a tab moves to the next
/// multiple of 8, a form feed starts again at 0. Other bytes (a byte order mark)
/// count nothing.
fn indentation(bytes: &[u8]) -> u32 {
    let mut column = 0u32;
    for &byte in bytes {
        match byte {
            b' ' => column += 1,
            b'\t' => column = (column / 8 + 1) * 8,
            b'\x0c' => column = 0,
            _ => {}
        }
    }
    column
}

/// [`lex`] for a language with [`Layout`]: the same steps, plus the layout
/// tokens. One pass; inserting a zero-width token moves only the trivia after
/// the last real token.
fn lex_layout(src: &[u8], language: &Language, layout: &Layout, tokens: &mut Tokens) {
    let mut levels = std::mem::take(&mut tokens.levels);
    levels.clear();
    levels.push(0);
    // Open brackets, not counting layout's own pair.
    let mut depth = 0u32;
    // A token of this logical line has been seen, so a line break ends it.
    let mut in_line = false;
    // Where the current physical line starts, and whether a token has been seen on it.
    let mut line_start = 0usize;
    let mut line_fresh = true;
    // One past the last significant token: where a `newline` goes.
    let mut after_last = 0usize;
    // Where the `dedent`s for the next logical line go: just after its `newline`.
    let mut after_newline = 0usize;
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
        if kind != layout.newline && !language.is_trivia(kind) {
            cursor.previous = kind;
        }
        if kind == layout.newline {
            tokens.push(WHITESPACE, end);
            line_start = cursor.pos;
            line_fresh = true;
            if depth == 0 && in_line {
                tokens.insert_empty(after_last, layout.newline);
                after_newline = after_last + 1;
                in_line = false;
            }
            continue;
        }
        let class = language.class(kind);
        if language.is_trivia(kind) {
            tokens.push(kind, end);
            continue;
        }
        if depth > 0 && line_fresh && layout.resets.contains(&kind) {
            // `foo(` then a line starting with `def`: the bracket cannot go on.
            depth = 0;
            tokens.insert_empty(after_last, layout.newline);
            after_newline = after_last + 1;
            in_line = false;
        }
        line_fresh = false;
        if !in_line {
            let column = indentation(&src[line_start..start]);
            if column > *levels.last().expect("level 0 stays") {
                levels.push(column);
                tokens.push(layout.indent, start as u32);
            } else {
                let mut dedents = 0;
                while column < *levels.last().expect("level 0 stays") {
                    levels.pop();
                    dedents += 1;
                }
                for _ in 0..dedents {
                    tokens.insert_empty(after_newline, layout.dedent);
                }
                if column > *levels.last().expect("level 0 stays") {
                    // A dedent to a column no outer line used: an error the
                    // grammar sees as an unexpected block.
                    levels.push(column);
                    tokens.push(layout.indent, start as u32);
                }
            }
            in_line = true;
        }
        match class {
            Class::Open(_) => depth += 1,
            Class::Close(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        if kind == UNKNOWN && tokens.kind.last() == Some(&UNKNOWN) {
            *tokens.end.last_mut().expect("a last kind has a last end") = end;
        } else {
            tokens.push(kind, end);
        }
        after_last = tokens.len();
    }
    if in_line {
        tokens.insert_empty(after_last, layout.newline);
        after_newline = after_last + 1;
    }
    for _ in 1..levels.len() {
        tokens.insert_empty(after_newline, layout.dedent);
    }
    tokens.levels = levels;
}
