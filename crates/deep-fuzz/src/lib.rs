//! deep-fuzz: a deterministic mutational fuzzer, shrinker and tree checker for
//! the deep CST parser.
//!
//! The fuzzer does not look for semantic errors. It asserts four things about
//! every input it tries:
//!
//! 1. the parse does not panic;
//! 2. it finishes inside its wall-clock budget (2 s);
//! 3. process peak RSS stays under 512 MB;
//! 4. the tree it produces is a single preorder arena reachable from its root,
//!    with `subtree_end` bounds that are consistent.
//!
//! A parser that never produces any of those four is not proved correct; it is
//! proved not to have failed on the inputs tried. [`mutate`] is a pure function
//! of a seed and a PRNG state, so a run names the inputs it tried by
//! `(seed, iteration)`, not by saving them.
//!
//! ```text
//! seeds (fixtures + corpus/) -> mutate -> Workspace::parse -> tree check
//!                 ^                              |
//!                 +------- shrink on failure ----+
//! ```
//!
//! Everything here is std only and links the grammar crate directly, which is
//! what a user of the parser links.

pub mod corpus;
pub mod harness;
pub mod mutate;
pub mod registry;
pub mod shrink;
pub mod treecheck;

pub use harness::{check, Failure, FuzzTarget, LanguageTarget, Report, HANG_LIMIT, MEMORY_LIMIT};
pub use mutate::{mutate, Rng};
pub use registry::{by_extension, by_name, fixtures_root, seeds_for, Entry, ENTRIES};
pub use shrink::{shrink, ShrinkResult};
pub use treecheck::{validate, TreeFault};
