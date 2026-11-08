//! The driver: one token at a time over a graph-structured stack, building the
//! CST as it goes and recovering without ever growing the process stack.
//!
//! The loop is a worklist, the stack is a `Vec`, and every reduction walks at
//! most `production_len` entries. There is no recursion anywhere in this crate.

use deep_abi::{
    Cst, Fatal, Grammar, Lexer, NodeId, Offset, StateId, SymbolId, Token, EOF_SYMBOL, NONE,
};

use crate::cst::Arena;
use crate::stack::{Gss, GssId};
use crate::table::{self, Action};
use crate::version::Version;

/// Everything the driver needs to know that the ABI does not carry.
///
/// Bracket pairs are the lexer's knowledge, handed over here because `Lexer`
/// only promises `next_token`. When they are absent the driver falls back to
/// scanning `source` for ASCII brackets; when both are absent a cascade
/// degrades to skipping one token at a time.
pub struct Config<'a> {
    /// `(open, close)` symbol pairs, in the grammar's numbering.
    pub pairs: &'a [(SymbolId, SymbolId)],
    /// The text being parsed. Only used to find matching brackets.
    pub source: Option<&'a [u8]>,
    /// Missing input inserted in a row before skipping becomes mandatory.
    pub max_inserts: u32,
    /// Hard ceiling on driver steps; 0 means no ceiling.
    pub max_steps: u64,
}

impl Default for Config<'_> {
    fn default() -> Config<'static> {
        Config {
            pairs: &[],
            source: None,
            max_inserts: 2,
            max_steps: 0,
        }
    }
}

impl<'a> Config<'a> {
    pub fn new() -> Config<'a> {
        Config::default()
    }

    pub fn with_pairs(mut self, pairs: &'a [(SymbolId, SymbolId)]) -> Config<'a> {
        self.pairs = pairs;
        self
    }

    pub fn with_source(mut self, source: &'a [u8]) -> Config<'a> {
        self.source = Some(source);
        self
    }

    pub fn with_max_inserts(mut self, max_inserts: u32) -> Config<'a> {
        self.max_inserts = max_inserts;
        self
    }
}

/// What the parse cost. Read this to see forks, drops and recoveries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Tokens the lexer produced.
    pub tokens: u32,
    /// Shifts performed by the winning version.
    pub shifts: u32,
    /// Reductions performed by the winning version.
    pub reductions: u32,
    /// Versions created, forks included.
    pub versions: u32,
    /// Most versions alive at once.
    pub peak_versions: u32,
    /// Branch slots the eight-parent cap refused.
    pub branch_drops: u32,
    /// Versions the ten-version cap destroyed.
    pub version_drops: u32,
    /// Times more than six versions were alive.
    pub overflow_events: u32,
    /// Missing tokens inserted.
    pub inserts: u32,
    /// Tokens skipped into ERROR nodes.
    pub skips: u32,
    /// Balanced groups jumped over as one ERROR node.
    pub cascades: u32,
}

/// Parse `lexer` with `grammar`, or say why no tree exists.
pub fn parse(
    grammar: &Grammar<'_>,
    lexer: &mut dyn Lexer,
    config: &Config<'_>,
) -> Result<(Cst, Stats), Fatal> {
    Parser::new(grammar, lexer, config).run()
}

pub(crate) struct Parser<'g, 'l, 'c> {
    pub(crate) grammar: &'g Grammar<'g>,
    pub(crate) lexer: &'l mut dyn Lexer,
    pub(crate) config: &'c Config<'c>,
    pub(crate) tokens: Vec<Token>,
    pub(crate) eof: Offset,
    pub(crate) arena: Arena,
    pub(crate) gss: Gss,
    pub(crate) set: crate::version::Set,
    pub(crate) stats: Stats,
}


impl<'g, 'l, 'c> Parser<'g, 'l, 'c> {
    fn new(grammar: &'g Grammar<'g>, lexer: &'l mut dyn Lexer, config: &'c Config<'c>) -> Self {
        Parser {
            grammar,
            lexer,
            config,
            tokens: Vec::new(),
            eof: 0,
            arena: Arena::with_capacity(256),
            gss: Gss::with_capacity(64),
            set: crate::version::Set::new(),
            stats: Stats::default(),
        }
    }

    /// The lexer is driven once per position: `state` only chooses a mode, and
    /// every version shares the resulting token.
    pub(crate) fn token_at(&mut self, pos: usize, state: StateId) -> Token {
        while self.tokens.len() <= pos {
            match self.lexer.next_token(state) {
                Some(token) => {
                    self.eof = token.end;
                    self.tokens.push(token);
                }
                None => {
                    let token = Token {
                        symbol: EOF_SYMBOL,
                        start: self.eof,
                        end: self.eof,
                    };
                    self.tokens.push(token);
                }
            }
        }
        self.tokens[pos]
    }

    pub(crate) fn hint(&self, version: &Version) -> StateId {
        version
            .frontier
            .first()
            .map(|&id| self.gss.entry(id).state)
            .unwrap_or(0)
    }

    pub(crate) fn put(&mut self, at: usize, version: Version) {
        *self.set.get_mut(at) = version;
    }

