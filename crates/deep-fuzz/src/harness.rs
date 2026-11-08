//! The harness: run one input through a target and decide whether it failed.
//!
//! The four checks are ordered by how much they trust the parse that produced
//! the result:
//!
//! 1. **panic** — a panic is caught so one bad input does not end the run, but
//!    it is still a failure. In a build with `panic = "abort"` it cannot be
//!    caught; the fuzzer says so at startup and the process aborts on the
//!    input, which is the same verdict with less evidence.
//! 2. **hang** — the parser's own [`Budget`] carries a 2 s wall clock, so a
//!    pathological input returns in about 2 s with `Exhausted::Time` instead of
//!    spinning forever. The harness also measures wall time around the call.
//! 3. **memory** — `VmHWM` (peak resident set) is read after the parse. RSS is
//!    process-wide, so this is the peak of the whole run; once it crosses the
//!    cap, the run is red for every later input too, which is the honest
//!    reading of "the process used more than the cap".
//! 4. **tree** — [`crate::treecheck::validate`] proves the arena is one
//!    preorder tree reachable from its root.
//!
//! Panics are caught with [`std::panic::catch_unwind`], so this binary must be
//! built with the default (unwinding) panic strategy. `cargo run -p deep-fuzz`
//! does that; a `--release` build inherits the workspace's `panic = "abort"`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use deep_grammars::deep::tree::Node;
use deep_grammars::deep::{Budget, Workspace};
use deep_grammars::deep::syntax::Language;

use crate::registry::Entry;
use crate::treecheck::{validate, TreeFault};

/// An input that takes longer than this is a hang. The parser's budget is set
/// to the same value so the check can finish and report.
pub const HANG_LIMIT: Duration = Duration::from_secs(2);
/// Peak resident set allowed for the fuzzing process.
pub const MEMORY_LIMIT: u64 = 512 * 1024 * 1024;

/// What a target hands back for one input: a view of the tree it built.
pub struct Parsed<'a> {
    pub nodes: &'a [Node],
    pub source_len: u32,
    pub root_kind: u16,
    /// Time the target's parse took, as measured by the target itself.
    pub parse: Duration,
}

/// A parser the harness can drive. The real implementation is
/// [`LanguageTarget`]; the tests use deliberately broken ones to prove the
/// harness can fail.
pub trait FuzzTarget {
    fn parse(&mut self, input: &[u8]) -> Parsed<'_>;
}

/// The real deep parser for one grammar, with a 2 s wall-clock budget.
pub struct LanguageTarget {
    language: &'static Language,
    budget: Budget,
    workspace: Workspace,
}

impl LanguageTarget {
    pub fn new(entry: &'static Entry) -> LanguageTarget {
        let budget = Budget { time: Some(HANG_LIMIT), ..Budget::default() };
        LanguageTarget { language: entry.language, budget, workspace: Workspace::default() }
    }

    pub fn with_budget(entry: &'static Entry, budget: Budget) -> LanguageTarget {
        LanguageTarget { language: entry.language, budget, workspace: Workspace::default() }
    }

    /// Drive an arbitrary [`Language`], which is what the "injected broken
    /// parser" test needs: a grammar static that panics on every input.
    pub fn from_language(language: &'static Language, budget: Budget) -> LanguageTarget {
        LanguageTarget { language, budget, workspace: Workspace::default() }
    }
}

impl FuzzTarget for LanguageTarget {
    fn parse(&mut self, input: &[u8]) -> Parsed<'_> {
        let started = Instant::now();
        let result = self.workspace.parse(input, self.language, &self.budget);
        let parse = started.elapsed();
        // A `TooLarge` input never reaches the lexer; it has no tree, which the
        // validator reports as `Empty`. `MAX_INPUT` is 4 GiB, so this is only
        // reachable if a caller hands the harness a 4 GiB slice.
        if result.is_err() {
            return Parsed { nodes: &[], source_len: input.len() as u32, root_kind: self.language.root, parse };
        }
        Parsed {
            nodes: &self.workspace.tree.nodes,
            source_len: input.len() as u32,
            root_kind: self.language.root,
            parse,
        }
    }
}

