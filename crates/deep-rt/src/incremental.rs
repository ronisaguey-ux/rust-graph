//! Incremental re-parsing over the flat CST arena.
//!
//! A batch parse throws the tree away and starts at byte zero. After a one-byte
//! edit that is almost all wasted work: the bytes are the same, so the LR
//! states are the same, so every reduction far from the edit produces exactly
//! the node it produced last time.
//!
//! This module re-parses from the smallest subtree that covers the edit and
//! splices everything else out of the old tree.
//!
//! # Why a splice is sound
//!
//! The driver is a deterministic LR parser: the tree it builds is a function of
//! the token stream *and* the parser state. For a node `N` of the old tree,
//! [`reparse`] records `state_before(N)`, the state on top of the stack at the
//! first byte of `N`, and the stack height there. The old parse produced `N`
//! from that state over exactly those bytes.
//!
//! So if the new parse reaches the same byte position (shifted by the edit's
//! length delta) with the same top state and the same stack height, and the
//! bytes of `N` are untouched, then re-parsing those bytes *must* rebuild `N`
//! node for node: same shifts, same reductions, same kinds, same spans. The
//! splice does exactly that work's result — it copies the old subtree with the
//! offsets shifted by `delta` and pushes it as one pre-reduced stack entry,
//! which is what the reduction would have pushed.
//!
//! The stack below a spliced node is never touched (an LR reduction pops only
//! the children it is about to wrap, all of which lie inside the node), so
//! induction over the splices keeps the current stack equal to the old one at
//! the matching position, and the final tree is identical to a full parse.
//! [`crate::incremental`]'s differential test checks that on real Python
//! sources rather than trusting the argument.
//!
//! # Where reuse stops
//!
//! * a node whose byte range the edit touches is rebuilt;
//! * a node whose *parse context* changed is rebuilt: if the edit changes how
//!   the tokens before it reduce (indentation is the loud case — one dedent
//!   moves every `DEDENT` token after it), the state at the splice candidate no
//!   longer matches and the candidate is refused;
//! * a tree with error nodes is not reused at all: §Recovery's missing/error
//!   nodes are inserted or adopted rather than shifted, so the recorded states
//!   would be a guess. [`reparse`] falls back to a full parse in that case,
//!   which is always correct and never produces a different tree.
//!
//! Tokenisation is *not* incremental here: the caller passes the whole new
//! token stream. Lexing is linear and much cheaper than parsing, and a
//! context-sensitive lexer (Python layout, mode stacks) cannot in general be
//! restarted at a byte offset without replaying it anyway.
//!
//! # Wiring
//!
//! The module is self-contained: it only uses the public `Arena::nodes` field
//! and the public table functions, so enabling it needs one line in
//! `crates/deep-rt/src/lib.rs`:
//!
//! ```text
//! pub mod incremental;
//! ```
//!
//! The tests drive the file through `#[path]`, so they pass before that line
//! lands.

use deep_abi::{
    Cst, Fatal, Grammar, Lexer, Node, NodeId, Offset, StateId, SymbolId, Token, EOF_SYMBOL,
    FLAG_ERROR, FLAG_MISSING, NONE,
};

use crate::cst::Arena;
use crate::driver::{parse, Config};
use crate::table::{self, Action};

/// One edit, in the old source's byte coordinates.
///
/// `start .. start + old_len` is replaced by `new_len` bytes. Insertions have
/// `old_len == 0`, deletions `new_len == 0`.
///
/// ```
/// # use deep_rt::incremental::Edit;
/// let insert = Edit::insert(10, 3);          // three bytes at offset 10
/// let delete = Edit::delete(10, 3);          // drop three bytes at offset 10
/// let replace = Edit::replace(10, 3, 5);     // three bytes become five
/// assert_eq!(insert.delta(), 3);
/// assert_eq!(delete.delta(), -3);
/// assert_eq!(replace.delta(), 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// First byte the edit touches.
    pub start: Offset,
    /// Bytes removed, in the old source.
    pub old_len: u32,
    /// Bytes inserted, in the new source.
    pub new_len: u32,
}

