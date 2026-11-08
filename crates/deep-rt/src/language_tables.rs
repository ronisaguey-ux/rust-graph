//! The bridge from a [`Language`] to the GLR driver.
//!
//! A `grammar!` language parses with a recursive descent function and needs no
//! tables. A language whose tables come from a generator cannot have that
//! function, so it carries a [`TableGrammar`] instead and
//! [`crate::Workspace::parse`] hands it to [`crate::driver`]. Both engines lex
//! with the same [`crate::lexer`], pair with the same [`crate::pairs`] and fill
//! the same [`crate::tree::Tree`], so everything above the runtime sees one
//! parser.
//!
//! # What a [`TableGrammar`] must say
//!
//! The tables carry parser states and grammar symbols; the language carries
//! [`Kind`]s. Two functions translate between them, so a table grammar keeps
//! its own symbol numbering (terminals from 1, nonterminals above them) and
//! does not have to be renumbered to match this crate's kind table:
//!
//! * [`TableGrammar::symbol`] maps a `Kind` to the terminal the tables use;
//! * [`TableGrammar::kind`] maps a symbol back to the `Kind` its node shows.
//!
//! A `Kind` that maps to symbol 0 is not part of the grammar. Such a token is
//! still handed to the driver, which has no action for it and recovers by
//! wrapping it in an `ERROR` node, so no byte of the source falls outside the
//! tree.
//!
//! # Differences from the descent engine, measured
//!
//! * **Node granularity.** Every terminal the driver shifts becomes a node, as
//!   does every nonterminal it reduces, so the tree has a node for punctuation
//!   and for each unit production. The descent engine emits a leaf only where a
//!   grammar calls `bump`. They are two different grammars' trees; the tests
//!   compare each engine against itself, not against the other.
//! * **`Outcome`.** The driver counts shifts, reductions and versions instead of
//!   parser steps, so `steps` is that sum and is not comparable across engines.
//!   It has no clock: the step ceiling is enforced by the driver's own guard and
//!   `exhausted` is set when the work done reached that ceiling. `deepest` is the
//!   flattened tree's depth, and `unclosed`/`stray` come from the pair pass the
//!   workspace already ran.
//! * **A leaf is a childless node with bytes.** The arena does not record which
//!   nodes came from the lexer, so [`crate::tree::TOKEN`] is set on every node with
//!   no children and a non-empty range.

use deep_abi::{Grammar, Lexer, StateId, Token};

use crate::budget::{Budget, Exhausted, Outcome};
use crate::lexer::Tokens;
use crate::pairs::Pairs;
use crate::parser::ParseError;
use crate::syntax::{Kind, Language, ERROR};
use crate::tree::{Node, Tree, HAS_ERROR, MISSING, TOKEN};

/// Everything the GLR engine needs that a [`Language`] does not carry.
#[derive(Clone, Copy)]
pub struct TableGrammar {
    /// The generated tables, in the packing X documents.
    pub grammar: Grammar<'static>,
    /// The kind a grammar symbol shows as: terminals and nonterminals alike.
    pub kind: fn(deep_abi::SymbolId) -> Kind,
    /// The terminal the tables use for a kind. 0 means "not in the grammar".
    pub symbol: fn(Kind) -> deep_abi::SymbolId,
    /// `(open, close)` symbol pairs, so recovery can contain a cascade.
    pub pairs: &'static [(deep_abi::SymbolId, deep_abi::SymbolId)],
    /// Missing input inserted in a row before skipping becomes mandatory.
    pub max_inserts: u32,
}

impl TableGrammar {
    pub const fn new(
        grammar: Grammar<'static>,
        kind: fn(deep_abi::SymbolId) -> Kind,
        symbol: fn(Kind) -> deep_abi::SymbolId,
        pairs: &'static [(deep_abi::SymbolId, deep_abi::SymbolId)],
    ) -> TableGrammar {
        TableGrammar { grammar, kind, symbol, pairs, max_inserts: 2 }
    }
}

