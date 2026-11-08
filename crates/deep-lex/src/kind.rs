//! Token kinds and the kind -> `deep_abi::SymbolId` map.
//!
//! A kind is a `u16` discriminant and the map is a fixed array, so choosing a
//! token's symbol is an indexed load: no string interning, no hash, no alloc.

use deep_abi::SymbolId;

/// Which language's rules the lexer applies. Everything shared lives in
/// `Default`; the variants only decide layout, comment syntax and scanners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// C-like: `//` and `/* */`, no layout tokens.
    Generic,
    /// Python: `#` comments, NEWLINE/INDENT/DEDENT, triple-quoted strings.
    Python,
    /// JavaScript/TypeScript: templates, regex-vs-division, `//` comments.
    JavaScript,
    /// JSON: double-quoted strings only, signed numbers, comments are trivia
    /// (as tree-sitter-json has them), no layout and no raw strings.
    Json,
    /// C/C++/Rust-ish: `//` comments and `R"delim(...)delim"` raw strings.
    C,
}

impl Language {
    /// Python is the only layout-sensitive language here.
    pub fn has_layout(self) -> bool {
        matches!(self, Language::Python)
    }

    /// JSON is the one language whose `'` is not a string delimiter and whose
    /// `-` binds into a number literal.
    pub fn is_json(self) -> bool {
        matches!(self, Language::Json)
    }
}

/// One terminal shape. Deliberately broad: the grammar agent picks the subset
/// it uses and remaps symbols; unknown bytes become `Error` rather than panic.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A byte no rule claimed. One byte, so the parser can skip and recover.
    Error = 0,
    Ident,
    Number,
    String,
    /// A raw string: C `R"(...)"` or a Python `r"..."` prefix.
    RawString,
    /// A JavaScript regular-expression literal.
    Regex,
    /// Template literal opening chunk: `` `text${ ``.
    TemplateHead,
    /// Template chunk between substitutions: `` }text${ ``.
    TemplateMiddle,
    /// Template closing chunk: `` }text` `` (or the whole literal when it has
    /// no substitutions).
    TemplateTail,
    LineComment,
    BlockComment,
    /// Zero-width token at a logical line end (Python layout).
    Newline,
    /// Zero-width token at the first code byte of a more-indented line.
    Indent,
    /// Zero-width token at the first code byte of a dedented line.
    Dedent,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Colon,
    ColonColon,
    Dot,
    DotDot,
    DotDotDot,
    Arrow,
    FatArrow,
    Question,
    QuestionQuestion,
    QuestionDot,
    At,
    Hash,
    Dollar,
    Backslash,
    Plus,
    PlusPlus,
    PlusEq,
    Minus,
    MinusMinus,
    MinusEq,
    Star,
    StarEq,
    Slash,
    SlashEq,
    Percent,
    PercentEq,
    Amp,
    AmpAmp,
    AmpEq,
    Pipe,
    PipePipe,
    PipeEq,
    Caret,
    CaretEq,
    Tilde,
    Bang,
    BangEq,
    BangEqEq,
    Eq,
    EqEq,
    EqEqEq,
    Lt,
    LtEq,
    LtLt,
    LtLtEq,
    Gt,
    GtEq,
    GtGt,
    GtGtGt,
    GtGtEq,
    GtGtGtEq,
}

impl Kind {
    pub const COUNT: usize = Kind::GtGtGtEq as usize + 1;

    /// Is this a token after which `/` means division, not a regex?
    pub fn closes_expression(self) -> bool {
        matches!(
            self,
            Kind::Ident
                | Kind::Number
                | Kind::String
                | Kind::RawString
                | Kind::Regex
                | Kind::TemplateTail
                | Kind::RParen
                | Kind::RBracket
                | Kind::RBrace
                | Kind::PlusPlus
                | Kind::MinusMinus
        )
    }

    pub fn opens_group(self) -> bool {
        matches!(self, Kind::LParen | Kind::LBracket | Kind::LBrace)
    }

    pub fn closes_group(self) -> bool {
        matches!(self, Kind::RParen | Kind::RBracket | Kind::RBrace)
    }

    /// Layout tokens are structure, not content: they must not move the
    /// regex/division decision or count as a line's first token.
    pub fn is_layout(self) -> bool {
        matches!(self, Kind::Newline | Kind::Indent | Kind::Dedent)
    }

    pub fn is_trivia(self) -> bool {
        matches!(self, Kind::LineComment | Kind::BlockComment)
    }

    pub const fn as_index(self) -> usize {
        self as usize
    }

