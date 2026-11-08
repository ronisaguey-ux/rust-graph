//! Reducing a failing input to the smallest byte sequence that still fails.
//!
//! The algorithm is delta-debugging over byte ranges: delete a contiguous
//! chunk; if the failure survives, keep the deletion and search inside the
//! smaller input. Chunk sizes start at half the input and halve down to one
//! byte, which finds "the one insert that breaks it" without assuming anything
//! about the grammar.
//!
//! A hang costs the 2 s limit on every attempt that still hangs, so the
//! shrinker has a wall-clock cap of its own. When it expires the best input
//! found so far is returned, and the result says the cap stopped it; a smaller
//! input is not claimed if the search did not prove it.

use std::time::{Duration, Instant};

use crate::harness::{check, Failure, FuzzTarget};

/// The outcome of a shrink.
#[derive(Clone, Debug)]
pub struct ShrinkResult {
    /// The smallest failing input found.
    pub bytes: Vec<u8>,
    /// Why it fails, re-checked after the last reduction.
    pub failure: Option<Failure>,
    /// How many parses the search spent.
    pub attempts: usize,
    /// True when the attempt or time cap stopped the search before it was
    /// minimal.
    pub capped: bool,
    /// Bytes in the input handed in.
    pub original_len: usize,
}

/// Reduce `input` while `make()` builds a fresh target each attempt.
///
/// `budget` bounds the search in wall time; the default for a CLI use is
/// [`DEFAULT_SHRINK_TIME`]. Returns `None` when the input does not fail, since
/// there is nothing to reduce.
pub fn shrink<T, F>(make: F, input: &[u8], budget: Duration) -> Option<ShrinkResult>
where
    T: FuzzTarget,
    F: FnMut() -> T,
{
    shrink_with(make, input, budget, usize::MAX)
}

/// [`shrink`] with an explicit attempt cap, for tests.
pub fn shrink_with<T, F>(mut make: F, input: &[u8], budget: Duration, max_attempts: usize) -> Option<ShrinkResult>
where
    T: FuzzTarget,
    F: FnMut() -> T,
{
    let deadline = Instant::now() + budget;
    let mut attempts = 0usize;
    let mut capped = false;
    let mut fails = |bytes: &[u8], attempts: &mut usize| -> Option<Failure> {
        *attempts += 1;
        check(&mut make(), bytes).failure
    };

    let first = fails(input, &mut attempts)?;
    let mut current = input.to_vec();
    let mut current_failure = first;

    if !current.is_empty() {
        let mut granularity = 2usize;
        loop {
            if attempts >= max_attempts || Instant::now() >= deadline {
                capped = true;
                break;
            }
            let len = current.len();
            let chunk = len.div_ceil(granularity);
            let mut reduced = false;
            let mut start = 0usize;
            while start < len {
                if attempts >= max_attempts || Instant::now() >= deadline {
                    capped = true;
                    break;
                }
                let end = (start + chunk).min(len);
                let mut candidate = Vec::with_capacity(len - (end - start));
                candidate.extend_from_slice(&current[..start]);
                candidate.extend_from_slice(&current[end..]);
                if candidate.len() < current.len() {
                    if let Some(failure) = fails(&candidate, &mut attempts) {
                        current = candidate;
                        current_failure = failure;
                        reduced = true;
                        break;
                    }
                }
                start = end;
            }
            if capped {
                break;
            }
            if !reduced {
                if granularity >= current.len() {
                    break;
                }
                granularity = (granularity * 2).min(current.len());
            } else {
                granularity = 2.max(granularity.saturating_sub(1));
            }
        }
    }

    // Final byte pass: the chunk search can stop with adjacent equal-size
    // chunks that only fail together; deleting bytes one at a time from the
    // end catches the rest until no single deletion survives.
    if !capped {
        loop {
            if attempts >= max_attempts || Instant::now() >= deadline {
                capped = true;
                break;
            }
            let mut changed = false;
            for at in 0..current.len() {
                if attempts >= max_attempts || Instant::now() >= deadline {
                    capped = true;
                    break;
                }
                let mut candidate = current.clone();
                candidate.remove(at);
                if let Some(failure) = fails(&candidate, &mut attempts) {
                    current = candidate;
                    current_failure = failure;
                    changed = true;
                    break;
                }
            }
            if !changed {
                break;
            }
        }
    }

    Some(ShrinkResult { bytes: current, failure: Some(current_failure), attempts, capped, original_len: input.len() })
}

