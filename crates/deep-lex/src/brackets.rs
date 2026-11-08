//! One O(n) pass that pairs every delimiter, so the parser's recovery can jump
//! from an unbalanced bracket to its mate instead of guessing.
//!
//! The result is a single `Vec<u32>` — the only allocation this module makes.
//! It holds two slots per delimiter, in source order:
//!
//! ```text
//! pair[2*i]     byte offset of delimiter i
//! pair[2*i + 1] ordinal of its mate, or NONE when unmatched
//! ```
//!
//! Building it needs a stack of open delimiters. Rather than allocate a second
//! `Vec`, the pass keeps the stack *inside* the mate slot as a link to the
//! previous opener and overwrites that slot with the mate ordinal on a match.
//! Unmatched openers are relinked to `NONE` in one final walk from the stack
//! head. So: one `Vec`, one linear scan, O(n) time.

use crate::cursor::Cursor;

/// No mate.
pub const NONE: u32 = u32::MAX;

/// Bracket family: 1 = round, 2 = square, 3 = curly, 0 = not a bracket.
fn family(b: u8) -> u8 {
    match b {
        b'(' | b')' => 1,
        b'[' | b']' => 2,
        b'{' | b'}' => 3,
        _ => 0,
    }
}

fn is_open(b: u8) -> bool {
    matches!(b, b'(' | b'[' | b'{')
}

fn is_close(b: u8) -> bool {
    matches!(b, b')' | b']' | b'}')
}

/// All delimiter pairs of one source. Cheap to build, cheap to query.
#[derive(Clone, Debug)]
pub struct Brackets {
    pair: Vec<u32>,
}

impl Default for Brackets {
    fn default() -> Self {
        Brackets::empty()
    }
}

/// Ordinal of the delimiter starting at `offset`, if any. Offsets are
/// ascending, so this is a binary search: O(log n), no allocation.
pub(crate) fn ordinal_in(pair: &[u32], offset: u32) -> Option<usize> {
    let delimiters = pair.len() / 2;
    let mut lo = 0usize;
    let mut hi = delimiters;
    while lo < hi {
        let mid = (lo + hi) / 2;
        match pair[2 * mid] {
            o if o == offset => return Some(mid),
            o if o < offset => lo = mid + 1,
            _ => hi = mid,
        }
    }
    None
}

/// The recovery jump over a raw pair table: delimiter offset to its mate's
/// offset. Shared by [`Brackets::jump`] and
/// [`PairJump`](crate::recovery::PairJump) so the runtime and the lexer agree
/// on one encoding.
pub fn jump_in(pair: &[u32], offset: u32) -> Option<u32> {
    let ordinal = ordinal_in(pair, offset)?;
    match pair.get(2 * ordinal + 1).copied()? {
        NONE => None,
        mate => pair.get(2 * mate as usize).copied(),
    }
}

impl Brackets {
    pub fn empty() -> Brackets {
        Brackets { pair: Vec::new() }
    }

    /// Scan `src` once. Any byte that is not a delimiter is skipped, and a
    /// mismatch never aborts: the closer is marked unmatched and scanning
    /// continues, which is what recovery wants.
    pub fn scan(src: &[u8]) -> Brackets {
        Brackets::scan_cursor(Cursor::new(src))
    }

    pub fn scan_cursor(mut cur: Cursor<'_>) -> Brackets {
        let src = cur.source();
        // Upper bound on delimiters keeps the single Vec from reallocating.
        let bound = src.iter().filter(|&&b| is_open(b) || is_close(b)).count();
        let mut pair: Vec<u32> = Vec::with_capacity(bound.saturating_mul(2));
        let mut head: u32 = NONE;

        while let Some(b) = cur.bump() {
            if !is_open(b) && !is_close(b) {
                continue;
            }
            let at = cur.pos() - 1;
            let idx = (pair.len() / 2) as u32;
            pair.push(at);
            pair.push(NONE);

            if is_open(b) {
                // The mate slot is the stack link until this opener matches.
                pair[2 * idx as usize + 1] = head;
                head = idx;
                continue;
            }

            // A closer only matches an opener of the same family.
            if head != NONE {
                let open_at = pair[2 * head as usize];
                let open_byte = src.get(open_at as usize).copied().unwrap_or(0);
                if family(open_byte) == family(b) {
                    let link = pair[2 * head as usize + 1];
                    pair[2 * idx as usize + 1] = head;
                    pair[2 * head as usize + 1] = idx;
                    head = link;
                    continue;
                }
            }
            // Unmatched closer: its slot stays NONE and the stack is untouched.
        }

        // Anything still on the stack was never closed.
        while head != NONE {
            let link = pair[2 * head as usize + 1];
            pair[2 * head as usize + 1] = NONE;
            head = link;
        }

        Brackets { pair }
    }

    /// The raw slots: `2 * delimiters()` entries, see the module docs.
    pub fn pair(&self) -> &[u32] {
        &self.pair
    }

    pub fn delimiters(&self) -> usize {
        self.pair.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.pair.is_empty()
    }

    pub fn offset(&self, ordinal: usize) -> Option<u32> {
        self.pair.get(2 * ordinal).copied()
    }

    /// The mate of delimiter `ordinal`, as a delimiter ordinal.
    pub fn mate(&self, ordinal: usize) -> Option<usize> {
        match self.pair.get(2 * ordinal + 1).copied()? {
            NONE => None,
            other => Some(other as usize),
        }
    }

    /// The mate's byte offset, or `None` when unmatched / not a delimiter.
    pub fn mate_offset(&self, ordinal: usize) -> Option<u32> {
        let mate = self.mate(ordinal)?;
        self.offset(mate)
    }

    pub fn is_unmatched(&self, ordinal: usize) -> bool {
        matches!(self.pair.get(2 * ordinal + 1).copied(), Some(NONE))
    }

    /// Ordinal of the delimiter starting at `offset`, if any. Offsets are
    /// ascending, so this is a binary search: O(log n), no allocation.
    pub fn ordinal_at(&self, offset: u32) -> Option<usize> {
        ordinal_in(&self.pair, offset)
    }

    /// The recovery jump: from a delimiter's byte offset to its mate's byte
    /// offset. `None` for unmatched or absent.
    pub fn jump(&self, offset: u32) -> Option<u32> {
        jump_in(&self.pair, offset)
    }
}
