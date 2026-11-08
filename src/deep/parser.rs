//! The parser runtime that every grammar runs on: a cursor over final tokens, the
//! group firewall, events, markers, backtracking and the budget.
//!
//! Grammar rules are plain functions over `&mut Parser`. They never see trivia,
//! never see past the end of the group they are in, and cannot loop forever: once
//! the budget runs out every token reads as `EOF`, so every rule loop exits.

use super::budget::{Budget, Exhausted, Meter, Outcome};
use super::lexer::{lex, Tokens, MAX_INPUT};
use super::pairs::{Pairs, NONE};
use super::syntax::{Class, Kind, Language, EOF, ERROR, SKIPPED, TOMBSTONE};
use super::tree::{Event, Tree, MISSING, UNCLOSED};

pub struct Parser<'a> {
    language: &'a Language,
    kinds: &'a [Kind],
    pair: &'a [u32],
    events: &'a mut Vec<Event>,
    /// Events whose `forward_parent` is set, so a rewind can unset the ones it cuts.
    links: &'a mut Vec<u32>,
    meter: Meter,
    max_depth: u32,
    /// The next significant token, never past `limit`.
    pos: u32,
    /// The current group's closer, or the token count at top level. Exclusive.
    limit: u32,
    depth: u32,
    deepest: u32,
    skipped: u32,
}

/// An open node. Close it with [`Parser::close`] or drop it with [`Parser::abandon`].
#[must_use]
pub struct Marker(u32);

/// A finished node, which [`Parser::precede`] can wrap.
#[derive(Clone, Copy)]
pub struct Closed(u32);

/// A point to rewind to. Rewinding is O(1) plus the precede links it cuts.
#[derive(Clone, Copy)]
pub struct Checkpoint {
    events: u32,
    links: u32,
    pos: u32,
}

impl<'a> Parser<'a> {
    /// Kind of the current token, or `EOF` at the end of the group or the budget.
    #[inline]
    pub fn current(&mut self) -> Kind {
        if !self.meter.step() || self.pos >= self.limit {
            return EOF;
        }
        self.kinds[self.pos as usize]
    }

    /// Kind of the significant token `ahead` places on, within the group.
    pub fn nth(&mut self, ahead: u32) -> Kind {
        if !self.meter.step() {
            return EOF;
        }
        let mut at = self.pos;
        for _ in 0..ahead {
            if at >= self.limit {
                return EOF;
            }
            at = self.skip_trivia(at + 1);
        }
        if at >= self.limit { EOF } else { self.kinds[at as usize] }
    }

    #[inline]
    pub fn at(&mut self, kind: Kind) -> bool {
        self.current() == kind
    }

    pub fn at_any(&mut self, kinds: &[Kind]) -> bool {
        let current = self.current();
        current != EOF && kinds.contains(&current)
    }

    #[inline]
    pub fn at_end(&mut self) -> bool {
        self.current() == EOF
    }

    /// Index of the current token, for diagnostics and tests.
    pub fn position(&self) -> u32 {
        self.pos
    }

    /// Adds the current token to the open node and moves past it and its trivia.
    pub fn bump(&mut self) {
        if self.pos >= self.limit {
            return;
        }
        self.events.push(Event::Token(self.pos));
        self.pos = self.skip_trivia(self.pos + 1);
    }

