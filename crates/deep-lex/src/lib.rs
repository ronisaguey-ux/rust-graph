//! A context-sensitive, zero-copy lexer for `deep-abi::Lexer`.
//!
//! The contract is small: produce `deep_abi::Token`s whose `start`/`end` are
//! byte offsets into the original `&[u8]`. Nothing here builds a `String`, a
//! `Vec<String>` or a substring; identifiers, numbers, strings and comments are
//! spans over the caller's buffer, and the only heap allocation the lexer
//! itself makes is the bracket-pair `Vec` built once in [`DeepLexer::new`].
//! ([`tokenize`], a convenience wrapper, also allocates the `Vec` of tokens it
//! returns; the lexer behind it does not.)
//!
//! Three things make it context-sensitive:
//!
//! * a mode stack ([`ModeStack`]) that the parser drives through a
//!   `StateId -> Mode` table and that the lexer also pushes and pops itself —
//!   template literals push `TemplateExpr` on `${` and pop on the matching `}`,
//!   raw strings and regexes are modes a parser state can request;
//! * Python layout: NEWLINE, INDENT and DEDENT are emitted as zero-width
//!   tokens, suppressed inside brackets and after a backslash continuation,
//!   and never emitted for blank or comment-only lines;
//! * an [`ExternalScanner`] for constructs no table can express (C raw strings,
//!   JavaScript regex-vs-division).
//!
//! Malformed input never panics: an unterminated string ends at the line break
//! or at EOF, an unmatched bracket is skipped, and a byte no rule claims
//! becomes a one-byte `Kind::Error` token for the parser's recovery to consume.

pub mod brackets;
pub mod cursor;
pub mod kind;
pub mod mode;
pub mod profiles;
pub mod python;
pub mod recovery;
pub mod scanner;

mod layout;
mod run;
mod scan;

pub use brackets::{Brackets, NONE};
pub use cursor::{Cursor, Span};
pub use deep_abi::{Lexer, StateId, Token};
pub use kind::{Kind, Language, SymbolMap, OPERATORS};
pub use mode::{IndentStack, Mode, ModeStack, INDENT_CAP, MODE_CAP};
pub use profiles::{PairRule, Profile, ALL, JAVASCRIPT, JSON, PYTHON, TYPESCRIPT};
pub use recovery::PairJump;
pub use scanner::{ExternalScanner, NativeScanner, NoScanner, ScanCtx};

const DEFAULT_TAB_WIDTH: u32 = python::TAB_WIDTH;

/// The lexer. `S` is the external scanner, statically dispatched; the default
/// [`NativeScanner`] covers the shipped languages, and `NoScanner` is available
/// for table-only grammars.
pub struct DeepLexer<'a, S = NativeScanner> {
    cur: Cursor<'a>,
    lang: Language,
    symbols: SymbolMap,
    modes: ModeStack,
    indents: IndentStack,
    /// Optional parser-supplied `StateId -> Mode` table.
    state_modes: &'a [Mode],
    /// Whether this lexer has a hint frame on the mode stack.
    hint_on: bool,
    scanner: S,
    /// Last significant token kind, for regex-vs-division and similar rules.
    prev: Kind,
    brackets: Brackets,
    /// True at the start of a physical line, including the first one.
    line_start: bool,
    /// True once the current line produced a real token (not layout/trivia).
    line_has_token: bool,
    /// Number of zero-width DEDENTs still owed.
    pending_dedents: u32,
    /// Whether a zero-width INDENT is owed.
    pending_indent: bool,
    /// Where the pending INDENT/DEDENT tokens sit (byte offset, zero width).
    layout_pos: u32,
    /// Open-bracket nesting; layout is suppressed while this is non-zero.
    suppress: u32,
    /// Leading indentation of the very first line is not an INDENT.
    first_line: bool,
    eof_newline_done: bool,
    eof_dedent_done: bool,
    comments_as_tokens: bool,
    tab_width: u32,
    done: bool,
}

impl<'a> DeepLexer<'a, NativeScanner> {
    /// The shipped lexer: built-in external scanner for the language.
    pub fn new(src: &'a [u8], lang: Language) -> DeepLexer<'a, NativeScanner> {
        DeepLexer::with_scanner(src, lang, NativeScanner::new(lang))
    }

    /// The shipped lexer for one of the [`profiles`]: language, state modes and
    /// bracket pairs from the same table, so the parser cannot pick a mode
    /// table that disagrees with the lexer's rules.
    pub fn for_profile(src: &'a [u8], profile: &'static Profile) -> DeepLexer<'a, NativeScanner> {
        DeepLexer::new(src, profile.language).with_state_modes(profile.states)
    }
}

impl<'a, S: ExternalScanner> DeepLexer<'a, S> {
    /// The next terminal for `state`. `None` only at end of input.
    pub fn next(&mut self, state: StateId) -> Option<Token> {
        self.next_token(state)
    }

    pub fn with_scanner(src: &'a [u8], lang: Language, scanner: S) -> DeepLexer<'a, S> {
        DeepLexer {
            cur: Cursor::new(src),
            lang,
            symbols: SymbolMap::identity(),
            modes: ModeStack::new(),
            indents: IndentStack::new(),
            state_modes: &[],
            hint_on: false,
            scanner,
            prev: Kind::Error, // nothing came before: a leading `/` is a regex
            brackets: Brackets::scan(src),
            line_start: true,
            line_has_token: false,
            pending_dedents: 0,
            pending_indent: false,
            layout_pos: 0,
            suppress: 0,
            first_line: true,
            eof_newline_done: false,
            eof_dedent_done: false,
            comments_as_tokens: false,
            tab_width: DEFAULT_TAB_WIDTH,
            done: false,
        }
    }

    /// Attach the parser's `StateId -> Mode` table. States beyond the table use
    /// the mode the lexer is in, which is what most states want.
    pub fn with_state_modes(mut self, table: &'a [Mode]) -> DeepLexer<'a, S> {
        self.state_modes = table;
        self
    }

    /// Emit comment tokens (off by default: comments are trivia the parser
    /// never sees).
    pub fn with_comments(mut self, on: bool) -> DeepLexer<'a, S> {
        self.comments_as_tokens = on;
        self
    }

    pub fn with_tab_width(mut self, width: u32) -> DeepLexer<'a, S> {
        self.tab_width = width.max(1);
        self
    }

