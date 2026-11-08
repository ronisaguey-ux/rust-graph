//! Incremental re-parsing over the flat CST arena.
//!
//! A batch parse throws the tree away and starts at byte zero. After a one-byte
//! edit that is almost all wasted work: the bytes are the same, so the LR
//! states are the same, so every reduction far from the edit produces exactly
//! the node it produced last time.
//!
//! This module re-parses from the smallest subtree that covers the edit and
//! splices everything else out of the old tree.
//!
//! # Why a splice is sound
//!
//! The driver is a deterministic LR parser: the tree it builds is a function of
//! the token stream *and* the parser state. For a node `N` of the old tree,
//! [`reparse`] records `state_before(N)`, the state on top of the stack at the
//! first byte of `N`, and the stack height there. The old parse produced `N`
//! from that state over exactly those bytes.
//!
//! So if the new parse reaches the same byte position (shifted by the edit's
//! length delta) with the same top state and the same stack height, and the
//! bytes of `N` are untouched, then re-parsing those bytes *must* rebuild `N`
//! node for node: same shifts, same reductions, same kinds, same spans. The
//! splice does exactly that work's result — it copies the old subtree with the
//! offsets shifted by `delta` and pushes it as one pre-reduced stack entry,
//! which is what the reduction would have pushed.
//!
//! The stack below a spliced node is never touched (an LR reduction pops only
//! the children it is about to wrap, all of which lie inside the node), so
//! induction over the splices keeps the current stack equal to the old one at
//! the matching position, and the final tree is identical to a full parse.
//! [`crate::incremental`]'s differential test checks that on real Python
//! sources rather than trusting the argument.
//!
//! # Where reuse stops
//!
//! * a node whose byte range the edit touches is rebuilt;
//! * a node whose *parse context* changed is rebuilt: if the edit changes how
//!   the tokens before it reduce (indentation is the loud case — one dedent
//!   moves every `DEDENT` token after it), the state at the splice candidate no
//!   longer matches and the candidate is refused;
//! * a tree with error nodes is not reused at all: §Recovery's missing/error
//!   nodes are inserted or adopted rather than shifted, so the recorded states
//!   would be a guess. [`reparse`] falls back to a full parse in that case,
//!   which is always correct and never produces a different tree.
//!
//! Tokenisation is *not* incremental here: the caller passes the whole new
//! token stream. Lexing is linear and much cheaper than parsing, and a
//! context-sensitive lexer (Python layout, mode stacks) cannot in general be
//! restarted at a byte offset without replaying it anyway.
//!
//! # Wiring
//!
//! The module is self-contained: it only uses the public `Arena::nodes` field
//! and the public table functions, so enabling it needs one line in
//! `crates/deep-rt/src/lib.rs`:
//!
//! ```text
//! pub mod incremental;
//! ```
//!
//! The tests drive the file through `#[path]`, so they pass before that line
//! lands.

use deep_abi::{
    Cst, Fatal, Grammar, Lexer, Node, NodeId, Offset, StateId, SymbolId, Token, EOF_SYMBOL,
    FLAG_ERROR, FLAG_MISSING, NONE,
};

use crate::cst::Arena;
use crate::driver::{parse, Config};
use crate::table::{self, Action};

/// One edit, in the old source's byte coordinates.
///
/// `start .. start + old_len` is replaced by `new_len` bytes. Insertions have
/// `old_len == 0`, deletions `new_len == 0`.
///
/// ```
/// # use deep_rt::incremental::Edit;
/// let insert = Edit::insert(10, 3);          // three bytes at offset 10
/// let delete = Edit::delete(10, 3);          // drop three bytes at offset 10
/// let replace = Edit::replace(10, 3, 5);     // three bytes become five
/// assert_eq!(insert.delta(), 3);
/// assert_eq!(delete.delta(), -3);
/// assert_eq!(replace.delta(), 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// First byte the edit touches.
    pub start: Offset,
    /// Bytes removed, in the old source.
    pub old_len: u32,
    /// Bytes inserted, in the new source.
    pub new_len: u32,
}

impl Edit {
    pub fn insert(start: Offset, len: u32) -> Edit {
        Edit {
            start,
            old_len: 0,
            new_len: len,
        }
    }

    pub fn delete(start: Offset, len: u32) -> Edit {
        Edit {
            start,
            old_len: len,
            new_len: 0,
        }
    }

    pub fn replace(start: Offset, old_len: u32, new_len: u32) -> Edit {
        Edit {
            start,
            old_len,
            new_len,
        }
    }

    /// Last byte of the old range.
    #[inline]
    pub fn old_end(&self) -> Offset {
        self.start.saturating_add(self.old_len)
    }

    /// Last byte of the new range.
    #[inline]
    pub fn new_end(&self) -> Offset {
        self.start.saturating_add(self.new_len)
    }

    /// New length minus old length. Negative for a deletion.
    #[inline]
    pub fn delta(&self) -> i64 {
        self.new_len as i64 - self.old_len as i64
    }