    pub fn eat(&mut self, kind: Kind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Eats `kind`, or records an empty `ERROR` flagged `MISSING` where it should be.
    pub fn expect(&mut self, kind: Kind) -> bool {
        if self.eat(kind) {
            return true;
        }
        self.missing();
        false
    }

    pub fn missing(&mut self) {
        let marker = self.open();
        self.events.push(Event::Flag(MISSING));
        self.close(marker, ERROR);
    }

    /// Wraps tokens in one `ERROR` until a token in `stop` or the end of the group.
    /// Always takes at least one token, so a loop calling it makes progress.
    /// Balanced groups inside are taken whole, so a `stop` token inside one does
    /// not end the error early.
    pub fn error_until(&mut self, stop: &[Kind]) {
        if self.at_end() {
            return;
        }
        let marker = self.open();
        loop {
            self.bump_balanced();
            if self.at_end() || self.at_any(stop) {
                break;
            }
        }
        self.close(marker, ERROR);
    }

    /// After a list item: a `sep` and more to come, or the end. Anything else up to
    /// the next `sep` becomes one `ERROR` (`[1, 2 3, 4]` makes `3` one), and so does
    /// a trailing `sep`. False when the list is done. `x % sep` in `grammar!`.
    pub fn separator(&mut self, sep: Kind) -> bool {
        if self.at_end() {
            return false;
        }
        if !self.at(sep) {
            self.error_until(&[sep]);
            if self.at_end() {
                return false;
            }
        }
        if self.nth(1) == EOF {
            self.error_one();
            return false;
        }
        self.bump();
        true
    }

    /// The current token alone as an `ERROR`.
    pub fn error_one(&mut self) {
        if self.at_end() {
            return;
        }
        let marker = self.open();
        self.bump_balanced();
        self.close(marker, ERROR);
    }

    /// One token, or a whole balanced group if the token opens one.
    fn bump_balanced(&mut self) {
        let close = self.partner();
        if close == NONE {
            self.bump();
            return;
        }
        while self.pos <= close && self.pos < self.limit {
            self.bump();
        }
    }

    /// The closer paired with the current token if it is an opener, else `NONE`.
    fn partner(&self) -> u32 {
        if self.pos >= self.limit {
            return NONE;
        }
        match self.language.class(self.kinds[self.pos as usize]) {
            Class::Open(_) => self.pair[self.pos as usize],
            _ => NONE,
        }
    }

    pub fn open(&mut self) -> Marker {
        let at = self.events.len() as u32;
        self.events.push(Event::Start { kind: TOMBSTONE, forward_parent: 0 });
        Marker(at)
    }

    pub fn close(&mut self, marker: Marker, kind: Kind) -> Closed {
        if let Event::Start { forward_parent, .. } = self.events[marker.0 as usize] {
            self.events[marker.0 as usize] = Event::Start { kind, forward_parent };
        }
        self.events.push(Event::Finish);
        Closed(marker.0)
    }

    pub fn abandon(&mut self, marker: Marker) {
        if marker.0 as usize + 1 == self.events.len() {
            self.events.pop();
        } else {
            self.events[marker.0 as usize] = Event::Start { kind: TOMBSTONE, forward_parent: 0 };
        }
    }

    /// Opens a node that becomes the parent of `closed`, without moving any event:
    /// how a Pratt loop turns `a` into `(a + b)` in O(1).
    pub fn precede(&mut self, closed: Closed) -> Marker {
        let marker = self.open();
        if let Event::Start { kind, .. } = self.events[closed.0 as usize] {
            self.events[closed.0 as usize] = Event::Start { kind, forward_parent: marker.0 - closed.0 };
            self.links.push(closed.0);
        }
        marker
    }

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { events: self.events.len() as u32, links: self.links.len() as u32, pos: self.pos }
    }

    /// Forgets everything since `checkpoint`. Only valid within the group the
    /// checkpoint was taken in.
    pub fn rewind(&mut self, checkpoint: Checkpoint) {
        while self.links.len() as u32 > checkpoint.links {
            let at = self.links.pop().expect("length checked") as usize;
            if at < checkpoint.events as usize {
                if let Event::Start { kind, .. } = self.events[at] {
                    self.events[at] = Event::Start { kind, forward_parent: 0 };
                }
            }
        }
        self.events.truncate(checkpoint.events as usize);
        self.pos = checkpoint.pos;
    }

    /// Parses the balanced group at the current opener as a `kind` node, with `body`
    /// bound to the group: the body sees `EOF` at the closer, so no error inside
    /// can reach outside. That is the delimiter firewall. An opener with no closer
    /// runs to the end of the enclosing group and is flagged `UNCLOSED`. Past
    /// `max_depth` the group becomes one `SKIPPED` leaf, in O(1).
    pub fn group(&mut self, kind: Kind, body: impl FnOnce(&mut Self)) -> Option<Closed> {
        if self.at_end() {
            return None;
        }
        debug_assert!(matches!(self.language.class(self.kinds[self.pos as usize]), Class::Open(_)), "group() needs an opener");
        let close = self.partner();
        if self.depth >= self.max_depth {
            let last = if close == NONE { self.last_significant_before(self.limit) } else { close };
            self.events.push(Event::Span { kind: SKIPPED, first: self.pos, last });
            self.skipped += 1;
            self.pos = if close == NONE { self.limit } else { self.skip_trivia(close + 1) };
            return None;
        }
        let marker = self.open();
        self.bump();
        let outer = self.limit;
        if close != NONE {
            self.limit = close;
        }
        self.depth += 1;
        self.deepest = self.deepest.max(self.depth);
        body(self);
        self.depth -= 1;
        self.take_rest();
        self.limit = outer;
        if close == NONE {
            // The closer is missing, as a token would be: an error the walkers see.
            self.missing();
            self.events.push(Event::Flag(UNCLOSED));
        } else {
            debug_assert_eq!(self.pos, close);
            self.bump();
        }
        Some(self.close(marker, kind))
    }

