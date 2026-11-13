//! The bridge rust-graph uses when the `deep` feature is on.
//!
//! One entry point, [`parse_file`], turning source bytes and a [`Lang`] into the
//! contract's concrete syntax tree. With the feature off there is no runtime to
//! call, [`parse_file`] returns `None`, and nothing here is compiled at all:
//! `src/main.rs` gains exactly two lines when the runtime lands,
//!
//! ```text
//! #[cfg(feature = "deep")] mod deep;
//! #[cfg(feature = "deep")] mod deep_integration;
//! ```
//!
//! and `src/main.rs` is not this field's file, so the wiring is written down
//! here rather than applied. Until those two lines exist the default build is
//! byte for byte today's binary; `crates/deep-bench/README.md` records the
//! artifact-hash check that proves it.
//!
//! The bridge does not reach into any parser internals: it calls
//! `Workspace::parse`, the same entry point `rust-graph-deep-scan` uses, and
//! converts the preorder arena into the sibling-linked node layout the contract
//! names, so the rest of rust-graph never sees the runtime's `subtree_end`.

// Only reachable with the feature on: the default build has no runtime module.
#[cfg(feature = "deep")]
use crate::deep;
#[cfg(feature = "deep")]
use crate::deep::syntax::Language;

/// An index into [`Cst::nodes`]. Never a pointer: the tree is one `Vec`.
pub type NodeId = u32;
/// No parent / no child sentinel, as in the contract.
pub const NONE: NodeId = NodeId::MAX;

/// Flags a node may carry, matching the contract's bit positions.
pub const FLAG_ERROR: u16 = 1 << 0;
pub const FLAG_MISSING: u16 = 1 << 1;

/// One node of the tree: 16 bytes, `#[repr(C)]`, byte range into the source.
///
/// Field for field this is `deep_abi::Node`. The contract crate is `#![no_std]`
/// and does not compile yet (`Vec` is not in scope there), so this mirrors the
/// shape instead of importing it; the day the contract crate builds, both
/// definitions become `pub use deep_abi::{Cst, Node, NodeId};` with no other
/// change in this file. `size_of` is asserted below so drift cannot slip in.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: u16,
    pub flags: u16,
    pub start: u32,
    pub end: u32,
    pub first_child: NodeId,
    pub next_sibling: NodeId,
}

/// A finished parse: the arena, the root and how many nodes are `ERROR`.
impl Cst {
    /// Recovery has a budget: a tree that needed an error node for more than one
    /// node in a hundred is a tree the grammar did not understand, not a tree
    /// with a mistake in it. Extraction refuses it and the pipeline emits the
    /// lexer's record instead, so a guess is never published as a fact.
    pub fn degraded(&self) -> bool {
        self.errors > 0 && (self.errors as f64) / (self.nodes.len().max(1) as f64) > 0.01
    }
}

pub struct Cst {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub errors: u32,
}

/// The languages the runtime knows, named the way rust-graph's callers name
/// them. Adding one is a variant plus a line in [`Lang::language`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Json,
    Python,
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    Java,
    C,
    /// C++, whose `Lang::tag` spelling is the lexer's `CPP`.
    Cpp,
}

impl Lang {
    /// The runtime's grammar for this language.
    #[cfg(feature = "deep")]
    pub fn language(self) -> &'static Language {
        match self {
            Lang::Json => &deep::json::JSON,
            Lang::Python => &deep::python::PYTHON,
            Lang::Rust => &deep::rust::RUST,
            Lang::JavaScript => &deep::javascript::JAVASCRIPT,
            Lang::TypeScript => &deep::javascript::TYPESCRIPT,
            Lang::Tsx => &deep::javascript::TSX,
            Lang::Go => &deep::go::GO,
            Lang::Java => &deep::java::JAVA,
            Lang::C => &deep::batch_c::c::C,
            Lang::Cpp => &deep::batch_c::cpp::CPP,
        }
    }
}

/// Parse `bytes` as `lang`, or `None` when there is no deep parser to parse
/// with — the feature is off, or the input is past the runtime's 4 GiB limit.
/// A tree that contains `ERROR` nodes is a `Some`: recovery is a result, not a
/// failure.
pub fn parse_file(bytes: &[u8], lang: Lang) -> Option<Cst> {
    imp::parse(bytes, lang)
}

#[cfg(feature = "deep")]
mod imp {
    use super::{Cst, Lang, Node, NodeId, FLAG_ERROR, FLAG_MISSING, NONE};
    use crate::deep;
    use crate::deep::syntax::ERROR;
    use crate::deep::tree::{HAS_ERROR, MISSING};

