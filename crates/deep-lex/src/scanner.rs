//! The escape hatch for the cases a token table cannot express: constructs
//! whose extent depends on context or on a delimiter chosen at the opening
//! byte. A scanner is a plain trait, statically dispatched, and it never
//! allocates — it hands back a `(Kind, Span)` into the source.
//!
//! Two real cases are implemented here, both in [`NativeScanner`]:
//!
//! * C raw strings, `R"delim(...)delim"`, where the closing sequence depends
//!   on the delimiter chosen after the opening quote, and
//! * JavaScript regex-vs-division: `/` is a regex literal after a token that
//!   cannot end an expression (`=`, `(`, `,`, `return`-like identifiers are
//!   left to the grammar) and division after one that can (identifier, number,
//!   `)`, `]`, `}`).

use crate::cursor::{Cursor, Span};
use crate::kind::{Kind, Language};
use crate::mode::Mode;

/// Everything a scanner may look at without touching the lexer.
#[derive(Clone, Copy, Debug)]
pub struct ScanCtx {
    pub mode: Mode,
    /// The last significant token, for context-sensitive decisions.
    pub prev: Kind,
    pub lang: Language,
}

/// Stateless or nearly so. `start` is the cheap "is there anything for me
/// here?" test; `scan` consumes exactly one token; `end` says whether the
/// scanner still holds state.
pub trait ExternalScanner {
    fn start(&mut self, cur: &Cursor<'_>, ctx: ScanCtx) -> bool;
    fn scan(&mut self, cur: &mut Cursor<'_>, ctx: ScanCtx) -> Option<(Kind, Span)>;
    fn end(&self, ctx: ScanCtx) -> bool;
}

/// For grammars that need no scanner at all. This is the default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoScanner;

impl ExternalScanner for NoScanner {
    fn start(&mut self, _cur: &Cursor<'_>, _ctx: ScanCtx) -> bool {
        false
    }

    fn scan(&mut self, _cur: &mut Cursor<'_>, _ctx: ScanCtx) -> Option<(Kind, Span)> {
        None
    }

    fn end(&self, _ctx: ScanCtx) -> bool {
        true
    }
}

/// The scanner the shipped lexer uses: C raw strings and JS regex literals.
#[derive(Clone, Copy, Debug)]
pub struct NativeScanner {
    pub lang: Language,
}

impl NativeScanner {
    pub fn new(lang: Language) -> NativeScanner {
        NativeScanner { lang }
    }

    /// A `/` here starts a regex when the previous token cannot end an
    /// expression. Comments are excluded before this is asked.
    fn regex_here(cur: &Cursor<'_>, ctx: ScanCtx) -> bool {
        cur.peek() == Some(b'/')
            && !cur.starts_with(b"//")
            && !cur.starts_with(b"/*")
            && !ctx.prev.closes_expression()
    }

    fn scan_regex(cur: &mut Cursor<'_>) -> Span {
        let start = cur.pos();
        cur.bump(); // opening '/'
        let mut in_class = false;
        loop {
            match cur.peek() {
                None => break,
                Some(b'\\') => {
                    cur.bump();
                    cur.bump();
                }
                Some(b'[') => {
                    in_class = true;
                    cur.bump();
                }
                Some(b']') => {
                    in_class = false;
                    cur.bump();
                }
                Some(b'/') if !in_class => {
                    cur.bump();
                    // Flags: letters only, ASCII, no allocation.
                    cur.skip_while(|b| b.is_ascii_alphabetic());
                    break;
                }
                Some(b'\n') | Some(b'\r') => break, // unterminated: end at the line break
                Some(_) => {
                    cur.bump();
                }
            }
        }
        cur.span(start)
    }

    /// `R"delim( ... )delim"`. The cursor must be at the `R`. Returns `None`
    /// and restores the cursor when the opening is malformed.
    fn scan_raw(cur: &mut Cursor<'_>) -> Option<Span> {
        let start = cur.pos();
        let src = cur.source();
        let mut i = cur.pos() as usize + 2; // past R"
        let delim_start = i;
        // Up to 16 characters, none may be space, paren, backslash or quote.
        while let Some(&b) = src.get(i) {
            if b == b'(' {
                break;
            }
            let len = i - delim_start;
            if len >= 16 || matches!(b, b' ' | b'\t' | b'\\' | b')' | b'"' | b'\n' | b'\r') {
                cur.set(start as usize);
                return None;
            }
            i += 1;
        }
        if src.get(i) != Some(&b'(') {
            cur.set(start as usize);
            return None;
        }
        let delim = &src[delim_start..i];
        i += 1; // past '('
        // Find ')' + delim + '"'.
        let rest = &src[i..];
        let mut k = 0usize;
        while k < rest.len() {
            if rest[k] == b')' {
                let after = k + 1;
                if rest.get(after..after + delim.len()) == Some(delim) {
                    let end = after + delim.len();
                    if rest.get(end) == Some(&b'"') {
                        let span = Span::new(start, (i + end + 1) as u32);
                        cur.set(i + end + 1);
                        return Some(span);
                    }
                }
            }
            k += 1;
        }
        // Unterminated: the token runs to EOF and the parser recovers.
        let span = Span::new(start, src.len() as u32);
        cur.set(src.len());
        Some(span)
    }
}

impl ExternalScanner for NativeScanner {
    fn start(&mut self, cur: &Cursor<'_>, ctx: ScanCtx) -> bool {
        if ctx.mode == Mode::Regex {
            return true;
        }
        match self.lang {
            Language::C => {
                (ctx.mode == Mode::RawString || (ctx.mode == Mode::Default && cur.starts_with(b"R\"")))
                    && cur.peek() == Some(b'R')
            }
            Language::JavaScript => ctx.mode == Mode::Default && Self::regex_here(cur, ctx),
            Language::Python => ctx.mode == Mode::RawString && cur.peek() == Some(b'R'),
            Language::Generic => ctx.mode == Mode::RawString,
        }
    }

    fn scan(&mut self, cur: &mut Cursor<'_>, ctx: ScanCtx) -> Option<(Kind, Span)> {
        if ctx.mode == Mode::Regex || (self.lang == Language::JavaScript && Self::regex_here(cur, ctx))
        {
            return Some((Kind::Regex, Self::scan_regex(cur)));
        }
        if cur.peek() == Some(b'R') && matches!(self.lang, Language::C | Language::Python) {
            return NativeScanner::scan_raw(cur).map(|span| (Kind::RawString, span));
        }
        None
    }

    fn end(&self, _ctx: ScanCtx) -> bool {
        true
    }
}