    fn next_live(&self) -> Option<usize> {
        (0..self.set.len()).find(|&at| {
            let version = self.set.get(at);
            !version.dead && !version.accepted
        })
    }

    fn run(mut self) -> Result<(Cst, Stats), Fatal> {
        if self.grammar.states == 0 || self.grammar.action_index.is_empty() {
            return Err(Fatal::NoTable);
        }
        let bottom = self.gss.push(0, NONE, None);
        self.set.push(Version::new(vec![bottom], 0));
        self.stats.versions = 1;
        let limit = if self.config.max_steps == 0 {
            u64::MAX
        } else {
            self.config.max_steps
        };
        let mut guard: u64 = 0;
        loop {
            guard += 1;
            if guard > limit {
                break;
            }
            let Some(at) = self.next_live() else { break };
            self.step(at)?;
            if guard % 256 == 0 {
                self.set.sweep();
            }
        }
        self.finish()
    }

    /// Advance one version by one token.
    fn step(&mut self, at: usize) -> Result<(), Fatal> {
        let placeholder = Version::new(Vec::new(), 0);
        let mut version = std::mem::replace(self.set.get_mut(at), placeholder);
        if version.dead || version.accepted {
            self.put(at, version);
            return Ok(());
        }
        let state = self.hint(&version);
        let token = self.token_at(version.pos, state);
        let symbol = token.symbol;

        let mut work: Vec<GssId> = version.frontier.clone();
        let mut next: Vec<GssId> = Vec::new();
        // Small enough to scan: a frontier is a handful of tops.
        let mut seen: Vec<(GssId, SymbolId)> = Vec::with_capacity(8);
        let mut accepted: Option<NodeId> = None;
        let mut budget: u32 = 1_000_000;

        while let Some(top) = work.pop() {
            if budget == 0 {
                version.dead = true;
                break;
            }
            budget -= 1;
            if seen.contains(&(top, symbol)) {
                continue;
            }
            seen.push((top, symbol));
            let state = self.gss.entry(top).state;
            match table::action(self.grammar, state, symbol) {
                Action::None => {}
                Action::Shift(target) => {
                    if target as u32 >= self.grammar.states {
                        return Err(Fatal::StateOverflow);
                    }
                    let node = self.arena.token(symbol, token.start, token.end);
                    let id = self.gss.push(target, node, Some(top));
                    next.push(id);
                }
                Action::Reduce(production) => {
                    let len = table::production_len(self.grammar, production);
                    let kind = table::lhs(self.grammar, state, production);
                    let mut bases = Vec::new();
                    self.gss.reduce_bases(top, len, &mut bases);
                    if bases.is_empty() {
                        continue;
                    }
                    for base in bases {
                        let base_state = self.gss.entry(base.base).state;
                        let Some(target) = table::goto(self.grammar, base_state, kind) else {
                            continue;
                        };
                        if target as u32 >= self.grammar.states {
                            return Err(Fatal::StateOverflow);
                        }
                        let (node, start, end) =
                            self.arena.reduce(kind, &base.children, 0, token.start);
                        self.adopt_pending(&mut version, node, start, end);
                        let id = self.gss.push(target, node, Some(base.base));
                        work.push(id);
                        self.stats.reductions += 1;
                    }
                }
                Action::Accept => {
                    accepted = Some(self.gss.entry(top).node);
                }
            }
        }

        if let Some(root) = accepted {
            version.accepted = true;
            version.root = root;
            self.flush_skip(&mut version);
            self.settle(&mut version, root);
            self.put(at, version);
            return Ok(());
        }

        if !next.is_empty() {
            next.sort_unstable();
            next.dedup();
            self.flush_skip(&mut version);
            version.frontier = next;
            version.pos += 1;
            version.reset_inserts();
            self.stats.shifts += 1;
            self.put(at, version);
            return Ok(());
        }

        self.recover(at, version)
    }

    fn finish(mut self) -> Result<(Cst, Stats), Fatal> {
        let chosen = self.set.winner().or_else(|| self.set.cheapest());
        let Some(at) = chosen else {
            return Err(Fatal::StackExhausted);
        };
        let mut version = std::mem::replace(
            self.set.get_mut(at),
            Version::new(Vec::new(), 0),
        );
        if version.root == NONE {
            version.root = version
                .frontier
                .first()
                .map(|&id| self.gss.entry(id).node)
                .unwrap_or(NONE);
        }
        let root = version.root;
        self.flush_skip(&mut version);
        if root != NONE {
            self.settle(&mut version, root);
        }
        let errors = self.arena.finish();
        self.stats.tokens = self.tokens.len() as u32;
        self.stats.peak_versions = self.set.peak() as u32;
        self.stats.version_drops = self.set.drops();
        self.stats.overflow_events = self.set.overflow_events();
        self.stats.branch_drops = self.gss.branch_drops();
        let nodes = std::mem::take(&mut self.arena.nodes);
        if root == NONE {
            return Err(Fatal::StackExhausted);
        }
        Ok((
            Cst {
                nodes,
                root,
                errors,
            },
            self.stats,
        ))
    }
}