    pub fn with_symbols(mut self, symbols: SymbolMap) -> DeepLexer<'a, S> {
        self.symbols = symbols;
        self
    }

    pub fn symbols_mut(&mut self) -> &mut SymbolMap {
        &mut self.symbols
    }

    pub fn language(&self) -> Language {
        self.lang
    }

    /// The bracket pairs computed in one pass at construction.
    pub fn brackets(&self) -> &Brackets {
        &self.brackets
    }

    /// The raw pair table: `2 * delimiters` `u32`s, offset then mate ordinal.
    /// This is the slice the runtime's recovery wants; see [`PairJump`].
    pub fn pair(&self) -> &[u32] {
        self.brackets.pair()
    }

    /// The mate of the delimiter at `offset`, as a byte offset. O(log n).
    pub fn jump(&self, offset: u32) -> Option<u32> {
        self.brackets.jump(offset)
    }

    /// A borrowable jump table for a runtime that cannot borrow `self`, such as
    /// a driver holding `&mut dyn Lexer`.
    pub fn pair_jump(&self) -> PairJump<'_> {
        PairJump::new(self.brackets.pair())
    }

    /// Current byte position of the cursor (where the next token would start).
    pub fn position(&self) -> u32 {
        self.cur.pos()
    }

    pub fn cursor(&self) -> &Cursor<'a> {
        &self.cur
    }

    pub fn modes(&self) -> &ModeStack {
        &self.modes
    }

    pub fn indents(&self) -> &IndentStack {
        &self.indents
    }

    pub fn source(&self) -> &'a [u8] {
        self.cur.source()
    }

    /// The text of a token, borrowed from the source. No allocation.
    pub fn text(&self, token: &Token) -> &'a [u8] {
        self.cur.slice(token.start, token.end)
    }

    pub(crate) fn ctx(&self) -> ScanCtx {
        ScanCtx {
            mode: self.modes.top(),
            prev: self.prev,
            lang: self.lang,
        }
    }

    /// Layout tokens only exist when the language has layout, we are in the
    /// default mode and no bracket is open.
    pub(crate) fn layout_on(&self) -> bool {
        self.lang.has_layout() && self.modes.top() == Mode::Default && self.suppress == 0
    }

    pub(crate) fn emit(&mut self, kind: Kind, span: Span) -> Token {
        if !kind.is_layout() {
            self.prev = kind;
            if !kind.is_trivia() {
                self.line_has_token = true;
            }
        }
        if self.modes.top() == Mode::Default {
            if kind.opens_group() {
                self.suppress = self.suppress.saturating_add(1);
            } else if kind.closes_group() {
                self.suppress = self.suppress.saturating_sub(1);
            }
        }
        Token {
            symbol: self.symbols.get(kind),
            start: span.start,
            end: span.end,
        }
    }

    /// A zero-width structural token. Never touches `prev` or the line state.
    pub(crate) fn layout_token_at(&self, kind: Kind, at: u32) -> Token {
        Token {
            symbol: self.symbols.get(kind),
            start: at,
            end: at,
        }
    }

}
/// The prefix, quote byte, triple-ness and rawness of a string literal that
/// starts at the cursor, or `None` when the cursor is not on one.
pub(crate) fn string_prefix(cur: &Cursor<'_>, lang: Language) -> Option<(usize, u8, bool, bool)> {
    let b0 = cur.peek()?;
    if lang.is_json() {
        // JSON has exactly one string delimiter and no prefixes: anything else
        // is an ERROR token, which is what the old JSON lexer's UNKNOWN is.
        return (b0 == b'"').then_some((0, b'"', false, false));
    }
    if b0 == b'"' || b0 == b'\'' {
        let triple =
            lang == Language::Python && cur.peek_n(1) == Some(b0) && cur.peek_n(2) == Some(b0);
        return Some((0, b0, triple, false));
    }
    if !b0.is_ascii_alphabetic() {
        return None;
    }
    let mut i = 0usize;
    let mut raw = false;
    // At most two prefix characters (`rb`, `br`, `u8`, `L`), then a quote.
    // `r`/`R` is a raw-string prefix in Python only; in C the scanner owns
    // `R"..."` and a malformed delimiter must fall back to an identifier.
    while i < 2 {
        match cur.peek_n(i)? {
            b'r' | b'R' if lang == Language::Python => raw = true,
            b'b' | b'B' | b'u' | b'U' | b'f' | b'F' | b'8' | b'L' => {}
            _ => break,
        }
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let quote = cur.peek_n(i)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let triple =
        lang == Language::Python && cur.peek_n(i + 1) == Some(quote) && cur.peek_n(i + 2) == Some(quote);
    Some((i, quote, triple, raw))
}

/// Lex a whole source with the shipped scanner, for tests and CLI callers.
/// Allocates one `Vec` of tokens; no token allocates.
pub fn tokenize(src: &[u8], lang: Language) -> Vec<Token> {
    let mut lexer = DeepLexer::new(src, lang);
    let mut out = Vec::new();
    while let Some(token) = lexer.next_token(0) {
        out.push(token);
    }
    out
}
