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
    links: &'a mut Vec<Link>,
    meter: Meter,
    max_depth: u32,
    /// The next significant token, never past `limit`.
    pos: u32,
    /// The current group's closer, or the token count at top level. Exclusive.
    limit: u32,
    depth: u32,
    deepest: u32,
    skipped: u32,
    /// Errors recorded so far, so a predicate can tell whether a trial parse failed.
    errors: u32,
    /// A cache for [`Parser::line_end`]: the first `newline` at or after `.0` is at `.1`.
    line_end: (u32, u32),
    /// The last [`Parser::wrap`]: the operand start it wrapped, and the outermost
    /// wrapper made. The next wrap of the same start walks on from there, so a
    /// chain `a + b + c + ...` costs O(1) per operator, not O(length).
    wrapped: (u32, u32),
    /// In a [`Parser::trial`]: the error count it started with, else `NONE`.
    /// A trial's first error ends its input, so a failed predicate costs what it
    /// read up to the error, never a recovery that scans the rest of the group.
    trial_errors: u32,
}

/// A predicate's trial parse, ended by [`Parser::end_trial`].
#[must_use]
pub struct Trial {
    checkpoint: Checkpoint,
    limit: u32,
    outer: u32,
}

/// An open node. Close it with [`Parser::close`] or drop it with [`Parser::abandon`].
#[must_use]
pub struct Marker(u32);

/// A finished node, which [`Parser::precede`] can wrap.
#[derive(Clone, Copy)]
pub struct Closed(u32);

/// A forward-parent link: the start at `at` was preceded when the events were
/// `made_at` long. `made_at` only grows along the list, since a rewind drops
/// the links made after its checkpoint.
#[derive(Clone, Copy)]
struct Link {
    at: u32,
    made_at: u32,
}