impl Edit {
    pub fn insert(start: Offset, len: u32) -> Edit {
        Edit {
            start,
            old_len: 0,
            new_len: len,
        }
    }

    pub fn delete(start: Offset, len: u32) -> Edit {
        Edit {
            start,
            old_len: len,
            new_len: 0,
        }
    }

    pub fn replace(start: Offset, old_len: u32, new_len: u32) -> Edit {
        Edit {
            start,
            old_len,
            new_len,
        }
    }

    /// Last byte of the old range.
    #[inline]
    pub fn old_end(&self) -> Offset {
        self.start.saturating_add(self.old_len)
    }

    /// Last byte of the new range.
    #[inline]
    pub fn new_end(&self) -> Offset {
        self.start.saturating_add(self.new_len)
    }

    /// New length minus old length. Negative for a deletion.
    #[inline]
    pub fn delta(&self) -> i64 {
        self.new_len as i64 - self.old_len as i64
    }

    /// Apply the edit: splice `text` into `source` at `start`.
    ///
    /// `text` must be `new_len` bytes; a mismatch is truncated or padded with
    /// nothing, so the caller's `Edit` stays the single source of truth.
    pub fn apply(&self, source: &[u8], text: &[u8]) -> Vec<u8> {
        let start = (self.start as usize).min(source.len());
        let end = (self.old_end() as usize).min(source.len());
        let text = &text[..text.len().min(self.new_len as usize)];
        let mut out = Vec::with_capacity(source.len() - (end - start) + text.len());
        out.extend_from_slice(&source[..start]);
        out.extend_from_slice(text);
        out.extend_from_slice(&source[end..]);
        out
    }
}

/// What the incremental pass cost, and how much of the old tree survived.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// First byte re-parsed.
    pub window_start: Offset,
    /// Last byte of the covering node in old coordinates.
    pub window_end: Offset,
    /// Nodes of the old tree that were reused verbatim (whole count, subtrees
    /// included): the untouched prefix plus every spliced suffix subtree.
    pub reused_nodes: u32,
    /// Bytes covered by reused nodes.
    pub reused_bytes: u64,
    /// Nodes the incremental pass allocated by parsing.
    pub rebuilt_nodes: u32,
    /// Nodes in the resulting tree.
    pub total_nodes: u32,
    /// How many suffix subtrees were spliced in (one per resumed region).
    pub splices: u32,
    /// True when the incremental pass gave up and re-parsed everything (error
    /// nodes in the old tree, or a state that did not line up).
    pub fell_back: bool,
}

impl Report {
    /// Reused nodes over total nodes. `1.0` means nothing was rebuilt.
    pub fn reuse_ratio(&self) -> f64 {
        if self.total_nodes == 0 {
            return 1.0;
        }
        self.reused_nodes as f64 / self.total_nodes as f64
    }
}

/// Re-parse `tokens` (the whole new source) against the tree of the old source.
///
/// `old` must be the tree of the previous parse, `old_len` its source length,
/// and `tokens` the new source's terminals in order (`EOF` is implicit; a
/// trailing `EOF_SYMBOL` token is tolerated). `config` is only used when the
/// pass has to fall back to a full parse, so its `source` should be the **new**
/// source.
///
/// The returned tree is identical — same kinds, same spans, same shape — to
/// [`parse`] over the same tokens. On any inconsistency the fallback guarantees
/// that, at the cost of no reuse.
pub fn reparse(
    grammar: &Grammar<'_>,
    old: &Cst,
    old_len: Offset,
    edit: &Edit,
    tokens: &[Token],
    config: &Config<'_>,
) -> Result<(Cst, Report), Fatal> {
    let tokens = trim_eof(tokens);
    let new_len = (old_len as i64 + edit.delta()).max(0) as Offset;
    let mut report = Report {
        window_start: 0,
        window_end: old_len,
        ..Report::default()
    };

    // Everything that could make the recorded states a guess, or that leaves no
    // tree to reuse, goes down the full-parse path.
    let usable = !old.nodes.is_empty()
        && old.root != NONE
        && (old.root as usize) < old.nodes.len()
        && old.errors == 0
        && edit.old_end() <= old_len;

    // A window whose prefix still does not line up is widened one level per
    // attempt; the root is always sound and always available as a last resort.
    if usable {
        for widen in 0..6 {
            let Some(prepared) = prepare(grammar, old, edit, old_len, tokens, widen) else {
                continue;
            };
            let mut attempt = Report {
                window_start: 0,
                window_end: old_len,
                ..Report::default()
            };
            if let Some(cst) = run(grammar, old, edit, tokens, new_len, &prepared, &mut attempt) {
                return Ok((cst, attempt));
            }
        }
    }

    let (cst, _) = full_parse(grammar, tokens, config)?;
    report.fell_back = true;
    report.total_nodes = cst.nodes.len() as u32;
    report.rebuilt_nodes = report.total_nodes;
    Ok((cst, report))
}