/// How long a `shrink` invocation searches before returning the best it has.
pub const DEFAULT_SHRINK_TIME: Duration = Duration::from_secs(120);

/// Render bytes so a terminal shows the exact sequence: printable ASCII as
/// itself, everything else as `\xNN`. Also gives the byte length.
pub fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for &byte in bytes {
        match byte {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7E => out.push(byte as char),
            _ => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use deep_grammars::deep::tree::Node;

    use crate::harness::Parsed;

    /// Fails when the input contains `CRASH`, with a tree that is otherwise
    /// valid, so the shrinker sees exactly one reason to fail.
    struct ContainsToken {
        nodes: Vec<Node>,
    }

    impl ContainsToken {
        fn new() -> ContainsToken {
            ContainsToken { nodes: vec![Node { kind: 16, flags: 0, start: 0, end: u32::MAX, subtree_end: 1 }] }
        }
    }

    impl FuzzTarget for ContainsToken {
        fn parse(&mut self, input: &[u8]) -> Parsed<'_> {
            self.nodes[0].end = input.len() as u32;
            if input.windows(5).any(|window| window == b"CRASH") {
                panic!("injected: CRASH present");
            }
            Parsed { nodes: &self.nodes, source_len: input.len() as u32, root_kind: 16, parse: Duration::ZERO }
        }
    }

    #[test]
    fn shrink_reduces_to_the_trigger() {
        let input = b"xxCRASHxxCRASHxxxxxxxxxxxxxxxx";
        let result = shrink_with(ContainsToken::new, input, Duration::from_secs(10), 10_000).expect("input fails");
        assert_eq!(result.bytes, b"CRASH");
        assert_eq!(result.original_len, input.len());
        assert!(!result.capped);
    }

    #[test]
    fn an_input_that_does_not_fail_is_not_shrunk() {
        assert!(shrink_with(ContainsToken::new, b"nothing here", Duration::from_secs(1), 100).is_none());
    }

    /// A real `Language` whose lexer panics on `Z`: the shrinker drives the
    /// actual workspace, lexer and parser, not a stand-in target.
    static BROKEN_LEXER: deep_grammars::deep::syntax::Language = deep_grammars::deep::syntax::Language::new(
        "broken-lexer",
        16,
        &[],
        broken_lex,
        broken_parse,
    );

    fn broken_lex(cursor: &mut deep_grammars::deep::lexer::Cursor) -> u16 {
        if cursor.peek() == b'Z' {
            panic!("injected lexer bug: Z is not a byte this lexer handles");
        }
        cursor.bump_char();
        deep_grammars::deep::syntax::WHITESPACE
    }

    fn broken_parse(_: &mut deep_grammars::deep::parser::Parser) {}

    #[test]
    fn shrink_drives_the_real_runtime_to_the_trigger_byte() {
        let input = b"aaaaaaaaaaaaaaaaaaaaaaaaZaZaaaaaaaaaaaaaaaa";
        let make = || {
            crate::harness::LanguageTarget::from_language(&BROKEN_LEXER, deep_grammars::deep::Budget::default())
        };
        let result = shrink_with(make, input, Duration::from_secs(10), 10_000).expect("the broken lexer fails");
        assert_eq!(result.bytes, b"Z");
        assert!(matches!(result.failure, Some(crate::harness::Failure::Panic { .. })), "{:?}", result.failure);
        assert!(!result.capped);
    }

    #[test]
    fn escape_is_lossless_for_the_report() {
        assert_eq!(escape(b"a\n\x00\xff\\"), "a\\n\\x00\\xff\\\\");
    }

    #[test]
    fn the_attempt_cap_stops_the_search() {
        let result = shrink_with(ContainsToken::new, b"CRASH-and-a-long-tail-of-bytes", Duration::from_secs(10), 3).unwrap();
        assert!(result.capped);
        assert!(result.failure.is_some());
    }
}
