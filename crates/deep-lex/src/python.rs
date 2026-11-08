//! Python's logical-line rules, kept out of the main state machine so they can
//! be read and tested on their own.
//!
//! The lexer tracks three things here: the column of each open suite (a stack of
//! widths), whether the current physical line has code on it, and whether
//! newlines are suppressed because a bracket is open or a backslash continues
//! the line.

use crate::cursor::{is_line_terminator, Cursor};

/// A tab advances to the next multiple of this, as CPython's tokenizer does.
pub const TAB_WIDTH: u32 = 8;

/// Consume leading spaces and tabs and return the column they reach. Stops at
/// the first byte that cannot be indentation.
pub fn measure_indent(cur: &mut Cursor<'_>, tab_width: u32) -> u32 {
    let mut col = 0u32;
    while let Some(b) = cur.peek() {
        match b {
            b' ' => {
                col = col.saturating_add(1);
                cur.bump();
            }
            b'\t' => {
                let step = tab_width.max(1);
                col = (col / step + 1) * step;
                cur.bump();
            }
            b'\x0c' => {
                // Form feed resets the column, as CPython does.
                col = 0;
                cur.bump();
            }
            _ => break,
        }
    }
    col
}

/// Length of the line terminator starting at the cursor: 2 for CRLF, 1 for a
/// lone LF or CR, 0 when there is none.
pub fn terminator_len(cur: &Cursor<'_>) -> usize {
    match (cur.peek(), cur.peek_n(1)) {
        (Some(b'\r'), Some(b'\n')) => 2,
        (Some(b'\n'), _) | (Some(b'\r'), _) => 1,
        _ => 0,
    }
}

/// Consume a line terminator if present. Returns whether one was consumed.
pub fn eat_terminator(cur: &mut Cursor<'_>) -> bool {
    let n = terminator_len(cur);
    if n == 0 {
        return false;
    }
    cur.advance(n);
    true
}

/// True when a `#` comment starts here. Python has no `//`.
pub fn is_comment_start(cur: &Cursor<'_>) -> bool {
    cur.peek() == Some(b'#')
}

/// Consume a `#` comment up to (but not including) the line terminator.
pub fn eat_line_comment(cur: &mut Cursor<'_>) -> usize {
    let start = cur.pos();
    while let Some(b) = cur.peek() {
        if is_line_terminator(b) {
            break;
        }
        cur.bump();
    }
    (cur.pos() - start) as usize
}

/// True when the physical line starting at the cursor holds neither code nor a
/// comment — only the terminator. Called after leading whitespace is consumed.
pub fn is_blank_line(cur: &Cursor<'_>) -> bool {
    matches!(cur.peek(), None | Some(b'\n') | Some(b'\r'))
}
