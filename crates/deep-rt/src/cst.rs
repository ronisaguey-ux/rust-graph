//! The concrete syntax tree: one flat `Vec<Node>`, no pointers, no recursion.
//!
//! Every terminal the parser shifts and every nonterminal it reduces becomes a
//! node, so the tree covers the whole source range. Siblings are linked with
//! `next_sibling`, children with `first_child`, and a node is never moved: the
//! arena only ever grows.

use deep_abi::{Node, NodeId, Offset, SymbolId, ERROR_SYMBOL, FLAG_ERROR, FLAG_MISSING, NONE};

/// Set while a node is already a child somewhere. Kept in a flag bit the ABI
/// never uses, and cleared before the tree is handed back.
pub(crate) const LINKED: u16 = 1 << 15;
/// The public flag bits; everything above is ours.
const PUBLIC_FLAGS: u16 = FLAG_ERROR | FLAG_MISSING | 0x0000_0ff8;

#[derive(Default)]
pub struct Arena {
    pub nodes: Vec<Node>,
    /// Reused by `reduce`, so a reduce allocates nothing.
    scratch: Vec<NodeId>,
}

impl Arena {
    pub fn with_capacity(capacity: usize) -> Arena {
        Arena {
            nodes: Vec::with_capacity(capacity),
            scratch: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn push(&mut self, node: Node) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(node);
        id
    }

    /// A terminal, exactly the byte range the lexer gave.
    pub fn token(&mut self, symbol: SymbolId, start: Offset, end: Offset) -> NodeId {
        self.push(Node {
            kind: symbol,
            flags: 0,
            start,
            end,
            first_child: NONE,
            next_sibling: NONE,
        })
    }

    /// A zero-width node for input the grammar expected and did not get.
    pub fn missing(&mut self, symbol: SymbolId, at: Offset) -> NodeId {
        self.push(Node {
            kind: symbol,
            flags: FLAG_MISSING,
            start: at,
            end: at,
            first_child: NONE,
            next_sibling: NONE,
        })
    }

    /// The error node recovery wraps skipped input in.
    pub fn error(&mut self, start: Offset, end: Offset) -> NodeId {
        self.push(Node {
            kind: ERROR_SYMBOL,
            flags: FLAG_ERROR,
            start,
            end,
            first_child: NONE,
            next_sibling: NONE,
        })
    }

    /// A nonterminal over `[start, end)`.
    pub fn branch(&mut self, kind: SymbolId, start: Offset, end: Offset, flags: u16) -> NodeId {
        self.push(Node {
            kind,
            flags,
            start,
            end,
            first_child: NONE,
            next_sibling: NONE,
        })
    }

    fn clone_node(&mut self, id: NodeId) -> NodeId {
        let copy = self.nodes[id as usize];
        self.push(copy)
    }

    /// Attach `child` under `parent` at the end of its sibling chain.
    ///
    /// A node that is already linked somewhere is copied first, so two branches
    /// of a fork can never rewrite each other's tree.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        let child = if self.nodes[child as usize].flags & LINKED != 0 {
            self.clone_node(child)
        } else {
            child
        };
        let mut last = self.nodes[parent as usize].first_child;
        if last == NONE {
            self.nodes[parent as usize].first_child = child;
            return;
        }
        while self.nodes[last as usize].next_sibling != NONE {
            last = self.nodes[last as usize].next_sibling;
        }
        self.nodes[last as usize].next_sibling = child;
    }

    /// Build a nonterminal over its children, in source order.
    ///
    /// `children` is small (a production's right-hand side); the passed slice is
    /// not retained. Returns the new node and its span.
    pub fn reduce(
        &mut self,
        kind: SymbolId,
        children: &[NodeId],
        flags: u16,
        at: Offset,
    ) -> (NodeId, Offset, Offset) {
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        for &child in children {
            if self.nodes[child as usize].flags & LINKED != 0 {
                let copy = self.clone_node(child);
                scratch.push(copy);
            } else {
                scratch.push(child);
            }
        }
        for i in 0..scratch.len() {
            let next = if i + 1 < scratch.len() {
                scratch[i + 1]
            } else {
                NONE
            };
            self.nodes[scratch[i] as usize].next_sibling = next;
            self.nodes[scratch[i] as usize].flags |= LINKED;
        }
        let (start, end) = if scratch.is_empty() {
            (at, at)
        } else {
            (
                self.nodes[scratch[0] as usize].start,
                self.nodes[scratch[scratch.len() - 1] as usize].end,
            )
        };
        let parent = self.push(Node {
            kind,
            flags,
            start,
            end,
            first_child: if scratch.is_empty() { NONE } else { scratch[0] },
            next_sibling: NONE,
        });
        self.scratch = scratch;
        (parent, start, end)
    }

    /// Attach nodes that recovery skipped, merging them into the parent's
    /// children by byte position. Used once per parent, so the O(n log n) sort
    /// is over a handful of entries.
    pub fn adopt_sorted(&mut self, parent: NodeId, extras: &[NodeId]) {
        if extras.is_empty() {
            return;
        }
        let extras: Vec<NodeId> = extras
            .iter()
            .map(|&extra| {
                if self.nodes[extra as usize].flags & LINKED != 0 {
                    self.clone_node(extra)
                } else {
                    extra
                }
            })
            .collect();
        let mut all: Vec<NodeId> = Vec::with_capacity(extras.len() + 2);
        let mut at = self.nodes[parent as usize].first_child;
        while at != NONE {
            all.push(at);
            at = self.nodes[at as usize].next_sibling;
        }
        all.extend(extras);
        all.sort_by_key(|&id| self.nodes[id as usize].start);
        for i in 0..all.len() {
            self.nodes[all[i] as usize].next_sibling = if i + 1 < all.len() {
                all[i + 1]
            } else {
                NONE
            };
            self.nodes[all[i] as usize].flags |= LINKED;
        }
        self.nodes[parent as usize].first_child = all[0];
    }

    /// Strip the internal flags and count what the caller cares about.
    pub fn finish(&mut self) -> u32 {
        let mut errors = 0u32;
        for node in &mut self.nodes {
            node.flags &= PUBLIC_FLAGS;
            if node.flags & (FLAG_ERROR | FLAG_MISSING) != 0 {
                errors += 1;
            }
        }
        errors
    }
}

/// Walk a tree iteratively, for tests and diagnostics: `(node, depth)` in
/// document order without touching the process stack.
pub fn preorder(nodes: &[Node], root: NodeId) -> Vec<(NodeId, u32)> {
    let mut out = Vec::new();
    let mut stack: Vec<(NodeId, u32)> = vec![(root, 0)];
    while let Some((id, depth)) = stack.pop() {
        out.push((id, depth));
        // Push siblings in reverse so the leftmost child is visited first.
        let mut children = Vec::new();
        let mut at = nodes[id as usize].first_child;
        while at != NONE {
            children.push(at);
            at = nodes[at as usize].next_sibling;
        }
        for &child in children.iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    out
}

/// The depth of the deepest node, measured without recursion.
pub fn depth(nodes: &[Node], root: NodeId) -> u32 {
    let mut deepest = 0;
    let mut stack: Vec<(NodeId, u32)> = vec![(root, 0)];
    while let Some((id, d)) = stack.pop() {
        if d > deepest {
            deepest = d;
        }
        let mut at = nodes[id as usize].first_child;
        while at != NONE {
            stack.push((at, d + 1));
            at = nodes[at as usize].next_sibling;
        }
    }
    deepest
}