    /// Apply the edit: splice `text` into `source` at `start`.
    ///
    /// `text` must be `new_len` bytes; a mismatch is truncated or padded with
    /// nothing, so the caller's `Edit` stays the single source of truth.
    pub fn apply(&self, source: &[u8], text: &[u8]) -> Vec<u8> {
        let start = (self.start as usize).min(source.len());
        let end = (self.old_end() as usize).min(source.len());
        let text = &text[..text.len().min(self.new_len as usize)];
        let mut out = Vec::with_capacity(source.len() - (end - start) + text.len());
        out.extend_from_slice(&source[..start]);
        out.extend_from_slice(text);
        out.extend_from_slice(&source[end..]);
        out
    }
}

/// What the incremental pass cost, and how much of the old tree survived.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// First byte re-parsed.
    pub window_start: Offset,
    /// Last byte of the covering node in old coordinates.
    pub window_end: Offset,
    /// Nodes of the old tree that were reused verbatim (whole count, subtrees
    /// included): the untouched prefix plus every spliced suffix subtree.
    pub reused_nodes: u32,
    /// Bytes covered by reused nodes.
    pub reused_bytes: u64,
    /// Nodes the incremental pass allocated by parsing.
    pub rebuilt_nodes: u32,
    /// Nodes in the resulting tree.
    pub total_nodes: u32,
    /// How many suffix subtrees were spliced in (one per resumed region).
    pub splices: u32,
    /// True when the incremental pass gave up and re-parsed everything (error
    /// nodes in the old tree, or a state that did not line up).
    pub fell_back: bool,
}

impl Report {
    /// Reused nodes over total nodes. `1.0` means nothing was rebuilt.
    pub fn reuse_ratio(&self) -> f64 {
        if self.total_nodes == 0 {
            return 1.0;
        }
        self.reused_nodes as f64 / self.total_nodes as f64
    }
}

/// Re-parse `tokens` (the whole new source) against the tree of the old source.
///
/// `old` must be the tree of the previous parse, `old_len` its source length,
/// and `tokens` the new source's terminals in order (`EOF` is implicit; a
/// trailing `EOF_SYMBOL` token is tolerated). `config` is only used when the
/// pass has to fall back to a full parse, so its `source` should be the **new**
/// source.
///
/// The returned tree is identical — same kinds, same spans, same shape — to
/// [`parse`] over the same tokens. On any inconsistency the fallback guarantees
/// that, at the cost of no reuse.
pub fn reparse(
    grammar: &Grammar<'_>,
    old: &Cst,
    old_len: Offset,
    edit: &Edit,
    tokens: &[Token],
    config: &Config<'_>,
) -> Result<(Cst, Report), Fatal> {
    let tokens = trim_eof(tokens);
    let new_len = (old_len as i64 + edit.delta()).max(0) as Offset;
    let mut report = Report {
        window_start: 0,
        window_end: old_len,
        ..Report::default()
    };

    // Everything that could make the recorded states a guess, or that leaves no
    // tree to reuse, goes down the full-parse path.
    let usable = !old.nodes.is_empty()
        && old.root != NONE
        && (old.root as usize) < old.nodes.len()
        && old.errors == 0
        && edit.old_end() <= old_len;

    // A window whose prefix still does not line up is widened one level per
    // attempt; the root is always sound and always available as a last resort.
    if usable {
        for widen in 0..6 {
            let Some(prepared) = prepare(grammar, old, edit, old_len, tokens, widen) else {
                continue;
            };
            let mut attempt = Report {
                window_start: 0,
                window_end: old_len,
                ..Report::default()
            };
            if let Some(cst) = run(grammar, old, edit, tokens, new_len, &prepared, &mut attempt) {
                return Ok((cst, attempt));
            }
        }
    }

    let (cst, _) = full_parse(grammar, tokens, config)?;
    report.fell_back = true;
    report.total_nodes = cst.nodes.len() as u32;
    report.rebuilt_nodes = report.total_nodes;
    Ok((cst, report))
}

/// Drop a trailing `EOF` token if the caller supplied one; the driver adds its
/// own, and a second would be shifted twice.
fn trim_eof(tokens: &[Token]) -> &[Token] {
    match tokens.last() {
        Some(token) if token.symbol == EOF_SYMBOL => &tokens[..tokens.len() - 1],
        _ => tokens,
    }
}

/// A token-slice lexer, for the fallback path.
struct Slices<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Lexer for Slices<'_> {
    fn next_token(&mut self, _state: StateId) -> Option<Token> {
        let token = self.tokens.get(self.at).copied();
        if token.is_some() {
            self.at += 1;
        }
        token
    }
}

fn full_parse(
    grammar: &Grammar<'_>,
    tokens: &[Token],
    config: &Config<'_>,
) -> Result<(Cst, crate::driver::Stats), Fatal> {
    let mut lexer = Slices { tokens, at: 0 };
    parse(grammar, &mut lexer, config)
}

