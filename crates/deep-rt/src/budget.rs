//! One budget per file: parser steps, nesting depth and wall clock. Running out of
//! any of them degrades the file and never panics.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// Steps allowed per token, on top of `base_steps`. A step is one look at the
    /// current token, so a correct grammar uses a small constant per token.
    pub steps_per_token: u64,
    pub base_steps: u64,
    /// Nesting depth. A group opened past this is jumped whole as one `SKIPPED` leaf.
    pub max_depth: u32,
    /// Wall clock for the parse, checked every [`CLOCK_EVERY`] steps.
    pub time: Option<Duration>,
}

/// Measured: JSON uses 80 bytes of stack per nesting level in a release build,
/// plus about 7 KiB (the deep-scan `probe --stack-kb` bisection: 85 KiB at 1,000
/// levels, 319 KiB at 4,000, 1,257 KiB at 16,000). 4,096 levels covers the deepest
/// real file found in the prior-art survey (Linux `intel-ucode-defs.h`, 3,333) in
/// about 330 KiB. Heavier grammars use more per level; Slice 3 measures Python's.
/// Past the limit a group becomes one flagged leaf, never a stack overflow.
pub const DEFAULT_MAX_DEPTH: u32 = 4096;

impl Default for Budget {
    fn default() -> Self {
        Budget { steps_per_token: 64, base_steps: 4096, max_depth: DEFAULT_MAX_DEPTH, time: Some(Duration::from_secs(10)) }
    }
}

/// The wall clock is read once per this many steps.
pub const CLOCK_EVERY: u64 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exhausted {
    Steps,
    Time,
}

/// How a parse went against its budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub tokens: u32,
    pub steps: u64,
    pub deepest: u32,
    /// Groups jumped whole because they opened past `max_depth`.
    pub skipped_groups: u32,
    /// Set if the step or time budget ran out. The tokens left became one `ERROR` leaf.
    pub exhausted: Option<Exhausted>,
    pub unclosed: u32,
    pub stray: u32,
}

impl Outcome {
    /// The file parsed in full: no budget ran out and nothing was skipped.
    pub fn complete(&self) -> bool {
        self.exhausted.is_none() && self.skipped_groups == 0
    }
}

pub(crate) struct Meter {
    steps: u64,
    limit: u64,
    next_clock: u64,
    deadline: Option<Instant>,
    pub exhausted: Option<Exhausted>,
}

impl Meter {
    pub fn new(budget: &Budget, tokens: usize) -> Self {
        let limit = budget.base_steps.saturating_add(budget.steps_per_token.saturating_mul(tokens as u64));
        let deadline = budget.time.and_then(|time| Instant::now().checked_add(time));
        let mut meter = Meter { steps: 0, limit, next_clock: CLOCK_EVERY, deadline, exhausted: None };
        if budget.time == Some(Duration::ZERO) {
            meter.exhausted = Some(Exhausted::Time);
        }
        meter
    }

    /// Counts one step. False once any budget has run out; it stays false.
    #[inline]
    pub fn step(&mut self) -> bool {
        if self.exhausted.is_some() {
            return false;
        }
        self.steps += 1;
        if self.steps > self.limit {
            self.exhausted = Some(Exhausted::Steps);
            return false;
        }
        if self.steps >= self.next_clock {
            self.next_clock += CLOCK_EVERY;
            if self.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                self.exhausted = Some(Exhausted::Time);
                return false;
            }
        }
        true
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }
}