    /// Reverse of the identity symbol mapping, for diagnostics and tests.
    pub fn from_symbol(sym: SymbolId) -> Option<Kind> {
        ALL_KINDS.iter().copied().find(|k| *k as SymbolId == sym)
    }
}

/// Every kind, in discriminant order. Used by [`Kind::from_symbol`].
pub const ALL_KINDS: &[Kind] = &[
    Error, Ident, Number, String, RawString, Regex, TemplateHead, TemplateMiddle, TemplateTail,
    LineComment, BlockComment, Newline, Indent, Dedent, LParen, RParen, LBracket, RBracket, LBrace,
    RBrace, Comma, Semicolon, Colon, ColonColon, Dot, DotDot, DotDotDot, Arrow, FatArrow, Question,
    QuestionQuestion, QuestionDot, At, Hash, Dollar, Backslash, Plus, PlusPlus, PlusEq, Minus,
    MinusMinus, MinusEq, Star, StarEq, Slash, SlashEq, Percent, PercentEq, Amp, AmpAmp, AmpEq, Pipe,
    PipePipe, PipeEq, Caret, CaretEq, Tilde, Bang, BangEq, BangEqEq, Eq, EqEq, EqEqEq, Lt, LtEq, LtLt,
    LtLtEq, Gt, GtEq, GtGt, GtGtGt, GtGtEq, GtGtGtEq,
];

use Kind::*;

/// Longest-first so the first match is the maximal munch. One static table:
/// a lookup here allocates nothing.
pub const OPERATORS: &[(&[u8], Kind)] = &[
    (b">>>=", GtGtGtEq),
    (b"===", EqEqEq),
    (b"!==", BangEqEq),
    (b"...", DotDotDot),
    (b"<<=", LtLtEq),
    (b">>=", GtGtEq),
    (b">>>", GtGtGt),
    (b"==", EqEq),
    (b"!=", BangEq),
    (b"<=", LtEq),
    (b">=", GtEq),
    (b"&&", AmpAmp),
    (b"||", PipePipe),
    (b"??", QuestionQuestion),
    (b"?.", QuestionDot),
    (b"++", PlusPlus),
    (b"--", MinusMinus),
    (b"+=", PlusEq),
    (b"-=", MinusEq),
    (b"*=", StarEq),
    (b"/=", SlashEq),
    (b"%=", PercentEq),
    (b"&=", AmpEq),
    (b"|=", PipeEq),
    (b"^=", CaretEq),
    (b"=>", FatArrow),
    (b"->", Arrow),
    (b"::", ColonColon),
    (b"..", DotDot),
    (b"<<", LtLt),
    (b">>", GtGt),
    (b"+", Plus),
    (b"-", Minus),
    (b"*", Star),
    (b"/", Slash),
    (b"%", Percent),
    (b"&", Amp),
    (b"|", Pipe),
    (b"^", Caret),
    (b"~", Tilde),
    (b"!", Bang),
    (b"=", Eq),
    (b"<", Lt),
    (b">", Gt),
    (b"?", Question),
    (b":", Colon),
    (b";", Semicolon),
    (b",", Comma),
    (b".", Dot),
    (b"@", At),
    (b"#", Hash),
    (b"$", Dollar),
    (b"\\", Backslash),
    (b"(", LParen),
    (b")", RParen),
    (b"[", LBracket),
    (b"]", RBracket),
    (b"{", LBrace),
    (b"}", RBrace),
];

/// Fixed kind -> symbol mapping. Identity by default; a grammar overrides the
/// entries it cares about with `set`.
#[derive(Clone)]
pub struct SymbolMap {
    map: [SymbolId; Kind::COUNT],
}

impl Default for SymbolMap {
    fn default() -> Self {
        let mut map = [0u16; Kind::COUNT];
        for (i, slot) in map.iter_mut().enumerate() {
            *slot = i as SymbolId;
        }
        SymbolMap { map }
    }
}

impl SymbolMap {
    pub fn identity() -> SymbolMap {
        SymbolMap::default()
    }

    pub fn get(&self, kind: Kind) -> SymbolId {
        // Bounds are guaranteed by construction, but stay total anyway.
        *self.map.get(kind.as_index()).unwrap_or(&0)
    }

    pub fn set(&mut self, kind: Kind, symbol: SymbolId) {
        if let Some(slot) = self.map.get_mut(kind.as_index()) {
            *slot = symbol;
        }
    }

    pub fn as_slice(&self) -> &[SymbolId] {
        &self.map
    }
}
