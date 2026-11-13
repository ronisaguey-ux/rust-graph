//! The flat tree: one preorder `Vec<Node>`, `u32` indices, 16 bytes a node.
//!
//! `subtree_end` is the index one past a node's last descendant, so skipping a
//! subtree is one load and every walk is a loop. No walk here recurses, so tree
//! depth cannot overflow the stack (Tree-sitter #5807).

use super::lexer::Tokens;
use super::syntax::{Class, Kind, Language, ERROR, SKIPPED, TOMBSTONE};

/// The node is `ERROR`, or holds one somewhere below.
pub const HAS_ERROR: u16 = 1;
/// A group whose opener has no closer.
pub const UNCLOSED: u16 = 2;
/// An empty `ERROR` standing for a token the grammar expected.
pub const MISSING: u16 = 4;
/// The node is `SKIPPED`, or holds one somewhere below.
pub const HAS_SKIP: u16 = 8;
/// A leaf for one token. Clear for nodes and for `SKIPPED` or budget leaves.
pub const TOKEN: u16 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Node {
    pub kind: Kind,
    pub flags: u16,
    /// Byte range. Trivia around the first and last token is outside it, except
    /// for the root, which spans the whole input.
    pub start: u32,
    pub end: u32,
    pub subtree_end: u32,
}

const _: () = assert!(std::mem::size_of::<Node>() == 16);

impl Node {
    #[inline]
    pub fn is_leaf(&self, index: u32) -> bool {
        self.subtree_end == index + 1
    }
}

/// What the parser records. The tree is built from these after the parse, so
/// backtracking is a truncation and wrapping a finished node is O(1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Opens a node. `forward_parent`, if not zero, is the distance to a later
    /// `Start` that becomes this node's parent (see `Parser::precede`).
    Start { kind: Kind, forward_parent: u32 },
    Finish,
    /// One token as a leaf.
    Token(u32),
    /// Tokens `first..=last` as one leaf of `kind`, with no children: a group
    /// jumped by a budget.
    Span { kind: Kind, first: u32, last: u32 },
    /// Flags for the innermost open node.
    Flag(u16),
}

#[derive(Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    stack: Vec<u32>,
}

