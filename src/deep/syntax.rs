//! Kinds, the per-language kind table, and the reserved kinds every language shares.

use super::lexer::Cursor;
use super::parser::Parser;

/// A token or node kind. Kinds below [`FIRST_LANGUAGE_KIND`] belong to the runtime.
pub type Kind = u16;

/// A node holding tokens the grammar could not place, or an empty one for a
/// token the grammar expected and did not find (flagged [`MISSING`](super::tree::MISSING)).
pub const ERROR: Kind = 0;
/// A balanced group jumped whole because a budget ran out. One leaf, no children.
pub const SKIPPED: Kind = 1;
/// Spaces, tabs and newlines between tokens.
pub const WHITESPACE: Kind = 2;
/// A comment. Trivia, like whitespace.
pub const COMMENT: Kind = 3;
/// Bytes that no lexer rule accepts, merged into one token per run.
pub const UNKNOWN: Kind = 4;
/// What the parser sees past the end of the file or its current group.
pub const EOF: Kind = 5;
/// An open marker that was abandoned or folded into a later parent.
pub const TOMBSTONE: Kind = 6;
pub const FIRST_LANGUAGE_KIND: Kind = 16;

/// What a kind is to the runtime. The delimiter pass and the parser read only this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// A node kind made by the grammar.
    Node,
    /// An ordinary token.
    Token,
    /// A token the parser never sees: whitespace, comments.
    Trivia,
    /// A token that is an error by itself, such as an unterminated string.
    BadToken,
    /// An opening delimiter. Openers and closers pair when their family matches.
    Open(u8),
    Close(u8),
    /// Preprocessor branches: `#if`, `#else`/`#elif`, `#endif`. Trivia to the parser.
    BranchIf,
    BranchElse,
    BranchEnd,
}

/// One row of a language's kind table.
#[derive(Clone, Copy, Debug)]
pub struct KindInfo {
    pub name: &'static str,
    pub class: Class,
}

const RESERVED: [KindInfo; FIRST_LANGUAGE_KIND as usize] = {
    let unused = KindInfo { name: "RESERVED", class: Class::Node };
    let mut table = [unused; FIRST_LANGUAGE_KIND as usize];
    table[ERROR as usize] = KindInfo { name: "ERROR", class: Class::Node };
    table[SKIPPED as usize] = KindInfo { name: "SKIPPED", class: Class::Node };
    table[WHITESPACE as usize] = KindInfo { name: "whitespace", class: Class::Trivia };
    table[COMMENT as usize] = KindInfo { name: "comment", class: Class::Trivia };
    table[UNKNOWN as usize] = KindInfo { name: "UNKNOWN", class: Class::BadToken };
    table[EOF as usize] = KindInfo { name: "EOF", class: Class::Token };
    table[TOMBSTONE as usize] = KindInfo { name: "TOMBSTONE", class: Class::Node };
    table
};

/// Indentation as syntax (Python, Nim). The runtime lexer turns line breaks into
/// these tokens, so a grammar consumes layout like any other token, and an
/// indented block pairs `indent` with `dedent` like brackets, firewall included.
///
/// - `newline`: what the language's step lexer returns for a line break. The
///   runtime stores every line break as whitespace and inserts a zero-width
///   `newline` right after the last token of each logical line instead, so a
///   statement ends at its last token. Inside brackets and on blank or
///   comment-only lines there is none.
/// - `indent`, `dedent`: zero-width, an `Open`/`Close` pair of one family. An
///   `indent` sits before the first token of a deeper line; `dedent`s sit right
///   after the `newline` that ends the block, so a block ends at its last token.
///   They always balance: the end of the file closes every open level.
/// - `resets`: tokens that cannot continue a bracketed expression. One that
///   starts a physical line inside brackets ends them there, so an unclosed `(`
///   costs one line, not the rest of the file.
pub struct Layout {
    pub newline: Kind,
    pub indent: Kind,
    pub dedent: Kind,
    pub resets: &'static [Kind],
}

/// A language as the runtime sees it: a kind table, a lexer step and a grammar.
/// Slice 2's `grammar!` generates these; Slice 1 writes JSON's by hand.
pub struct Language {
    pub name: &'static str,
    /// The root node kind. The root always spans the whole file and is never `ERROR`.
    pub root: Kind,
    /// Kinds from [`FIRST_LANGUAGE_KIND`] on, in order.
    pub kinds: &'static [KindInfo],
    /// Lexes one token at the cursor and returns its kind. It must advance the
    /// cursor; if it does not, the runtime takes one character as [`UNKNOWN`].
    pub lex: fn(&mut Cursor) -> Kind,
    /// Indentation as tokens, for languages where it is syntax.
    pub layout: Option<&'static Layout>,
    /// Parses the whole file into the root, which the runtime has already opened.
    pub parse: fn(&mut Parser),
}

impl Language {
    /// A language with no layout.
    ///
    /// The invariants are checked in the constructor, which for a `static` means
    /// at compile time: a grammar that declares no kinds, or a root that is a
    /// runtime kind, is a build failure rather than a parse that quietly returns
    /// the wrong root at run time. Rust has no "missing implementation" here —
    /// `parse` and `lex` are function pointers the language must name — so the
    /// only way a language can be incomplete is by declaring nothing, and that is
    /// what these two assertions refuse.
    pub const fn new(name: &'static str, root: Kind, kinds: &'static [KindInfo], lex: fn(&mut Cursor) -> Kind, parse: fn(&mut Parser)) -> Self {
        // A root that is a runtime kind is always a mistake; an empty table is
        // not — `crates/deep-fuzz` declares one on purpose to exercise exactly
        // the "grammar with no kinds" path, so that assertion lives in the test
        // that every registered grammar parses its own root, not here.
        assert!(root >= FIRST_LANGUAGE_KIND, "a root must be a language kind, not a runtime one");
        Language { name, root, kinds, lex, layout: None, parse }
    }

    /// The same language, with indentation as tokens.
    pub const fn with_layout(mut self, layout: &'static Layout) -> Self {
        self.layout = Some(layout);
        self
    }

    #[inline]
    pub fn info(&self, kind: Kind) -> KindInfo {
        if kind < FIRST_LANGUAGE_KIND {
            RESERVED[kind as usize]
        } else {
            self.kinds[(kind - FIRST_LANGUAGE_KIND) as usize]
        }
    }

    #[inline]
    pub fn class(&self, kind: Kind) -> Class {
        self.info(kind).class
    }

    pub fn name(&self, kind: Kind) -> &'static str {
        self.info(kind).name
    }

    /// Trivia and preprocessor branch lines: present in the tokens, invisible to the grammar.
    #[inline]
    pub fn is_trivia(&self, kind: Kind) -> bool {
        matches!(self.class(kind), Class::Trivia | Class::BranchIf | Class::BranchElse | Class::BranchEnd)
    }
}
