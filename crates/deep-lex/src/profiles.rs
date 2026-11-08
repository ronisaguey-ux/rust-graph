//! Per-language tables, as data: the mode a parser state selects, and the
//! bracket pairs recovery may jump over.
//!
//! The lexer has one implementation. What makes Python, JSON, JavaScript and
//! TypeScript different is entirely in this file: a [`Profile`] names the
//! language, a `StateId -> Mode` slice, and the byte pairs that open and close
//! a group. A grammar with its own state numbering passes its own slice to
//! [`DeepLexer::with_state_modes`](crate::DeepLexer::with_state_modes); the
//! tables here are the shared default the four shipped grammars start from.
//!
//! # State numbering
//!
//! State ids come from the parser, not from the lexer, so a profile table can
//! only be meaningful if both agree on a few anchor states. The deep grammars
//! use the three contexts below for the states where lexing is not obvious;
//! every other state maps to [`Mode::Default`] and the lexer's own rules apply.
//!
//! ```text
//! 0  DEFAULT             ordinary: an operand may follow or continue
//! 1  OPERAND_EXPECTED    an expression starts here (`= /re/`, `( /re/`)
//! 2  RAW_STRING_EXPECTED a raw-string prefix may start here
//! ```
//!
//! A state past the end of a table keeps the mode the lexer is already in, so
//! a grammar may supply only the prefix of the numbering it cares about.

use crate::kind::{Kind, Language, SymbolMap};
use crate::mode::Mode;
use deep_abi::{StateId, SymbolId};

/// Ordinary code; the lexer's context rules decide everything.
pub const DEFAULT: StateId = 0;
/// An operand starts here: in JavaScript and TypeScript `/` is a regex.
pub const OPERAND_EXPECTED: StateId = 1;
/// A raw string may start here: the external scanner owns it.
pub const RAW_STRING_EXPECTED: StateId = 2;

/// How many `(open, close)` byte pairs a profile can declare. Three is every
/// language here; the fixed array lets [`Profile::symbol_pairs`] stay
/// allocation-free.
pub const MAX_PAIRS: usize = 3;

/// One delimiter family, as bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairRule {
    pub open: u8,
    pub close: u8,
}

impl PairRule {
    pub const fn new(open: u8, close: u8) -> PairRule {
        PairRule { open, close }
    }

    /// The kind a grammar maps to a symbol for the opening byte, if any.
    pub fn open_kind(self) -> Option<Kind> {
        Some(match self.open {
            b'(' => Kind::LParen,
            b'[' => Kind::LBracket,
            b'{' => Kind::LBrace,
            _ => return None,
        })
    }

    pub fn close_kind(self) -> Option<Kind> {
        Some(match self.close {
            b')' => Kind::RParen,
            b']' => Kind::RBracket,
            b'}' => Kind::RBrace,
            _ => return None,
        })
    }
}

/// Everything language-specific the lexer and the runtime's recovery need.
///
/// `Copy` and `'static`: a profile is a constant, never built per file.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub name: &'static str,
    pub language: Language,
    /// `StateId -> Mode`, indexed by the parser's state numbering.
    pub states: &'static [Mode],
    /// Delimiter families, in the order recovery should test them.
    pub pairs: &'static [PairRule],
}

impl Profile {
    /// The pair rules in the grammar's symbol numbering, as a fixed array plus
    /// its live length. Call it once and keep the array: `deep_rt::Config`
    /// borrows the slice for the duration of the parse.
    ///
    /// ```ignore
    /// let (pairs, n) = profile.symbol_pairs(&lexer_symbols());
    /// let config = deep_rt::Config::new().with_pairs(&pairs[..n]).with_source(src);
    /// ```
    pub fn symbol_pairs(&self, map: &SymbolMap) -> ([(SymbolId, SymbolId); MAX_PAIRS], usize) {
        let mut out = [(0, 0); MAX_PAIRS];
        let mut n = 0usize;
        for rule in self.pairs.iter().take(MAX_PAIRS) {
            if let (Some(open), Some(close)) = (rule.open_kind(), rule.close_kind()) {
                out[n] = (map.get(open), map.get(close));
                n += 1;
            }
        }
        (out, n)
    }

    /// The mode for `state`, or `None` when the table does not name it and the
    /// lexer should keep the mode it is in.
    pub fn mode_for(&self, state: StateId) -> Option<Mode> {
        self.states.get(state as usize).copied()
    }
}

const PARENS: PairRule = PairRule::new(b'(', b')');
const BRACKETS: PairRule = PairRule::new(b'[', b']');
const BRACES: PairRule = PairRule::new(b'{', b'}');

/// Python: indentation is syntax, so the layout stack runs; raw strings are
/// requested by a state, and there is no regex literal to disambiguate.
pub static PYTHON: Profile = Profile {
    name: "python",
    language: Language::Python,
    states: &[Mode::Default, Mode::Default, Mode::RawString],
    pairs: &[PARENS, BRACKETS, BRACES],
};

/// JSON: no layout, no modes. Strings are double-quoted and numbers may be
/// signed, which the lexer knows from [`Language::Json`]. Objects and arrays
/// are the only groups.
pub static JSON: Profile = Profile {
    name: "json",
    language: Language::Json,
    states: &[Mode::Default, Mode::Default, Mode::Default],
    pairs: &[BRACES, BRACKETS],
};

/// JavaScript: `OPERAND_EXPECTED` is the state where `/` starts a regular
/// expression rather than a division.
pub static JAVASCRIPT: Profile = Profile {
    name: "javascript",
    language: Language::JavaScript,
    states: &[Mode::Default, Mode::Regex, Mode::Default],
    pairs: &[PARENS, BRACKETS, BRACES],
};

/// TypeScript lexes exactly like JavaScript; only the grammar's reading of `<`
/// differs, and that is a grammar decision, not a lexer mode. The table exists
/// separately so the two languages can diverge without touching the lexer.
pub static TYPESCRIPT: Profile = Profile {
    name: "typescript",
    language: Language::JavaScript,
    states: &[Mode::Default, Mode::Regex, Mode::Default],
    pairs: &[PARENS, BRACKETS, BRACES],
};

/// Every shipped profile, in one static array: the same lexer serves all four.
pub static ALL: &[&Profile] = &[&PYTHON, &JSON, &JAVASCRIPT, &TYPESCRIPT];
