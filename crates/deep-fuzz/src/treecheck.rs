//! The tree checker: every node must be reachable from the root, exactly once.
//!
//! The arena is a preorder `Vec<Node>`: a node's descendants are the indices
//! `index + 1 .. subtree_end`. That layout is only a tree if every
//! `subtree_end` points forward, stays inside its parent's range, and the
//! siblings tile the parent's range with no gap and no overlap. If any of that
//! fails, a node exists in the vector that no walk from the root will reach —
//! the failure this checker is named for.
//!
//! It walks the arena iteratively: a depth-first recursion over a corrupt tree
//! can be made to overflow the stack by the very input being checked.

use std::fmt;

use deep_grammars::deep::tree::{Node, TOKEN};

const NO_PARENT: u32 = u32::MAX;

/// Why a tree is not a tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeFault {
    Empty,
    RootKind { found: u16, expected: u16 },
    RootSpan { start: u32, end: u32, source: u32 },
    RootSubtree { subtree_end: u32, nodes: u32 },
    BadSpan { node: u32, start: u32, end: u32, source: u32 },
    ChildOutsideParent { node: u32, parent: u32, start: u32, end: u32, parent_start: u32, parent_end: u32 },
    SubtreeOutside { node: u32, subtree_end: u32, index: u32, limit: u32 },
    TokenWithChildren { node: u32, subtree_end: u32 },
    Unreachable { visited: u32, nodes: u32 },
}

impl fmt::Display for TreeFault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeFault::Empty => write!(out, "tree has no nodes"),
            TreeFault::RootKind { found, expected } => write!(out, "root kind {found}, expected {expected}"),
            TreeFault::RootSpan { start, end, source } => write!(out, "root spans {start}..{end} of {source} bytes"),
            TreeFault::RootSubtree { subtree_end, nodes } => write!(out, "root subtree_end {subtree_end} for {nodes} nodes"),
            TreeFault::BadSpan { node, start, end, source } => write!(out, "node {node} span {start}..{end} outside 0..{source}"),
            TreeFault::ChildOutsideParent { node, parent, start, end, parent_start, parent_end } => write!(
                out,
                "node {node} span {start}..{end} outside parent {parent} span {parent_start}..{parent_end}"
            ),
            TreeFault::SubtreeOutside { node, subtree_end, index, limit } => {
                write!(out, "node {node} subtree_end {subtree_end} not in {}..={limit}", index + 1)
            }
            TreeFault::TokenWithChildren { node, subtree_end } => {
                write!(out, "token node {node} has children (subtree_end {subtree_end})")
            }
            TreeFault::Unreachable { visited, nodes } => write!(out, "{visited} of {nodes} nodes reachable from the root"),
        }
    }
}

