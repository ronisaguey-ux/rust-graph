//! A cursor over the original bytes. It only ever holds a slice and an index:
//! every "string" the lexer sees is a `Span` into that slice, never a copy.

/// Half-open byte range `[start, end)` into the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }
    pub const fn empty(at: u32) -> Span {
        Span { start: at, end: at }
    }
    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// A position in the source. Copy is deliberate: a lookahead checkpoint is a
/// usize, never a buffer.
#[derive(Clone, Copy)]
pub struct Cursor<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(src: &'a [u8]) -> Cursor<'a> {
        Cursor { src, pos: 0 }
    }

    pub fn at(src: &'a [u8], pos: usize) -> Cursor<'a> {
        Cursor {
            src,
            pos: pos.min(src.len()),
        }
    }

    pub fn source(&self) -> &'a [u8] {
        self.src
    }

    pub fn len(&self) -> usize {
        self.src.len()
    }

    pub fn pos(&self) -> u32 {
        self.pos as u32
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    pub fn remaining(&self) -> usize {
        self.src.len().saturating_sub(self.pos)
    }

    pub fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    pub fn peek_n(&self, n: usize) -> Option<u8> {
        self.src.get(self.pos.saturating_add(n)).copied()
    }

    pub fn starts_with(&self, pat: &[u8]) -> bool {
        self.src
            .get(self.pos..)
            .map_or(false, |rest| rest.starts_with(pat))
    }

    pub fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    pub fn advance(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.src.len());
    }

    pub fn set(&mut self, pos: usize) {
        self.pos = pos.min(self.src.len());
    }

    pub fn rest(&self) -> &'a [u8] {
        self.src.get(self.pos..).unwrap_or(&[])
    }

    /// A clamped slice, so a caller with a bad offset cannot panic.
    pub fn slice(&self, start: u32, end: u32) -> &'a [u8] {
        let start = (start as usize).min(self.src.len());
        let end = (end as usize).min(self.src.len()).max(start);
        self.src.get(start..end).unwrap_or(&[])
    }

    pub fn span(&self, start: u32) -> Span {
        Span::new(start, self.pos())
    }

    /// Consume bytes while `f` holds. Returns the number consumed.
    pub fn skip_while(&mut self, f: impl Fn(u8) -> bool) -> usize {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if !f(b) {
                break;
            }
            self.pos += 1;
        }
        self.pos - start
    }

    /// Decode the UTF-8 scalar at the cursor without allocating. Returns the
    /// scalar and its byte length, or `None` on an invalid or truncated
    /// sequence — callers then treat the single byte as opaque, never panic.
    pub fn decode(&self) -> Option<(char, usize)> {
        decode_utf8(self.src, self.pos)
    }
}

/// Manual UTF-8 decoding: `char::from_u32` plus the overlong-length checks a
/// decoder needs. A malformed sequence is `None`, not an error.
pub fn decode_utf8(src: &[u8], at: usize) -> Option<(char, usize)> {
    let b0 = *src.get(at)?;
    let (len, mut cp) = match b0 {
        0x00..=0x7f => return Some((b0 as char, 1)),
        0xc2..=0xdf => (2, (b0 & 0x1f) as u32),
        0xe0..=0xef => (3, (b0 & 0x0f) as u32),
        0xf0..=0xf4 => (4, (b0 & 0x07) as u32),
        _ => return None,
    };
    for k in 1..len {
        let b = *src.get(at + k)?;
        if b & 0xc0 != 0x80 {
            return None;
        }
        cp = (cp << 6) | (b & 0x3f) as u32;
    }
    // Reject overlong forms and out-of-range scalars.
    let min = match len {
        2 => 0x80,
        3 => 0x800,
        _ => 0x1_0000,
    };
    if cp < min || cp > 0x10_ffff {
        return None;
    }
    char::from_u32(cp).map(|c| (c, len))
}

/// An identifier may start with a letter or `_`/`$`; std's Unicode tables do
/// the work, so a multibyte identifier costs a decode and no allocation.
pub fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

pub fn is_ident_continue(c: char) -> bool {
    is_ident_start(c) || c.is_numeric() || c == '\u{200c}' || c == '\u{200d}'
}

/// True for the ASCII bytes that never appear in an identifier, used to bound
/// fast paths without decoding.
pub fn is_ascii_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | 0x0b | 0x0c)
}

pub fn is_line_terminator(b: u8) -> bool {
    b == b'\n' || b == b'\r'
}
