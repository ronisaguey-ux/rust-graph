//! The delimiter pass: one linear sweep that pairs every opener with its closer
//! before the parser starts.
//!
//! The parser binds a group's body to `pair[open]`, so an error inside a group can
//! never escape it (the #5910 firewall), and a group can be jumped whole in O(1).

use super::lexer::Tokens;
use super::syntax::{Class, Language};

/// No partner: an unclosed opener, a stray closer, or a token that is neither.
pub const NONE: u32 = u32::MAX;

/// How far below the top of the stack a closer looks for its opener. A closer
/// that finds its family within this window closes the openers above it as
/// unclosed (`{ ( }` pairs the braces); one that does not is stray. The window
/// keeps the pass linear: a run of stray closers cannot rescan a deep stack.
pub const SEARCH_WINDOW: usize = 8;

/// Snapshots are skipped above this stack depth, so a deep stack under many
/// `#if` lines cannot make the pass quadratic. Pairing still works, minus the
/// `#else` restore.
pub const MAX_SNAPSHOT_DEPTH: usize = 1024;

#[derive(Default)]
pub struct Pairs {
    /// For each token, the index of its partner, or [`NONE`].
    pub pair: Vec<u32>,
    pub unclosed: u32,
    pub stray: u32,
    stack: Vec<(u32, u8)>,
    /// `reach[i]`: the furthest closer that any stack entry at or below `i` is
    /// already paired with. Entries are paired while still on the stack only after
    /// `#else` restores them, so this is 0 outside preprocessor branches.
    reach: Vec<u32>,
    saved: Vec<(u32, u8)>,
    /// One per open `#if`: its token, and where its snapshot starts in `saved` and
    /// how long it is. The start is `NONE` when the snapshot was skipped.
    branches: Vec<(u32, u32, u32)>,
}

impl Pairs {
    pub fn build(&mut self, tokens: &Tokens, language: &Language) {
        self.pair.clear();
        self.pair.resize(tokens.len(), NONE);
        self.stack.clear();
        self.reach.clear();
        self.saved.clear();
        self.branches.clear();
        self.unclosed = 0;
        self.stray = 0;
        for (index, &kind) in tokens.kind.iter().enumerate() {
            let index = index as u32;
            match language.class(kind) {
                Class::Open(family) => {
                    self.stack.push((index, family));
                    self.reach.push(self.reach.last().copied().unwrap_or(0));
                }
                Class::Close(family) => self.close(index, family),
                Class::BranchIf => {
                    if self.stack.len() <= MAX_SNAPSHOT_DEPTH {
                        self.branches.push((index, self.saved.len() as u32, self.stack.len() as u32));
                        self.saved.extend_from_slice(&self.stack);
                    } else {
                        self.branches.push((index, NONE, 0));
                    }
                }
                Class::BranchElse => {
                    // Each branch starts from the stack as it was at `#if`.
                    if let Some(&(branch, at, len)) = self.branches.last() {
                        if at != NONE {
                            // Openers this branch left open are abandoned here.
                            self.unclosed += self.stack.iter().filter(|(open, _)| *open > branch).count() as u32;
                            self.stack.clear();
                            self.stack.extend_from_slice(&self.saved[at as usize..(at + len) as usize]);
                            self.recompute_reach(0);
                        }
                    }
                }
                Class::BranchEnd => {
                    // The last branch's stack carries on.
                    if let Some((_, at, _)) = self.branches.pop() {
                        if at != NONE {
                            self.saved.truncate(at as usize);
                        }
                    }
                }
                Class::Node | Class::Token | Class::Trivia | Class::BadToken => {}
            }
        }
        self.unclosed += self.stack.iter().filter(|(open, _)| self.pair[*open as usize] == NONE).count() as u32;
    }

    fn close(&mut self, index: u32, family: u8) {
        let depth = self.stack.len();
        let window = depth.min(SEARCH_WINDOW);
        let Some(found) = (1..=window).find(|back| self.stack[depth - back].1 == family) else {
            self.stray += 1;
            return;
        };
        let at = depth - found;
        let (open, _) = self.stack[at];
        for &(skipped, _) in &self.stack[at + 1..] {
            if self.pair[skipped as usize] == NONE {
                self.unclosed += 1;
            }
        }
        // An earlier branch may have paired an entry below with a closer past
        // `open`. Pairing now would cross it; the later branch wins, so that older
        // pair is undone and its closer becomes stray.
        if at > 0 && self.reach[at - 1] > open {
            for below in 0..at {
                let (older, _) = self.stack[below];
                let closer = self.pair[older as usize];
                if closer != NONE && closer > open {
                    self.pair[older as usize] = NONE;
                    self.pair[closer as usize] = NONE;
                    self.stray += 1;
                }
            }
            self.recompute_reach(0);
        }
        self.stack.truncate(at);
        self.reach.truncate(at);
        // An opener restored by `#else` may already be paired in an earlier branch.
        // The latest closer wins and the earlier one becomes stray, so `pair` stays
        // symmetric and the pairs still nest.
        let previous = self.pair[open as usize];
        if previous != NONE {
            self.pair[previous as usize] = NONE;
            self.stray += 1;
        }
        self.pair[open as usize] = index;
        self.pair[index as usize] = open;
    }

    fn recompute_reach(&mut self, from: usize) {
        self.reach.truncate(from);
        let mut furthest = if from == 0 { 0 } else { self.reach[from - 1] };
        for &(open, _) in &self.stack[from..] {
            let closer = self.pair[open as usize];
            if closer != NONE {
                furthest = furthest.max(closer);
            }
            self.reach.push(furthest);
        }
    }
}