    /// Parse on a fresh [`deep::Workspace`], then flatten the arena.
    ///
    /// The runtime keeps a preorder `Vec` where `subtree_end` is the index one
    /// past a node's last descendant; the contract wants `first_child` and
    /// `next_sibling` instead. Both are index arithmetic — no copy of the tree
    /// and no second pass over the bytes.
    pub(super) fn parse(bytes: &[u8], lang: Lang) -> Option<Cst> {
        let mut workspace = deep::Workspace::default();
        let budget = deep::Budget::default();
        workspace.parse(bytes, lang.language(), &budget).ok()?;
        Some(flatten(&workspace.tree))
    }

    fn flatten(tree: &deep::tree::Tree) -> Cst {
        let count = tree.nodes.len();
        // Where each node's parent's subtree ends, so a node knows whether the
        // index after its own subtree is a sibling or an uncle. The root is
        // pushed first and its parent ends past the arena.
        let mut parent_end = vec![count; count];
        let mut open: Vec<usize> = Vec::new();
        for index in 0..count {
            while open.last().is_some_and(|&parent| tree.nodes[parent].subtree_end as usize <= index) {
                open.pop();
            }
            if let Some(&parent) = open.last() {
                parent_end[index] = tree.nodes[parent].subtree_end as usize;
            }
            open.push(index);
        }

        let mut nodes = Vec::with_capacity(count);
        let mut errors = 0u32;
        for (index, node) in tree.nodes.iter().enumerate() {
            let end = node.subtree_end as usize;
            let mut flags = 0;
            if node.kind == ERROR || node.flags & HAS_ERROR != 0 {
                flags |= FLAG_ERROR;
            }
            if node.flags & MISSING != 0 {
                flags |= FLAG_MISSING;
            }
            if node.kind == ERROR {
                errors += 1;
            }
            nodes.push(Node {
                kind: node.kind,
                flags,
                start: node.start,
                end: node.end,
                first_child: if end > index + 1 { (index + 1) as NodeId } else { NONE },
                next_sibling: if end < parent_end[index] { end as NodeId } else { NONE },
            });
        }
        Cst { nodes, root: 0, errors }
    }
}

#[cfg(not(feature = "deep"))]
mod imp {
    use super::{Cst, Lang};

    /// The feature is off: there is no parser, and pretending otherwise would
    /// make the default binary a different program.
    pub(super) fn parse(_bytes: &[u8], _lang: Lang) -> Option<Cst> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feature_off_build_has_no_parser_but_still_has_the_signature() {
        // A Cst must be constructible in both builds: the type is the contract,
        // the value is what changes.
        let cst = Cst { nodes: Vec::new(), root: 0, errors: 0 };
        assert!(cst.nodes.is_empty());

        #[cfg(not(feature = "deep"))]
        assert!(parse_file(b"{\"a\": 1}", Lang::Json).is_none());
    }

    #[test]
    fn the_node_layout_is_the_contract_layout() {
        // The contract's prose says "16 bytes", but its own field list
        // (2 + 2 + 4 + 4 + 4 + 4) can only be 20; this follows the fields.
        // Reported, not silently "fixed": deep-abi is not this field's file.
        assert_eq!(std::mem::size_of::<Node>(), 20);
        assert_eq!(std::mem::align_of::<Node>(), 4);
        assert_eq!(NONE, u32::MAX);
        assert_eq!((FLAG_ERROR, FLAG_MISSING), (1, 2));
    }

    #[cfg(feature = "deep")]
    #[test]
    fn the_deep_build_parses_json_into_the_contract_shape() {
        let cst = parse_file(b"{\"a\": [1, true, null]}", Lang::Json).expect("json parses");
        assert_eq!(cst.root, 0);
        assert!(cst.nodes.len() > 3, "{} nodes", cst.nodes.len());
        // Every node must be reachable from the root by first_child/next_sibling.
        let mut seen = vec![false; cst.nodes.len()];
        let mut pending = vec![cst.root];
        let mut visited = 0;
        while let Some(index) = pending.pop() {
            assert!(!seen[index as usize], "node {index} has two parents");
            seen[index as usize] = true;
            visited += 1;
            let node = cst.nodes[index as usize];
            if node.next_sibling != NONE {
                pending.push(node.next_sibling);
            }
            if node.first_child != NONE {
                pending.push(node.first_child);
            }
        }
        assert_eq!(visited, cst.nodes.len(), "unreachable nodes in the arena");
        // The root spans the whole input and is the only node that does.
        assert_eq!(cst.nodes[0].start, 0);
        assert_eq!(cst.nodes[0].end as usize, 22);
        assert_eq!(cst.nodes[0].next_sibling, NONE);
    }
}
