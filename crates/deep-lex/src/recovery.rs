//! The lexer half of the runtime's cascade recovery.
//!
//! `deep_rt`'s `Abandon` rule wraps a failed bracketed group in one ERROR node
//! and resumes after its closer. Finding that closer by walking tokens is
//! O(group size) and, on a file with one unclosed `(` early, O(file). The
//! lexer already computed every mate in one O(n) pass at construction; this
//! module hands that table to the driver in the shape its `Config` takes.
//!
//! It is deliberately a plain slice, not a closure over the lexer: the driver
//! borrows `&mut dyn Lexer` while it may need to jump, and a `&[u32]` outlives
//! that borrow. See `docs` on [`PairJump`] for the exact driver change.

use crate::brackets::jump_in;

/// A borrowed view of [`Brackets::pair`](crate::Brackets::pair).
///
/// ```text
/// pair[2*i]     byte offset of delimiter i
/// pair[2*i + 1] ordinal of its mate, or NONE
/// ```
///
/// `jump(offset)` is a binary search over the ascending offsets followed by one
/// indexed load: O(log n) and no allocation. A failed group therefore costs a
/// jump, not a scan.
#[derive(Clone, Copy, Debug)]
pub struct PairJump<'a> {
    /// The raw slots, exactly as the lexer built them.
    pub pair: &'a [u32],
}

impl<'a> PairJump<'a> {
    pub fn new(pair: &'a [u32]) -> PairJump<'a> {
        PairJump { pair }
    }

    /// The mate's byte offset, for an opener or a closer alike.
    pub fn jump(&self, offset: u32) -> Option<u32> {
        jump_in(self.pair, offset)
    }

    /// Where recovery resumes after failing on `offset`: the byte just past the
    /// closing delimiter, and only when `offset` really is an opener and has a
    /// mate. `None` means "no cascade"; skip one token instead.
    pub fn skip_to_closer(&self, src: &[u8], offset: u32) -> Option<u32> {
        let byte = *src.get(offset as usize)?;
        if !matches!(byte, b'(' | b'[' | b'{') {
            return None;
        }
        Some(self.jump(offset)?.saturating_add(1))
    }

    /// Where recovery resumes after failing on the token that opens the group
    /// at `offset`, when the closer is itself the last byte of input.
    pub fn group_end(&self, src: &[u8], offset: u32) -> Option<u32> {
        let byte = *src.get(offset as usize)?;
        if !matches!(byte, b'(' | b'[' | b'{') {
            return None;
        }
        self.jump(offset)
    }
}