/// One way an input can fail the parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Panic { message: String },
    Hang { elapsed: Duration },
    Memory { peak: u64, limit: u64 },
    Unreachable { why: TreeFault },
}

impl std::fmt::Display for Failure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Panic { message } => write!(out, "panic: {message}"),
            Failure::Hang { elapsed } => write!(out, "hang: {} ms over the {} ms limit", elapsed.as_millis(), HANG_LIMIT.as_millis()),
            Failure::Memory { peak, limit } => write!(out, "memory: peak {peak} bytes over {limit}"),
            Failure::Unreachable { why } => write!(out, "unreachable tree: {why}"),
        }
    }
}

/// What one checked input cost.
#[derive(Clone, Debug)]
pub struct Report {
    pub failure: Option<Failure>,
    pub elapsed: Duration,
    /// Nodes in the tree, or 0 when the parse panicked or returned none.
    pub nodes: u32,
}

/// Check one input against a target.
pub fn check<T: FuzzTarget>(target: &mut T, input: &[u8]) -> Report {
    let started = Instant::now();
    // The closure owns the verdict because the parsed tree borrows `target`,
    // and a borrow cannot escape `catch_unwind`'s boxed closure.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let parsed = target.parse(input);
        let nodes = parsed.nodes.len() as u32;
        if parsed.parse >= HANG_LIMIT {
            return (Err(Failure::Hang { elapsed: parsed.parse }), nodes);
        }
        let verdict = validate(parsed.nodes, parsed.source_len, parsed.root_kind).map_err(|why| Failure::Unreachable { why });
        (verdict, nodes)
    }));
    let elapsed = started.elapsed();
    let peak = peak_rss_bytes().unwrap_or(0);
    match outcome {
        Err(payload) => Report { failure: Some(Failure::Panic { message: panic_message(payload) }), elapsed, nodes: 0 },
        Ok((verdict, nodes)) => {
            if elapsed >= HANG_LIMIT {
                return Report { failure: Some(Failure::Hang { elapsed }), elapsed, nodes };
            }
            if peak > MEMORY_LIMIT {
                return Report { failure: Some(Failure::Memory { peak, limit: MEMORY_LIMIT }), elapsed, nodes };
            }
            Report { failure: verdict.err(), elapsed, nodes }
        }
    }
}

/// Peak resident set of this process, in bytes, from `/proc/self/status`.
/// `None` on a platform without that file.
pub fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmHWM:") {
            let kb: u64 = value.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

