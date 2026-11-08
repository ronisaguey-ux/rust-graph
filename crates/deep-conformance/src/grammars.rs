//! The registry populated with the grammars that exist in this worktree.
//!
//! This is the seam, and it is deliberately small: when a language crate lands,
//! its owner adds one `measure_<language>` wrapper and one `set_parser` line.
//! Nothing in the scanner, report, summary, gates or diff changes.
//!
//! The adapter is feature-gated (`--features deep-grammars`) because the
//! grammar crate is being replaced in parallel; the conformance tool itself
//! must build and test with no parser present.

use deep_grammars::deep::budget::Budget;
use deep_grammars::deep::parser::Workspace;
use deep_grammars::deep::syntax::{Language, ERROR};
use deep_grammars::javascript::{JAVASCRIPT, TSX, TYPESCRIPT};
use deep_grammars::python::PYTHON;

use crate::registry::{ParseFacts, Registry};

fn measure(source: &[u8], language: &Language) -> ParseFacts {
    let mut workspace = Workspace::default();
    match workspace.parse(source, language, &Budget::default()) {
        Err(_) => ParseFacts::fatal("input exceeds the parser's 32-bit offset limit"),
        Ok(_outcome) => {
            let nodes = &workspace.tree.nodes;
            // Count every ERROR node, but add a byte range once: an ERROR nested
            // inside another is already covered by its ancestor.
            let error_nodes = nodes.iter().filter(|node| node.kind == ERROR).count() as u64;
            let mut error_bytes = 0u64;
            let mut at = 0usize;
            while at < nodes.len() {
                let node = &nodes[at];
                if node.kind == ERROR {
                    error_bytes += node.end.saturating_sub(node.start) as u64;
                    at = (node.subtree_end as usize).max(at + 1);
                } else {
                    at += 1;
                }
            }
            ParseFacts {
                nodes: nodes.len() as u64,
                max_depth: workspace.tree.depth(),
                error_nodes,
                error_bytes,
                fatal: None,
            }
        }
    }
}

fn python(source: &[u8]) -> ParseFacts {
    measure(source, &PYTHON)
}

fn javascript(source: &[u8]) -> ParseFacts {
    measure(source, &JAVASCRIPT)
}

fn typescript(source: &[u8]) -> ParseFacts {
    measure(source, &TYPESCRIPT)
}

fn tsx(source: &[u8]) -> ParseFacts {
    measure(source, &TSX)
}

/// Attach the parsers this build has. Called by [`crate::registry::builtin`].
pub fn register(registry: &mut Registry) {
    assert!(registry.set_parser("Python", python), "the registry lost Python");
    assert!(registry.set_parser("JavaScript", javascript), "the registry lost JavaScript");
    assert!(registry.set_parser("TypeScript", typescript), "the registry lost TypeScript");
    // `.tsx` is the same language row but the JSX lexer, so it gets its own
    // parser. `.ts`, `.mts` and `.cts` keep the plain TypeScript lexer.
    assert!(registry.set_extension_parser("TypeScript", "tsx", tsx), "TypeScript lost .tsx");
}