/// Validate an arena against the root it claims and the source it came from.
pub fn validate(nodes: &[Node], source_len: u32, root_kind: u16) -> Result<(), TreeFault> {
    let Some(root) = nodes.first() else {
        return Err(TreeFault::Empty);
    };
    let nodes_len = nodes.len() as u32;
    if root.kind != root_kind {
        return Err(TreeFault::RootKind { found: root.kind, expected: root_kind });
    }
    if root.start != 0 || root.end != source_len {
        return Err(TreeFault::RootSpan { start: root.start, end: root.end, source: source_len });
    }
    if root.subtree_end != nodes_len {
        return Err(TreeFault::RootSubtree { subtree_end: root.subtree_end, nodes: nodes_len });
    }
    if root.flags & TOKEN != 0 {
        return Err(TreeFault::TokenWithChildren { node: 0, subtree_end: root.subtree_end });
    }

    let mut visited = 0u32;
    // (index, parent index or NO_PARENT, exclusive end of the parent's range)
    let mut stack: Vec<(u32, u32, u32)> = vec![(0, NO_PARENT, nodes_len)];
    while let Some((index, parent, limit)) = stack.pop() {
        if index >= nodes_len {
            return Err(TreeFault::SubtreeOutside { node: index, subtree_end: index, index, limit });
        }
        let node = &nodes[index as usize];
        if node.start > node.end || node.end > source_len {
            return Err(TreeFault::BadSpan { node: index, start: node.start, end: node.end, source: source_len });
        }
        let subtree_end = node.subtree_end;
        if subtree_end <= index || subtree_end > limit {
            return Err(TreeFault::SubtreeOutside { node: index, subtree_end, index, limit });
        }
        if node.flags & TOKEN != 0 && subtree_end != index + 1 {
            return Err(TreeFault::TokenWithChildren { node: index, subtree_end });
        }
        if parent != NO_PARENT {
            let parent_node = &nodes[parent as usize];
            if node.start < parent_node.start || node.end > parent_node.end {
                return Err(TreeFault::ChildOutsideParent {
                    node: index,
                    parent,
                    start: node.start,
                    end: node.end,
                    parent_start: parent_node.start,
                    parent_end: parent_node.end,
                });
            }
        }
        visited += 1;
        // Siblings tile `index + 1 .. subtree_end`; the next sibling starts
        // where the previous one's subtree ends.
        let mut child = index + 1;
        while child < subtree_end {
            let child_end = nodes[child as usize].subtree_end;
            if child_end <= child || child_end > subtree_end {
                return Err(TreeFault::SubtreeOutside { node: child, subtree_end: child_end, index: child, limit: subtree_end });
            }
            stack.push((child, index, subtree_end));
            child = child_end;
        }
    }
    if visited != nodes_len {
        return Err(TreeFault::Unreachable { visited, nodes: nodes_len });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: u16, start: u32, end: u32, subtree_end: u32) -> Node {
        Node { kind, flags: 0, start, end, subtree_end }
    }

    /// A token leaf at `index`; a leaf's subtree ends one past itself.
    fn token(index: u32, start: u32, end: u32) -> Node {
        Node { kind: 9, flags: TOKEN, start, end, subtree_end: index + 1 }
    }

    #[test]
    fn a_well_formed_arena_passes() {
        let nodes = vec![node(16, 0, 3, 4), node(17, 0, 2, 3), token(2, 0, 1), token(3, 2, 3)];
        assert_eq!(validate(&nodes, 3, 16), Ok(()));
    }

    #[test]
    fn an_empty_arena_fails() {
        assert_eq!(validate(&[], 0, 16), Err(TreeFault::Empty));
    }

    #[test]
    fn a_wrong_root_kind_or_span_fails() {
        let nodes = vec![node(16, 0, 3, 1)];
        assert_eq!(validate(&nodes, 3, 17), Err(TreeFault::RootKind { found: 16, expected: 17 }));
        assert_eq!(validate(&nodes, 4, 16), Err(TreeFault::RootSpan { start: 0, end: 3, source: 4 }));
    }

    #[test]
    fn a_node_whose_subtree_ends_past_its_parent_is_unreachable() {
        // The child claims two nodes but the root claims only itself.
        let nodes = vec![node(16, 0, 3, 2), node(17, 0, 3, 3)];
        assert_eq!(
            validate(&nodes, 3, 16),
            Err(TreeFault::SubtreeOutside { node: 1, subtree_end: 3, index: 1, limit: 2 })
        );
    }

    #[test]
    fn an_orphan_node_is_unreachable() {
        // Root ends before the third node, which no sibling chain reaches.
        let nodes = vec![node(16, 0, 3, 2), token(1, 0, 3), token(2, 0, 1)];
        assert_eq!(validate(&nodes, 3, 16), Err(TreeFault::RootSubtree { subtree_end: 2, nodes: 3 }));
    }

    #[test]
    fn a_sibling_chain_that_jumps_backwards_is_a_fault() {
        // Index 2's subtree_end points back at index 1, so walking forward from
        // it would revisit a node and never reach index 3.
        let nodes = vec![node(16, 0, 3, 4), node(17, 0, 1, 2), Node { subtree_end: 1, ..node(17, 1, 2, 0) }, token(3, 2, 3)];
        assert_eq!(
            validate(&nodes, 3, 16),
            Err(TreeFault::SubtreeOutside { node: 2, subtree_end: 1, index: 2, limit: 4 })
        );
    }

    #[test]
    fn a_token_with_children_is_a_fault() {
        let nodes = vec![node(16, 0, 3, 3), Node { flags: TOKEN, ..node(9, 0, 3, 3) }, token(2, 0, 1)];
        assert_eq!(validate(&nodes, 3, 16), Err(TreeFault::TokenWithChildren { node: 1, subtree_end: 3 }));
    }

    #[test]
    fn a_child_spilling_out_of_its_parent_span_is_a_fault() {
        // The token at index 2 claims 1..3 inside a parent that ends at 2.
        let nodes = vec![node(16, 0, 4, 3), node(17, 1, 2, 3), Node { subtree_end: 3, ..token(2, 1, 3) }];
        assert_eq!(
            validate(&nodes, 4, 16),
            Err(TreeFault::ChildOutsideParent { node: 2, parent: 1, start: 1, end: 3, parent_start: 1, parent_end: 2 })
        );
    }
}
