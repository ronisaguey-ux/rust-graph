//! Error recovery: insert, skip, and the cascade that keeps the root alive.
//!
//! See the crate docs for the contract; this module is the implementation of
//! [`deep_abi::Recovery`] on top of the driver's stack and arena.

use deep_abi::{Fatal, NodeId, Offset, StateId, SymbolId, Token, EOF_SYMBOL};

use crate::driver::Parser;
use crate::stack::GssId;
use crate::table::{self, Action};
use crate::version::{Lane, Version};

/// How the opening bracket we are sitting on closes.
enum Closer {
    Symbol(SymbolId),
    Byte(u8),
}

impl<'g, 'l, 'c> Parser<'g, 'l, 'c> {
    /// Attach the skipped-run node and pending error nodes to a finished tree.
    pub(crate) fn settle(&mut self, version: &mut Version, root: NodeId) {
        let extras: Vec<NodeId> = version.pending.drain(..).map(|(id, _, _)| id).collect();
        if !extras.is_empty() {
            self.arena.adopt_sorted(root, &extras);
        }
    }

    /// Turn a skip run, if any, into exactly one ERROR node.
    pub(crate) fn flush_skip(&mut self, version: &mut Version) {
        if let Some((start, end)) = version.skip.take() {
            let node = self.arena.error(start, end);
            version.pending.push((node, start, end));
        }
    }

    /// Give pending skipped input to the smallest reduction whose span holds it.
    pub(crate) fn adopt_pending(&mut self, version: &mut Version, parent: NodeId, start: Offset, end: Offset) {
        if version.pending.is_empty() {
            return;
        }
        let mut fitting: Vec<NodeId> = Vec::new();
        let mut keep: Vec<(NodeId, Offset, Offset)> = Vec::with_capacity(version.pending.len());
        for &(id, ps, pe) in &version.pending {
            if ps >= start && pe <= end {
                fitting.push(id);
            } else {
                keep.push((id, ps, pe));
            }
        }
        if !fitting.is_empty() {
            self.arena.adopt_sorted(parent, &fitting);
        }
        version.pending = keep;
    }

    pub(crate) fn recover(&mut self, at: usize, mut version: Version) -> Result<(), Fatal> {
        let state = self.hint(&version);
        let token = self.token_at(version.pos, state);
        if token.symbol == EOF_SYMBOL {
            version.dead = true;
            self.flush_skip(&mut version);
            self.put(at, version);
            return Ok(());
        }

        // First choice: insert what the state expected, and fork a skip so the
        // other reading of the error is explored too.
        if version.inserts_here() < self.config.max_inserts {
            let before = version.clone();
            let mut inserted: Vec<GssId> = Vec::new();
            for &top in &version.frontier {
                let state = self.gss.entry(top).state;
                let Some(expected) = table::expected(self.grammar, state) else {
                    continue;
                };
                let Some(terminal) = expected.terminal else {
                    continue;
                };
                let mut current = top;
                for step in &expected.steps {
                    let node = self.arena.missing(step.symbol, token.start);
                    current = self.gss.push(step.state, node, Some(current));
                }
                let from = self.gss.entry(current).state;
                if let Action::Shift(target) = table::action(self.grammar, from, terminal) {
                    let node = self.arena.missing(terminal, token.start);
                    inserted.push(self.gss.push(target, node, Some(current)));
                }
            }
            if !inserted.is_empty() {
                inserted.sort_unstable();
                inserted.dedup();
                let mut fork = None;
                if version.may_fork() {
                    version.note_fork();
                    let mut alternative = before.fork(Lane::Skip);
                    alternative.forked_at_set(version.pos);
                    fork = Some(alternative);
                }
                version.frontier = inserted;
                version.note_insert();
                version.cost += 1;
                self.stats.inserts += 1;
                self.put(at, version);
                if let Some(fork) = fork {
                    self.set.push_protected(fork, at);
                    self.stats.versions += 1;
                }
                return Ok(());
            }
        }

        // Second choice: skip. A balanced group is one cascade, not one token.
        let pre_skip = version.clone();
        let mut end = token.end;
        let mut next_pos = version.pos + 1;
        if let Some(closer) = self.opening(&token) {
            if let Some(close_at) = self.matching_close(version.pos, closer) {
                let close = self.token_at(close_at, state);
                end = close.end;
                next_pos = close_at + 1;
                self.stats.cascades += 1;
            }
        }
        version.skip = match version.skip {
            Some((start, _)) => Some((start, end)),
            None => Some((token.start, end)),
        };
        self.stats.skips += (next_pos - version.pos) as u32;
        version.pos = next_pos;
        version.cost += 1;
        version.reset_inserts();

        // A second reading of the same error: skip straight to the next token
        // this state can act on, and record the whole gap as one ERROR node.
        if version.may_fork() {
            if let Some(sync) = self.find_sync(version.pos, state) {
                if sync > version.pos + 1 {
                    version.note_fork();
                    let last = self.token_at(sync - 1, state);
                    let mut alternative = pre_skip.fork(Lane::Skip);
                    alternative.forked_at_set(pre_skip.pos);
                    alternative.skip = Some((pre_skip.skip.map(|(s, _)| s).unwrap_or(token.start), last.end));
                    alternative.pos = sync;
                    alternative.cost += 1;
                    alternative.reset_inserts();
                    self.stats.skips += (sync - pre_skip.pos) as u32;
                    self.set.push_protected(alternative, at);
                    self.stats.versions += 1;
                }
            }
        }
        self.put(at, version);
        Ok(())
    }

