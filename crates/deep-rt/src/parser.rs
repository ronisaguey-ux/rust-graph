//! The parser runtime that every grammar runs on: a cursor over final tokens, the
//! group firewall, events, markers, backtracking and the budget.
//!
//! Grammar rules are plain functions over `&mut Parser`. They never see trivia,
//! never see past the end of the group they are in, and cannot loop forever: once
//! the budget runs out every token reads as `EOF`, so every rule loop exits.

use crate::budget::{Budget, Exhausted, Meter, Outcome};
use crate::lexer::{lex, Tokens, MAX_INPUT};
use crate::pairs::{Pairs, NONE};
use crate::syntax::{Class, Kind, Language, EOF, ERROR, SKIPPED, TOMBSTONE};
use crate::tree::{Event, Tree, MISSING, UNCLOSED};

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

