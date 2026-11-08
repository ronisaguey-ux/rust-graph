//! The graph-structured stack.
//!
//! A LR stack is a list of `(state, node)` pairs. The moment the parser forks,
//! several stacks share their bottom, so the list becomes a graph: every entry
//! keeps its state and CST node plus up to [`MAX_BRANCHES`] parent links.
//! Indices are `u32` into one `Vec`, so nothing here is a pointer and nothing
//! recurses.
//!
//! Entries are only ever appended, which makes them trivially shareable between
//! the live versions of a fork.

use deep_abi::{NodeId, StateId, NONE};

/// At most this many branches leave one stack entry.
pub const MAX_BRANCHES: usize = 8;

/// One `(state, node)` pair plus the entries beneath it.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub state: StateId,
    pub node: NodeId,
    parents: [u32; MAX_BRANCHES],
    parent_len: u8,
    /// Length of the shortest path to the bottom, for diagnostics and costs.
    pub depth: u32,
}

/// An index into the graph.
pub type GssId = u32;

#[derive(Default)]
pub struct Gss {
    entries: Vec<Entry>,
    /// How often a ninth parent was refused.
    branch_drops: u32,
    /// Reused by `reduce_bases`.
    path: Vec<GssId>,
}

/// One way to see a production's right-hand side on the stack.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Base {
    /// The entry below the production, where the goto is looked up.
    pub base: GssId,
    /// The CST nodes of the production, in source order.
    pub children: Vec<NodeId>,
}

impl Gss {
    pub fn new() -> Gss {
        Gss::default()
    }

    pub fn with_capacity(capacity: usize) -> Gss {
        Gss {
            entries: Vec::with_capacity(capacity),
            branch_drops: 0,
            path: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn branch_drops(&self) -> u32 {
        self.branch_drops
    }

    pub fn entry(&self, id: GssId) -> &Entry {
        &self.entries[id as usize]
    }

    pub fn parents(&self, id: GssId) -> &[u32] {
        let entry = &self.entries[id as usize];
        &entry.parents[..entry.parent_len as usize]
    }

    /// Push `(state, node)` on top of `parent`.
    ///
    /// A shift has a parent; the bottom of a stack does not. When an entry
    /// already has [`MAX_BRANCHES`] parents the new branch is refused and
    /// counted rather than silently corrupting the graph.
    pub fn push(&mut self, state: StateId, node: NodeId, parent: Option<GssId>) -> GssId {
        let mut entry = Entry {
            state,
            node,
            parents: [0; MAX_BRANCHES],
            parent_len: 0,
            depth: 1,
        };
        if let Some(parent) = parent {
            let below = self.entries[parent as usize];
            if below.parent_len as usize >= MAX_BRANCHES {
                self.branch_drops += 1;
            } else {
                entry.parents[0] = parent;
                entry.parent_len = 1;
                entry.depth = below.depth + 1;
            }
        }
        let id = self.entries.len() as GssId;
        self.entries.push(entry);
        id
    }

    /// Add another parent to an existing entry, which is how two reductions
    /// converging on the same state share one continuation.
    pub fn link_parent(&mut self, id: GssId, parent: GssId) -> bool {
        if id == parent {
            return false;
        }
        let before = self.entries[id as usize].depth;
        let entry = &mut self.entries[id as usize];
        if entry.parent_len as usize >= MAX_BRANCHES {
            self.branch_drops += 1;
            return false;
        }
        if entry.parents[..entry.parent_len as usize].contains(&parent) {
            return false;
        }
        entry.parents[entry.parent_len as usize] = parent;
        entry.parent_len += 1;
        let depth = self.entries[parent as usize].depth + 1;
        self.entries[id as usize].depth = depth;
        depth < before
    }

    /// Every distinct path of exactly `len` entries ending at `top` that a
    /// reduction of that length can pop, with the nodes in source order.
    ///
    /// Iterative by construction: a stack a million entries deep costs a
    /// million-entry `Vec`, not a million frames.
    pub fn reduce_bases(&mut self, top: GssId, len: u32, out: &mut Vec<Base>) {
        out.clear();
        if len == 0 {
            out.push(Base {
                base: top,
                children: Vec::new(),
            });
            return;
        }
        let mut path: Vec<GssId> = std::mem::take(&mut self.path);
        path.clear();
        let mut cursor: Vec<(GssId, u8)> = Vec::with_capacity(len as usize + 1);
        path.push(top);
        cursor.push((top, 0));
        loop {
            if cursor.is_empty() {
                break;
            }
            if path.len() == len as usize + 1 {
                let mut children = Vec::with_capacity(len as usize);
                for i in (0..len as usize).rev() {
                    children.push(self.entries[path[i] as usize].node);
                }
                out.push(Base {
                    base: path[len as usize],
                    children,
                });
                cursor.pop();
                path.pop();
                continue;
            }
            let (id, next) = *cursor.last().expect("cursor is not empty");
            let parents = self.parents(id);
            if next as usize >= parents.len() {
                cursor.pop();
                path.pop();
                continue;
            }
            cursor.last_mut().expect("cursor is not empty").1 = next + 1;
            let parent = parents[next as usize];
            path.push(parent);
            cursor.push((parent, 0));
        }
        self.path = path;
    }

    /// The node supplied by the bottom of the `depth`-th parent path, which is
    /// what a caller reports when a version reaches the end.
    pub fn bottom(&self, id: GssId) -> GssId {
        let mut current = id;
        let mut guard = 0usize;
        loop {
            let parents = self.parents(current);
            if parents.is_empty() || guard > self.entries.len() {
                return current;
            }
            current = parents[0];
            guard += 1;
        }
    }
}

/// The root node a finished stack stands for: the top entry's node, or `NONE`.
pub fn root_node(entry: &Entry) -> NodeId {
    if entry.node == NONE {
        NONE
    } else {
        entry.node
    }
}