/// A point to rewind to. Rewinding is O(1) plus the precede links it cuts.
#[derive(Clone, Copy)]
pub struct Checkpoint {
    events: u32,
    links: u32,
    pos: u32,
    errors: u32,
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
        self.errors += 1;
        let marker = self.open();
        self.events.push(Event::Flag(MISSING));
        self.close(marker, ERROR);
        self.stop_failed_trial();
    }

    /// In a trial that has failed, every token from here reads as `EOF`.
    #[inline]
    fn stop_failed_trial(&mut self) {
        if self.trial_errors != NONE && self.errors > self.trial_errors {
            self.limit = self.pos;
        }
    }

    /// Starts a trial parse for a predicate: run the rule, then
    /// [`Parser::end_trial`] says whether it parsed with no error and rewinds.
    pub fn trial(&mut self) -> Trial {
        let trial = Trial { checkpoint: self.checkpoint(), limit: self.limit, outer: self.trial_errors };
        self.trial_errors = self.errors;
        trial
    }

    /// Ends `trial`: true if nothing failed since it started. The input is as before.
    pub fn end_trial(&mut self, trial: Trial) -> bool {
        let parsed = self.errors == trial.checkpoint.errors;
        self.rewind(trial.checkpoint);
        self.limit = trial.limit;
        self.trial_errors = trial.outer;
        parsed
    }

    /// Wraps tokens in one `ERROR` until a token in `stop` or the end of the group.
    /// Always takes at least one token, so a loop calling it makes progress.
    /// Balanced groups inside are taken whole, so a `stop` token inside one does
    /// not end the error early.
    pub fn error_until(&mut self, stop: &[Kind]) {
        if self.at_end() {
            return;
        }
        self.errors += 1;
        if self.trial_errors != NONE {
            self.limit = self.pos;
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

    /// [`Parser::separator`], where a trailing `sep` is allowed (Python's
    /// `[1, 2,]`). `x %? sep` in `grammar!`.
    pub fn separator_trailing(&mut self, sep: Kind) -> bool {
        if self.at_end() {
            return false;
        }
        if !self.at(sep) {
            self.error_until(&[sep]);
            if self.at_end() {
                return false;
            }
        }
        self.bump();
        !self.at_end()
    }

    /// Errors recorded so far. A predicate compares it before and after a trial.
    pub fn errors(&self) -> u32 {
        self.errors
    }

    /// Where the next node would start in the events: remember it before an
    /// operand, and [`Parser::wrap`] the operand once an operator follows.
    pub fn event_count(&self) -> u32 {
        self.events.len() as u32
    }

    /// Opens a node around everything recorded since `start`, an
    /// [`Parser::event_count`] taken before one complete operand: how a Pratt
    /// loop turns `a` into `(a + b)`. A node there is preceded in O(1). A leaf
    /// (a token, or a few, as in `"a" "b"`) gets a start inserted before it,
    /// which moves only the operand's own events.
    pub fn wrap(&mut self, start: u32) -> Marker {
        let at = start as usize;
        if let Some(&Event::Start { kind, mut forward_parent }) = self.events.get(at) {
            if kind != TOMBSTONE {
                // Wrapped before (`f(x)[0]`): the outermost wrapper is the operand now.
                let mut outer = start;
                if self.wrapped.0 == start {
                    outer = self.wrapped.1;
                    let Event::Start { forward_parent: next, .. } = self.events[outer as usize] else { unreachable!("the cached wrapper is a start") };
                    forward_parent = next;
                }
                while forward_parent != 0 {
                    outer += forward_parent;
                    let Event::Start { forward_parent: next, .. } = self.events[outer as usize] else { unreachable!("a forward parent is a start") };
                    forward_parent = next;
                }
                let marker = self.precede(Closed(outer));
                self.wrapped = (start, marker.0);
                return marker;
            }
        }
        // Nothing past `start` is linked to anything before it: a Pratt loop
        // only ever wraps its own last operand.
        self.events.insert(at, Event::Start { kind: TOMBSTONE, forward_parent: 0 });
        if self.wrapped.0 != NONE && self.wrapped.0 >= start {
            self.wrapped = (NONE, NONE);
        } else if self.wrapped.1 != NONE && self.wrapped.1 >= start {
            self.wrapped.1 += 1;
        }
        // Only links made since `start` was taken can point past it, and those
        // are the tail of the list: the scan stops at the first older one, so
        // wrapping stays O(operand) however long the file.
        for link in self.links.iter_mut().rev() {
            if link.made_at <= start {
                break;
            }
            link.made_at += 1;
            if link.at >= start {
                link.at += 1;
            }
        }
        Marker(start)
    }

    /// The current token alone as an `ERROR`.
    pub fn error_one(&mut self) {
        if self.at_end() {
            return;
        }
        self.errors += 1;
        if self.trial_errors != NONE {
            self.limit = self.pos;
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
            self.links.push(Link { at: closed.0, made_at: self.events.len() as u32 });
        }
        marker
    }

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { events: self.events.len() as u32, links: self.links.len() as u32, pos: self.pos, errors: self.errors }
    }

    /// Forgets everything since `checkpoint`. Only valid within the group the
    /// checkpoint was taken in.
    pub fn rewind(&mut self, checkpoint: Checkpoint) {
        while self.links.len() as u32 > checkpoint.links {
            let at = self.links.pop().expect("length checked").at as usize;
            if at < checkpoint.events as usize {
                if let Event::Start { kind, .. } = self.events[at] {
                    self.events[at] = Event::Start { kind, forward_parent: 0 };
                }
            }
        }
        self.events.truncate(checkpoint.events as usize);
        if self.wrapped.1 != NONE && self.wrapped.1 >= checkpoint.events {
            self.wrapped = (NONE, NONE);
        }
        self.pos = checkpoint.pos;
        self.errors = checkpoint.errors;
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
        } else if let Some(layout) = self.language.layout {
            // Unclosed in a layout language: the lexer ended the bracket at a
            // line end, so the group ends there too.
            self.limit = self.line_end(layout.newline).min(outer);
        }
        self.depth += 1;
        self.deepest = self.deepest.max(self.depth);
        body(self);
        self.depth -= 1;
        self.take_rest();
        if self.trial_errors != NONE && self.errors > self.trial_errors {
            // A failed trial stops inside the group: the outer rules see `EOF` too.
            self.limit = self.pos;
            return Some(self.close(marker, kind));
        }
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
        self.errors += 1;
        if self.trial_errors != NONE {
            self.limit = self.pos;
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

    /// The first `newline` token at or after the current one, or the token count.
    /// Unclosed openers on one line ask for the same answer, so it is cached and
    /// the total cost stays linear.
    fn line_end(&mut self, newline: Kind) -> u32 {
        let (from, found) = self.line_end;
        if self.pos >= from && self.pos <= found && found != NONE {
            return found;
        }
        let mut at = self.pos;
        let count = self.kinds.len() as u32;
        while at < count && self.kinds[at as usize] != newline {
            at += 1;
        }
        self.line_end = (self.pos, at);
        at
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

/// How many parses had to grow the arena, process-wide. See
/// [`Workspace::allocations`].
pub static ALLOCATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Everything one parse allocates, kept between files so the steady state
/// allocates nothing.
#[derive(Default)]
pub struct Workspace {
    pub tokens: Tokens,
    pub pairs: Pairs,
    pub events: Vec<Event>,
    pub tree: Tree,
    links: Vec<Link>,
}

impl Workspace {
    /// How many times a parse had to grow an arena. The workspace is reused
    /// across files, so the steady state is zero: a corpus run that keeps growing
    /// is a corpus whose largest file keeps getting bigger, and a run that grows
    /// on every file is not reusing anything.
    pub fn allocations() -> u64 {
        ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The capacity of every buffer this workspace owns, summed: a parse that
    /// changes it allocated.
    fn capacity(&self) -> usize {
        self.tokens.kind.capacity()
            + self.tokens.end.capacity()
            + self.pairs.pair.capacity()
            + self.events.capacity()
            + self.links.capacity()
            + self.tree.nodes.capacity()
    }

    pub fn parse(&mut self, src: &[u8], language: &Language, budget: &Budget) -> Result<Outcome, TooLarge> {
        let capacity = self.capacity();
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
            errors: 0,
            line_end: (NONE, NONE),
            wrapped: (NONE, NONE),
            trial_errors: NONE,
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
        if self.capacity() != capacity {
            ALLOCATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
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
