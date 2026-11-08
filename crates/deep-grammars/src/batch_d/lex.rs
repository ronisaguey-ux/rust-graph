//! Scanning helpers shared by the long-tail lexers: comments, quoted literals
//! and numbers. Each language's step lexer still decides the tokens; these only
//! do the byte-level scanning every language repeats.

use crate::deep::lexer::Cursor;

/// One whole UTF-8 character, since several lexers step over text they do not
/// tokenise.
pub fn advance(c: &mut Cursor) {
    c.bump_char();
}

/// A word byte in the C-like sense: `[A-Za-z0-9_]`, or any non-ASCII byte, so a
/// UTF-8 identifier is one token.
#[inline]
pub fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// A `#`, `//`, `--` or `;` line comment: everything to the end of the line.
pub fn line_comment(c: &mut Cursor) {
    c.eat_while(|byte| byte != b'\n' && byte != b'\r');
}

/// A block comment from `open` to `close`, optionally nested. False at end of
/// input without a close, so the caller can mark the token bad.
pub fn block_comment(c: &mut Cursor, open: &[u8], close: &[u8], nested: bool) -> bool {
    c.bump_n(open.len());
    let mut depth = 1u32;
    while !c.at_end() {
        if c.starts_with(close) {
            c.bump_n(close.len());
            depth -= 1;
            if depth == 0 {
                return true;
            }
        } else if nested && c.starts_with(open) {
            c.bump_n(open.len());
            depth += 1;
        } else {
            c.bump_char();
        }
    }
    false
}

/// A quoted literal from its opening quote. A line break ends an unterminated
/// one, so a stray quote cannot swallow the file.
pub fn quoted(c: &mut Cursor, quote: u8) -> bool {
    c.bump();
    c.quoted(quote, true)
}

/// A numeric literal: decimal, and any `0x`/`0b`/`0o` form, with `_` between
/// digits, a fraction, an exponent and a trailing type suffix (`1_000_000u64`).
pub fn number(c: &mut Cursor) {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return;
    }
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.peek() == b'.' && c.peek_at(1).is_ascii_digit() {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'e' | b'E' | b'p' | b'P') {
        let sign = usize::from(matches!(c.peek_at(1), b'+' | b'-'));
        if c.peek_at(1 + sign).is_ascii_digit() {
            c.bump_n(1 + sign);
            c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
        }
    }
    c.eat_while(|byte| matches!(byte, b'f' | b'i' | b'u' | b'l' | b'L' | b'F' | b'd' | b'j' | b'J' | b'm' | b'M' | b'n' | b'N'));
}

/// The ASCII upper case of a word, for case-insensitive keyword lookup (SQL,
/// COBOL, Fortran, Pascal). Non-ASCII bytes are copied unchanged.
pub fn upper(text: &[u8]) -> Vec<u8> {
    text.iter().map(u8::to_ascii_uppercase).collect()
}