/// Drop a trailing `EOF` token if the caller supplied one; the driver adds its
/// own, and a second would be shifted twice.
fn trim_eof(tokens: &[Token]) -> &[Token] {
    match tokens.last() {
        Some(token) if token.symbol == EOF_SYMBOL => &tokens[..tokens.len() - 1],
        _ => tokens,
    }
}

/// A token-slice lexer, for the fallback path.
struct Slices<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Lexer for Slices<'_> {
    fn next_token(&mut self, _state: StateId) -> Option<Token> {
        let token = self.tokens.get(self.at).copied();
        if token.is_some() {
            self.at += 1;
        }
        token
    }
}

fn full_parse(
    grammar: &Grammar<'_>,
    tokens: &[Token],
    config: &Config<'_>,
) -> Result<(Cst, crate::driver::Stats), Fatal> {
    let mut lexer = Slices { tokens, at: 0 };
    parse(grammar, &mut lexer, config)
}

/// Everything the replay learned about the old tree, and the window to start at.
struct Prepared {
    /// State on top of the stack at the first byte of each old node.
    state_before: Vec<StateId>,
    /// Stack height (states.len()) at the first byte of each old node.
    height_before: Vec<u32>,
    /// The stack at `window_start`, as `(state, old node)` pairs.
    prefix_states: Vec<StateId>,
    prefix_nodes: Vec<NodeId>,
    /// First and one-past-last token ordinal of each old node's subtree, in
    /// document order. These are what a splice advances the token index by:
    /// byte offsets cannot tell two zero-width `DEDENT`s at the same position
    /// apart, token ordinals can.
    leaf_start: Vec<u32>,
    leaf_end: Vec<u32>,
    /// The old token stream in document order: symbol and span of every leaf.
    /// A splice must find exactly these tokens, shifted, in the new stream.
    leaves: Vec<(SymbolId, Offset, Offset)>,
    /// How many zero-width tokens at `window.start` the prefix already
    /// consumed (the `DEDENT`s that close the blocks before a statement).
    empty_before: u32,
    /// The smallest node covering the edit.
    window: NodeId,
    /// Old nodes strictly after the edit, sorted by start and then by span
    /// descending: `(old_start, old_id)`.
    suffix: Vec<(Offset, NodeId)>,
}

/// Walk the old tree in the order the parser built it, replaying the LR stack.
///
/// Returns `None` when the tree cannot be replayed exactly (a missing node, an
/// action the table does not have), which the caller treats as "parse it all".
fn prepare(
    grammar: &Grammar<'_>,
    old: &Cst,
    edit: &Edit,
    old_len: Offset,
    tokens: &[Token],
    widen: usize,
) -> Option<Prepared> {
    // The smallest node covering the edit is the first candidate. When the
    // prefix's token stream no longer matches the new one — a deleted line can
    // remove a `DEDENT` *before* the window — widen to the parent and try again.
    // The last candidate is the root, whose prefix is empty. `widen` starts the
    // search higher, for the caller's retry after a parse that did not line up.
    let parent = parents(old);
    let mut window = covering(&old.nodes, old.root, edit);
    for _ in 0..widen {
        if window == old.root {
            break;
        }
        window = parent[window as usize];
    }
    loop {
        if let Some(prepared) = prepare_at(grammar, old, edit, old_len, window, tokens) {
            return Some(prepared);
        }
        if window == old.root {
            return None;
        }
        window = parent[window as usize];
    }
}