    /// The next token from `from` that `state` can act on, if any.
    fn find_sync(&mut self, from: usize, state: StateId) -> Option<usize> {
        let mut at = from;
        let horizon = from + 4096;
        while at < horizon {
            let token = self.token_at(at, state);
            if token.symbol == EOF_SYMBOL {
                return Some(at);
            }
            if table::action(self.grammar, state, token.symbol) != Action::None {
                return Some(at);
            }
            at += 1;
        }
        None
    }

    /// If this token opens a bracket, how it closes.
    fn opening(&self, token: &Token) -> Option<Closer> {
        for &(open, close) in self.config.pairs {
            if token.symbol == open {
                return Some(Closer::Symbol(close));
            }
        }
        if self.config.pairs.is_empty() {
            if let Some(byte) = self.byte_at(token.start) {
                match byte {
                    b'(' => return Some(Closer::Byte(b')')),
                    b'[' => return Some(Closer::Byte(b']')),
                    b'{' => return Some(Closer::Byte(b'}')),
                    _ => {}
                }
            }
        }
        None
    }

    fn byte_at(&self, offset: Offset) -> Option<u8> {
        self.config
            .source
            .and_then(|source| source.get(offset as usize).copied())
    }

    /// Index of the token that closes the group opening at `from`.
    ///
    /// Token-level when the lexer gave symbol pairs, byte-level otherwise. The
    /// scan is iterative and bounded by the token count.
    fn matching_close(&mut self, from: usize, closer: Closer) -> Option<usize> {
        match closer {
            Closer::Symbol(close) => {
                let open = self.token_at(from, 0).symbol;
                let mut depth = 1u32;
                let mut at = from + 1;
                while at < from + 4_000_000 {
                    let token = self.token_at(at, 0);
                    if token.symbol == EOF_SYMBOL {
                        return None;
                    }
                    if token.symbol == open {
                        depth += 1;
                    } else if token.symbol == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some(at);
                        }
                    }
                    at += 1;
                }
                None
            }
            Closer::Byte(close) => {
                let source = self.config.source?;
                let open = *source.get(self.token_at(from, 0).start as usize)?;
                let mut depth = 1u32;
                let mut at = self.token_at(from, 0).start as usize + 1;
                while at < source.len() {
                    let byte = source[at];
                    if byte == open {
                        depth += 1;
                    } else if byte == close {
                        depth -= 1;
                        if depth == 0 {
                            return self.token_ending_after(at as Offset, from);
                        }
                    }
                    at += 1;
                }
                None
            }
        }
    }

    /// The token that covers byte `offset`, searching from `from`.
    fn token_ending_after(&mut self, offset: Offset, from: usize) -> Option<usize> {
        let mut at = from + 1;
        loop {
            let token = self.token_at(at, 0);
            if token.symbol == EOF_SYMBOL {
                return None;
            }
            if token.start <= offset && offset < token.end {
                return Some(at);
            }
            if token.start > offset {
                return Some(at);
            }
            at += 1;
        }
    }
}