impl Tree {
    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Children of `parent`, in order. One add per sibling.
    pub fn children(&self, parent: u32) -> Children<'_> {
        Children { nodes: &self.nodes, next: parent + 1, end: self.nodes[parent as usize].subtree_end }
    }

    /// Greatest nesting depth, the root at 0. A loop with one stack of ends.
    pub fn depth(&self) -> u32 {
        let mut ends: Vec<u32> = Vec::new();
        let mut deepest = 0;
        for (index, node) in self.nodes.iter().enumerate() {
            while ends.last().is_some_and(|&end| end <= index as u32) {
                ends.pop();
            }
            deepest = deepest.max(ends.len() as u32);
            ends.push(node.subtree_end);
        }
        deepest
    }

    /// Builds the tree from the events, consuming forward-parent links as it goes.
    pub(crate) fn build(&mut self, events: &mut [Event], tokens: &Tokens, language: &Language, src_len: u32) {
        self.nodes.clear();
        self.stack.clear();
        // The tree is sized once, from the token count, instead of growing as
        // the build pushes: a node per token plus the groups that wrap them, and
        // a little slack for the wrappers a grammar inserts. Growth is not
        // free — every reallocation copies the whole array — and the count is
        // known before the first push.
        let estimate = tokens.kind.len().saturating_mul(2).saturating_add(16);
        if self.nodes.capacity() < estimate {
            self.nodes.reserve(estimate - self.nodes.len());
        }
        // Stack entries from `unset` up still lack a start: no token reached them yet.
        let mut unset = 0usize;
        let mut last_end = 0u32;
        let mut parents: Vec<Kind> = Vec::new();
        for at in 0..events.len() {
            match events[at] {
                Event::Start { kind: TOMBSTONE, forward_parent: 0 } => {}
                Event::Start { kind, forward_parent } => {
                    // Follow the chain to the outermost parent; open outermost first.
                    parents.clear();
                    parents.push(kind);
                    let mut link = forward_parent;
                    let mut cursor = at;
                    while link != 0 {
                        cursor += link as usize;
                        let Event::Start { kind, forward_parent } = events[cursor] else {
                            unreachable!("a forward parent is always a Start");
                        };
                        events[cursor] = Event::Start { kind: TOMBSTONE, forward_parent: 0 };
                        parents.push(kind);
                        link = forward_parent;
                    }
                    for &kind in parents.iter().rev() {
                        if kind != TOMBSTONE {
                            self.open(kind);
                        }
                    }
                }
                Event::Finish => {
                    let index = self.stack.pop().expect("every Finish has a Start");
                    if unset <= self.stack.len() {
                        // An empty node: place it, and any ancestors still waiting,
                        // where the last token ended, so ranges still nest.
                        for &open in &self.stack[unset..] {
                            self.nodes[open as usize].start = last_end;
                        }
                        self.nodes[index as usize].start = last_end;
                    }
                    unset = self.stack.len();
                    let end = self.nodes.len() as u32;
                    let node = &mut self.nodes[index as usize];
                    node.end = last_end.max(node.start);
                    node.subtree_end = end;
                    if node.kind == ERROR {
                        node.flags |= HAS_ERROR;
                    }
                    let carried = node.flags & (HAS_ERROR | HAS_SKIP);
                    if let Some(&parent) = self.stack.last() {
                        self.nodes[parent as usize].flags |= carried;
                    }
                }
                Event::Token(token) => {
                    let token = token as usize;
                    let kind = tokens.kind[token];
                    let mut flags = TOKEN;
                    if language.class(kind) == Class::BadToken {
                        flags |= HAS_ERROR;
                    }
                    self.leaf(kind, flags, tokens.start(token), tokens.end[token], &mut unset);
                    last_end = tokens.end[token];
                }
                Event::Span { kind, first, last } => {
                    let flags = match kind {
                        SKIPPED => HAS_SKIP,
                        ERROR => HAS_ERROR,
                        _ => 0,
                    };
                    let (start, end) = (tokens.start(first as usize), tokens.end[last as usize]);
                    self.leaf(kind, flags, start, end, &mut unset);
                    last_end = end;
                }
                Event::Flag(flags) => {
                    let open = *self.stack.last().expect("a Flag has an open node");
                    self.nodes[open as usize].flags |= flags;
                }
            }
        }
        assert!(self.stack.is_empty(), "every Start has a Finish");
        // The root owns the whole input, leading and trailing trivia included.
        let root = &mut self.nodes[0];
        root.start = 0;
        root.end = src_len;
    }

    fn open(&mut self, kind: Kind) {
        self.stack.push(self.nodes.len() as u32);
        self.nodes.push(Node { kind, flags: 0, start: u32::MAX, end: 0, subtree_end: 0 });
    }

    fn leaf(&mut self, kind: Kind, flags: u16, start: u32, end: u32, unset: &mut usize) {
        for &open in &self.stack[(*unset).min(self.stack.len())..] {
            self.nodes[open as usize].start = start;
        }
        *unset = self.stack.len();
        let index = self.nodes.len() as u32;
        self.nodes.push(Node { kind, flags, start, end, subtree_end: index + 1 });
        if flags & (HAS_ERROR | HAS_SKIP) != 0 {
            if let Some(&parent) = self.stack.last() {
                self.nodes[parent as usize].flags |= flags & (HAS_ERROR | HAS_SKIP);
            }
        }
    }

    /// The tree as an S-expression: nodes as `(kind child ...)`, token leaves as
    /// their kind name. `named_only` drops punctuation leaves (kinds whose name does
    /// not start with a letter), as Tree-sitter's `ts_node_string` does. Iterative,
    /// so any depth prints.
    pub fn sexp(&self, language: &Language, named_only: bool) -> String {
        let mut out = String::new();
        let mut ends: Vec<u32> = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            let index = index as u32;
            while ends.last().is_some_and(|&end| end <= index) {
                ends.pop();
                out.push(')');
            }
            let name = language.name(node.kind);
            let is_token = node.is_leaf(index) && node.flags & TOKEN != 0;
            if named_only && is_token && !name.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
                continue;
            }
            if index > 0 {
                out.push(' ');
            }
            if is_token {
                out.push_str(name);
            } else {
                out.push('(');
                out.push_str(name);
                if node.flags & MISSING != 0 {
                    out.push_str(" MISSING");
                }
                if node.flags & UNCLOSED != 0 {
                    out.push_str(" UNCLOSED");
                }
                ends.push(node.subtree_end);
            }
        }
        for _ in ends {
            out.push(')');
        }
        out
    }

    /// The tree as JSON, written by hand: `{"kind","start","end"[,"flags"][,"text"]
    /// [,"children":[...]]}`. Token leaves carry their text. Iterative.
    pub fn write_json(&self, language: &Language, src: &[u8], out: &mut String) {
        let mut ends: Vec<u32> = Vec::new();
        let mut first = true;
        for (index, node) in self.nodes.iter().enumerate() {
            let index = index as u32;
            while ends.last().is_some_and(|&end| end <= index) {
                ends.pop();
                out.push_str("]}");
            }
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str("{\"kind\":");
            push_json_string(out, language.name(node.kind).as_bytes());
            out.push_str(",\"start\":");
            out.push_str(&node.start.to_string());
            out.push_str(",\"end\":");
            out.push_str(&node.end.to_string());
            let flags = node.flags & !TOKEN;
            if flags != 0 {
                out.push_str(",\"flags\":[");
                let mut comma = false;
                for (bit, name) in [(HAS_ERROR, "error"), (UNCLOSED, "unclosed"), (MISSING, "missing"), (HAS_SKIP, "skipped")] {
                    if flags & bit != 0 {
                        if comma {
                            out.push(',');
                        }
                        comma = true;
                        push_json_string(out, name.as_bytes());
                    }
                }
                out.push(']');
            }
            if node.flags & TOKEN != 0 {
                out.push_str(",\"text\":");
                push_json_string(out, &src[node.start as usize..node.end as usize]);
            }
            if node.is_leaf(index) {
                out.push('}');
            } else {
                out.push_str(",\"children\":[");
                ends.push(node.subtree_end);
                first = true;
            }
        }
        for _ in ends {
            out.push_str("]}");
        }
    }
}

pub struct Children<'t> {
    nodes: &'t [Node],
    next: u32,
    end: u32,
}

impl Iterator for Children<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.next >= self.end {
            return None;
        }
        let index = self.next;
        self.next = self.nodes[index as usize].subtree_end;
        Some(index)
    }
}

/// A JSON string literal. Invalid UTF-8 becomes U+FFFD, as in `from_utf8_lossy`.
pub fn push_json_string(out: &mut String, bytes: &[u8]) {
    out.push('"');
    for ch in String::from_utf8_lossy(bytes).chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0}'..='\u{1f}' | '\u{7f}' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
}