    /// Runs `rule` one level deeper, for recursion that does not open a group
    /// (prefix operators, nested statements). Past `max_depth` the rest of the
    /// group becomes one `SKIPPED` leaf and `rule` does not run.
    pub fn nested<R>(&mut self, rule: impl FnOnce(&mut Self) -> R) -> Option<R> {
        if self.depth >= self.max_depth {
            if self.pos < self.limit {
                let last = self.last_significant_before(self.limit);
                self.events.push(Event::Span { kind: SKIPPED, first: self.pos, last });
                self.skipped += 1;
                self.pos = self.limit;
            }
            return None;
        }
        self.depth += 1;
        self.deepest = self.deepest.max(self.depth);
        let result = rule(self);
        self.depth -= 1;
        Some(result)
    }

    pub fn exhausted(&self) -> bool {
        self.meter.exhausted.is_some()
    }

    /// Tokens a body left before its limit. After a budget runs out they become one
    /// `ERROR` leaf in O(1). Otherwise the grammar stopped early, and they are
    /// wrapped in an `ERROR` token by token.
    fn take_rest(&mut self) {
        if self.pos >= self.limit {
            return;
        }
        if self.exhausted() {
            let last = self.last_significant_before(self.limit);
            self.events.push(Event::Span { kind: ERROR, first: self.pos, last });
            self.pos = self.limit;
            return;
        }
        let marker = self.open();
        while self.pos < self.limit {
            self.bump();
        }
        self.close(marker, ERROR);
    }

    #[inline]
    fn skip_trivia(&self, mut at: u32) -> u32 {
        let end = self.kinds.len() as u32;
        while at < end && self.language.is_trivia(self.kinds[at as usize]) {
            at += 1;
        }
        // Never past `limit`: the token at a group's limit is its closer, which is
        // not trivia, and the top-level limit is the token count.
        at
    }

    /// The last non-trivia token before `limit`; `pos` itself is one, and is before it.
    fn last_significant_before(&self, limit: u32) -> u32 {
        let mut at = limit - 1;
        while at > self.pos && self.language.is_trivia(self.kinds[at as usize]) {
            at -= 1;
        }
        at
    }
}

/// The input does not fit `u32` offsets.
#[derive(Debug, PartialEq, Eq)]
pub struct TooLarge;

/// Everything one parse allocates, kept between files so the steady state
/// allocates nothing.
#[derive(Default)]
pub struct Workspace {
    pub tokens: Tokens,
    pub pairs: Pairs,
    pub events: Vec<Event>,
    pub tree: Tree,
    links: Vec<u32>,
}

impl Workspace {
    pub fn parse(&mut self, src: &[u8], language: &Language, budget: &Budget) -> Result<Outcome, TooLarge> {
        if src.len() > MAX_INPUT {
            return Err(TooLarge);
        }
        lex(src, language, &mut self.tokens);
        self.pairs.build(&self.tokens, language);
        self.events.clear();
        self.links.clear();
        let count = self.tokens.len() as u32;
        let mut parser = Parser {
            language,
            kinds: &self.tokens.kind,
            pair: &self.pairs.pair,
            events: &mut self.events,
            links: &mut self.links,
            meter: Meter::new(budget, self.tokens.len()),
            max_depth: budget.max_depth,
            pos: 0,
            limit: count,
            depth: 0,
            deepest: 0,
            skipped: 0,
        };
        parser.pos = parser.skip_trivia(0);
        let root = parser.open();
        (language.parse)(&mut parser);
        parser.take_rest();
        parser.close(root, language.root);
        let outcome = Outcome {
            tokens: count,
            steps: parser.meter.steps(),
            deepest: parser.deepest,
            skipped_groups: parser.skipped,
            exhausted: parser.meter.exhausted,
            unclosed: self.pairs.unclosed,
            stray: self.pairs.stray,
        };
        self.tree.build(&mut self.events, &self.tokens, language, src.len() as u32);
        Ok(outcome)
    }
}

impl Outcome {
    pub fn describe(&self) -> String {
        let exhausted = match self.exhausted {
            None => "none",
            Some(Exhausted::Steps) => "steps",
            Some(Exhausted::Time) => "time",
        };
        format!(
            "tokens={} steps={} deepest={} skipped_groups={} exhausted={} unclosed={} stray={}",
            self.tokens, self.steps, self.deepest, self.skipped_groups, exhausted, self.unclosed, self.stray
        )
    }
}