static QUIET: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// Silence and record panic messages for the rest of the process.
///
/// The fuzzer reports panics as data; the default hook would also print a
/// backtrace to stderr for every attempt, including every attempt `shrink`
/// makes. The recorded messages are only for debugging the harness itself.
pub fn quiet_panics() {
    QUIET.get_or_init(|| {
        std::panic::set_hook(Box::new(|info| {
            if let Some(store) = QUIET.get() {
                if let Ok(mut messages) = store.lock() {
                    messages.push(info.to_string());
                    if messages.len() > 64 {
                        messages.remove(0);
                    }
                }
            }
        }));
        Mutex::new(Vec::new())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_grammars::deep::tree::{Node, TOKEN};

    fn node(kind: u16) -> Node {
        Node { kind, flags: 0, start: 0, end: 4, subtree_end: 1 }
    }

    /// A target that hands the harness a fixed tree, so a corrupt tree can be
    /// injected without touching the parser.
    struct Fixed {
        nodes: Vec<Node>,
        source: u32,
        root: u16,
    }

    impl FuzzTarget for Fixed {
        fn parse(&mut self, _input: &[u8]) -> Parsed<'_> {
            Parsed { nodes: &self.nodes, source_len: self.source, root_kind: self.root, parse: Duration::ZERO }
        }
    }

    /// A target that panics on the input `boom`, which is how a real parser
    /// panic looks to the harness, and otherwise returns a valid one-node tree.
    struct Fragile {
        nodes: Vec<Node>,
    }

    impl Fragile {
        fn new() -> Fragile {
            Fragile { nodes: vec![Node { kind: 16, flags: 0, start: 0, end: 0, subtree_end: 1 }] }
        }
    }

    impl FuzzTarget for Fragile {
        fn parse(&mut self, input: &[u8]) -> Parsed<'_> {
            self.nodes[0].end = input.len() as u32;
            if input == b"boom" {
                panic!("index out of bounds: the len is 3 but the index is 3");
            }
            Parsed { nodes: &self.nodes, source_len: input.len() as u32, root_kind: 16, parse: Duration::ZERO }
        }
    }

    /// A target that sleeps past the limit, standing in for a hang.
    struct Slow;

    impl FuzzTarget for Slow {
        fn parse(&mut self, input: &[u8]) -> Parsed<'_> {
            std::thread::sleep(HANG_LIMIT + Duration::from_millis(50));
            Parsed { nodes: &[], source_len: input.len() as u32, root_kind: 0, parse: HANG_LIMIT }
        }
    }

    /// A real `Language` whose parse function panics, driven through the real
    /// workspace and the real lexer: the harness must catch it exactly as it
    /// would catch a bug in a shipped grammar.
    static BROKEN: Language = Language::new("broken", 16, &[], broken_lex, broken_parse);

    fn broken_lex(cursor: &mut deep_grammars::deep::lexer::Cursor) -> u16 {
        let _ = cursor;
        deep_grammars::deep::syntax::EOF
    }

    fn broken_parse(_: &mut deep_grammars::deep::parser::Parser) {
        panic!("injected parser bug: unexpected token");
    }

    #[test]
    fn an_injected_broken_grammar_is_caught_through_the_real_runtime() {
        let mut target = LanguageTarget::from_language(&BROKEN, Budget::default());
        let report = check(&mut target, b"any input at all");
        match report.failure {
            Some(Failure::Panic { ref message }) => assert!(message.contains("injected parser bug"), "{message}"),
            other => panic!("expected a panic, got {other:?}"),
        }
    }

    #[test]
    fn a_panic_is_caught_and_reported() {
        let mut target = Fragile::new();
        let report = check(&mut target, b"boom");
        assert!(matches!(report.failure, Some(Failure::Panic { .. })), "{report:?}");
        // The harness survives and can check the next input.
        let report = check(&mut target, b"fine");
        assert_eq!(report.failure, None);
    }

    #[test]
    fn a_hang_is_reported() {
        let mut target = Slow;
        let report = check(&mut target, b"x");
        assert!(matches!(report.failure, Some(Failure::Hang { .. })), "{report:?}");
    }

    #[test]
    fn a_corrupt_tree_is_reported_as_unreachable() {
        let mut target = Fixed { nodes: vec![node(16)], source: 4, root: 17 };
        let report = check(&mut target, b"abcd");
        assert!(matches!(report.failure, Some(Failure::Unreachable { .. })), "{report:?}");

        // The same arena with the root kind fixed still has a token with no
        // TOKEN flag at index 0, which is fine, but a second orphan is not.
        let mut target = Fixed { nodes: vec![node(16), Node { flags: TOKEN, ..node(9) }], source: 4, root: 16 };
        let report = check(&mut target, b"abcd");
        assert!(matches!(report.failure, Some(Failure::Unreachable { .. })), "{report:?}");

        let mut target = Fixed { nodes: vec![node(16)], source: 4, root: 16 };
        assert_eq!(check(&mut target, b"abcd").failure, None);
    }
}