/// Parse the already-lexed `tokens` with the GLR driver and flatten the result
/// into `tree`. The pair pass must have run first: its counts are part of the
/// outcome and its `unclosed`/`stray` numbers are what the descent engine reports.
pub fn parse(
    src: &[u8],
    tokens: &Tokens,
    language: &Language,
    tables: &'static TableGrammar,
    budget: &Budget,
    pairs: &Pairs,
    tree: &mut Tree,
) -> Result<Outcome, ParseError> {
    let mut lexer = TokenStream { tokens, language, tables, at: 0 };
    let cap = budget.base_steps.saturating_add(budget.steps_per_token.saturating_mul(tokens.len() as u64));
    let config = crate::driver::Config::new()
        .with_pairs(tables.pairs)
        .with_source(src)
        .with_max_inserts(tables.max_inserts);
    let mut config = config;
    config.max_steps = cap.max(1);

    let (cst, stats) = crate::driver::parse(&tables.grammar, &mut lexer, &config).map_err(ParseError::from)?;
    flatten(&cst, tables, src.len() as u32, tree);

    let work = stats.shifts as u64 + stats.reductions as u64 + stats.versions as u64;
    let outcome = Outcome {
        tokens: tokens.len() as u32,
        steps: work,
        deepest: tree.depth(),
        // The descent engine's counter for groups it jumped; the driver reports
        // cascades under its own name and both end up inside an error node.
        skipped_groups: 0,
        exhausted: if work >= config.max_steps { Some(Exhausted::Steps) } else { None },
        unclosed: pairs.unclosed,
        stray: pairs.stray,
    };
    Ok(outcome)
}

/// The significant tokens of a lexed file, as the driver's [`Lexer`].
struct TokenStream<'t> {
    tokens: &'t Tokens,
    language: &'t Language,
    tables: &'static TableGrammar,
    at: usize,
}

impl Lexer for TokenStream<'_> {
    fn next_token(&mut self, _state: StateId) -> Option<Token> {
        while self.at < self.tokens.len() {
            let index = self.at;
            self.at += 1;
            let kind = self.tokens.kind[index];
            if self.language.is_trivia(kind) {
                continue;
            }
            return Some(Token {
                symbol: (self.tables.symbol)(kind),
                start: self.tokens.start(index),
                end: self.tokens.end[index],
            });
        }
        None
    }

    fn mode(&self) -> &'static str {
        "table"
    }
}

/// Preorder-flatten the arena into the runtime's tree.
///
/// The arena links children and siblings; the tree stores `subtree_end`, so the
/// walk is a stack of open nodes and each node is visited twice. No recursion, so
/// a tree a thousand levels deep flattens on any stack.
pub fn flatten(cst: &deep_abi::Cst, tables: &TableGrammar, src_len: u32, tree: &mut Tree) {
    tree.nodes.clear();
    let nodes = &cst.nodes;
    // (arena id, whether this visit closes the node)
    let mut work: Vec<(deep_abi::NodeId, bool)> = Vec::with_capacity(nodes.len() + 4);
    // The emitted index of every node currently open, innermost last.
    let mut open: Vec<u32> = Vec::with_capacity(32);
    work.push((cst.root, false));
    let mut children: Vec<deep_abi::NodeId> = Vec::new();
    while let Some((id, closing)) = work.pop() {
        let node = nodes[id as usize];
        if closing {
            let index = open.pop().expect("a close follows its open");
            tree.nodes[index as usize].subtree_end = tree.nodes.len() as u32;
            if let Some(&parent) = open.last() {
                let carried = tree.nodes[index as usize].flags & (HAS_ERROR | crate::tree::HAS_SKIP);
                tree.nodes[parent as usize].flags |= carried;
            }
            continue;
        }
        let mut flags = 0u16;
        if node.flags & deep_abi::FLAG_ERROR != 0 {
            flags |= HAS_ERROR;
        }
        if node.flags & deep_abi::FLAG_MISSING != 0 {
            flags |= MISSING;
        }
        if node.first_child == deep_abi::NONE && node.end > node.start {
            flags |= TOKEN;
        }
        let index = tree.nodes.len() as u32;
        tree.nodes.push(Node {
            // An error node keeps the runtime's own kind: it is not a grammar
            // symbol, so the language cannot name it.
            kind: if node.kind == deep_abi::ERROR_SYMBOL { ERROR } else { (tables.kind)(node.kind) },
            flags,
            start: node.start,
            end: node.end,
            subtree_end: 0,
        });
        open.push(index);
        work.push((id, true));
        children.clear();
        let mut at = node.first_child;
        while at != deep_abi::NONE {
            children.push(at);
            at = nodes[at as usize].next_sibling;
        }
        for &child in children.iter().rev() {
            work.push((child, false));
        }
    }
    debug_assert!(open.is_empty(), "every open node was closed");
    // The root owns the whole input, leading and trailing trivia included, as in
    // `Tree::build`.
    if let Some(root) = tree.nodes.first_mut() {
        root.start = 0;
        root.end = src_len;
    }
}
