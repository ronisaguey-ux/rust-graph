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
//! that borrow.
//!
//! # Wiring it into `deep_rt` (three small changes, in crates/deep-rt)
//!
//! The runtime already scans for a closer, twice: token by token in
//! `recovery::matching_close(Closer::Symbol)` and byte by byte for
//! `Closer::Byte`. Both are O(group size) and, on an unclosed opener near the
//! top of a file, O(file). The pair table makes the same decision in O(log n).
//!
//! 1. `driver.rs`, `Config`: add the table beside `source`:
//!
//!    ```ignore
//!    pub pair: Option<&'a [u32]>,
//!    ```
//!
//!    and a builder that takes it:
//!
//!    ```ignore
//!    pub fn with_pair_table(mut self, pair: &'a [u32]) -> Config<'a> {
//!        self.pair = Some(pair);
//!        self
//!    }
//!    ```
//!
//! 2. `recovery.rs`, `Parser::matching_close`: before the symbol or byte scan,
//!    ask the table:
//!
//!    ```ignore
//!    if let Some(pair) = self.config.pair {
//!        if let Some(close_at) = deep_lex::brackets::jump_in(pair, token.start) {
//!            return self.token_ending_after(close_at, from);
//!        }
//!    }
//!    ```
//!
//!    (`jump_in` is the free function; `deep_rt` may depend on `deep-lex` for
//!    it, or copy its three lines if that dependency direction is unwanted.)
//!
//! 3. The caller fills it in, in the same place it builds `Config`:
//!
//!    ```ignore
//!    let config = deep_rt::Config::new()
//!        .with_pairs(&symbol_pairs)
//!        .with_source(src)
//!        .with_pair_table(lexer.pair());
//!    ```
//!
//!    `lexer.pair()` must be taken before the `&mut dyn Lexer` borrow starts;
//!    `DeepLexer::pair_jump()` exists for that shape.
//!
//! Until step 1 lands, `DeepLexer::jump` and [`PairJump`] are still the right
//! answer for any caller that owns the lexer: the tests here drive a failed
//! group to its closer in one step.

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