/// Parent of every node, so a window can be widened one level at a time.
fn parents(old: &Cst) -> Vec<NodeId> {
    let mut parent = vec![NONE; old.nodes.len()];
    let mut stack: Vec<NodeId> = vec![old.root];
    while let Some(id) = stack.pop() {
        let mut child = old.nodes[id as usize].first_child;
        while child != NONE {
            parent[child as usize] = id;
            stack.push(child);
            child = old.nodes[child as usize].next_sibling;
        }
    }
    parent
}

fn prepare_at(
    grammar: &Grammar<'_>,
    old: &Cst,
    edit: &Edit,
    old_len: Offset,
    window: NodeId,
    tokens: &[Token],
) -> Option<Prepared> {
    macro_rules! giveup {
        ($($t:tt)*) => {{
            return None;
        }};
    }
    let nodes = &old.nodes;
    let mut state_before = vec![StateId::MAX; nodes.len()];
    let mut height_before = vec![0u32; nodes.len()];
    let mut leaf_start = vec![0u32; nodes.len()];
    let mut leaf_end = vec![0u32; nodes.len()];
    let mut leaves = 0u32;
    let mut leaf_tokens: Vec<(SymbolId, Offset, Offset)> = Vec::new();
    let mut states: Vec<StateId> = Vec::with_capacity(64);
    states.push(0);
    let mut entries: Vec<NodeId> = Vec::with_capacity(64);
    entries.push(NONE);
    let mut prefix: Option<(Vec<StateId>, Vec<NodeId>)> = None;

    // Iterative post-order: children before their parent, left to right.
    let mut work: Vec<(NodeId, bool)> = Vec::with_capacity(64);
    work.push((old.root, false));
    let mut kids: Vec<NodeId> = Vec::new();
    while let Some((id, done)) = work.pop() {
        let node = nodes[id as usize];
        if !done {
            work.push((id, true));
            kids.clear();
            let mut child = node.first_child;
            while child != NONE {
                if nodes[child as usize].flags & FLAG_ERROR == 0 {
                    kids.push(child);
                }
                child = nodes[child as usize].next_sibling;
            }
            for &child in kids.iter().rev() {
                work.push((child, false));
            }
            continue;
        }

        if node.flags & (FLAG_ERROR | FLAG_MISSING) != 0 {
            // Error nodes are adopted into a parent and missing nodes are
            // inserted, so neither is a plain stack entry.
            giveup!("prepare: error/missing node");
        }

        // Count the children this node's reduction pops.
        let mut popped = 0usize;
        let mut child = node.first_child;
        while child != NONE {
            if nodes[child as usize].flags & FLAG_ERROR == 0 {
                popped += 1;
            }
            child = nodes[child as usize].next_sibling;
        }
        if popped > 0 && popped + 1 > states.len() {
            giveup!("prepare: stack too short at {id}");
        }
        if popped > 0 {
            let keep = states.len() - popped;
            states.truncate(keep);
            entries.truncate(keep);
        }
        let top = *states.last()?;
        state_before[id as usize] = top;
        height_before[id as usize] = states.len() as u32;
        if id == window {
            prefix = Some((states.clone(), entries.clone()));
        }

        if popped == 0 {
            // Either a terminal the parser shifted, or a node an ε production
            // reduced — an empty `STMTS` has no children and is not a token.
            match table::action(grammar, top, node.kind) {
                Action::Shift(target) => {
                    leaf_start[id as usize] = leaves;
                    leaves += 1;
                    leaf_end[id as usize] = leaves;
                    leaf_tokens.push((node.kind, node.start, node.end));
                    states.push(target);
                    entries.push(id);
                }
                _ => match table::goto(grammar, top, node.kind) {
                    Some(target) => {
                        leaf_start[id as usize] = leaves;
                        leaf_end[id as usize] = leaves;
                        states.push(target);
                        entries.push(id);
                    }
                    None => giveup!("prepare: no goto/shift for kind {} at {id}", node.kind),
                },
            }
        } else {
            let kind = node.kind;
            leaf_end[id as usize] = leaves;
            let mut first = node.first_child;
            leaf_start[id as usize] = leaves;
            while first != NONE {
                if nodes[first as usize].flags & FLAG_ERROR == 0 {
                    leaf_start[id as usize] = leaf_start[first as usize];
                    break;
                }
                first = nodes[first as usize].next_sibling;
            }
            match table::goto(grammar, top, kind) {
                Some(target) => {
                    states.push(target);
                    entries.push(id);
                }
                None => giveup!("prepare: no goto for kind {kind} from {top} at {id}"),
            }
        }
    }

    // A clean replay ends with state 0 plus the root.
    if states.len() != 2 {
        giveup!("prepare: replay ended at len {}", states.len());
    }
    let Some((prefix_states, prefix_nodes)) = prefix else {
        giveup!("prepare: window {window} was never visited");
    };
    // Zero-width tokens sit at the same byte as the node that follows them, so
    // a byte lookup lands on one the prefix already ate.
    let window_start = nodes[window as usize].start;
    let mut empty_before = 0u32;
    for (id, node) in nodes.iter().enumerate() {
        if node.start == window_start
            && node.end == window_start
            && leaf_end[id] == leaf_start[id] + 1
            && leaf_start[id] < leaf_start[window as usize]
        {
            empty_before += 1;
        }
    }

    // Children of the window that the edit does not touch are still candidates
    // but the window itself is not: it is the region we rebuild.
    let mut suffix: Vec<(Offset, NodeId)> = Vec::new();
    for (id, node) in nodes.iter().enumerate() {
        if node.flags & (FLAG_ERROR | FLAG_MISSING) != 0 {
            continue;
        }
        if node.end <= node.start || node.start < edit.old_end() || node.end > old_len {
            continue;
        }
        suffix.push((node.start, id as NodeId));
    }
    // Biggest span first inside each start, so the first candidate that lines up
    // reuses as much as possible.
    suffix.sort_unstable_by(|a, b| {
        let sa = nodes[a.1 as usize].end - nodes[a.1 as usize].start;
        let sb = nodes[b.1 as usize].end - nodes[b.1 as usize].start;
        a.0.cmp(&b.0).then(sb.cmp(&sa)).then(a.1.cmp(&b.1))
    });

    // Is the prefix really the same token sequence? Layout tokens are the
    // reason this matters: deleting the `def` that opened a block removes a
    // `DEDENT` before the edit, and every state after it is then meaningless.
    let mut ti = 0usize;
    while ti < tokens.len() && tokens[ti].start < window_start {
        ti += 1;
    }
    ti += empty_before as usize;
    let before = leaf_start[window as usize] as usize;
    if ti != before {
        giveup!("prepare: prefix has {before} tokens, the new stream {ti}");
    }
    let order = leaf_tokens;
    // The prefix state must be able to act on the next token. When an edit
    // deletes a whole token — dedenting a line removes its `INDENT` — the state
    // that expected that token cannot continue, and the window has to go back
    // to an enclosing production.
    if let (Some(top), Some(next)) = (prefix_states.last().copied(), tokens.get(ti)) {
        if table::action(grammar, top, next.symbol) == Action::None {
            giveup!("prepare: state {top} cannot act on symbol {}", next.symbol);
        }
    }
    if ti > 0 {
        for at in 0..ti {
            if order[at].0 != tokens[at].symbol {
                giveup!("prepare: token {at} was {} and is now {}", order[at].0, tokens[at].symbol);
            }
        }
    }

    Some(Prepared {
        state_before,
        height_before,
        leaf_start,
        leaf_end,
        prefix_states,
        prefix_nodes,
        empty_before,
        leaves: order,
        window,
        suffix,
    })
}

