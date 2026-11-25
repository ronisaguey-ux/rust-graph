//! FRCE — the Fallback Root-Cause Engine.
//!
//! Every time the comparison guard refuses the CST's record, the refusal is
//! classified into one canonical reason, counted, and sampled — so a corpus run
//! can answer *why* files fall back instead of only *how many*.
//!
//! Design notes, in the order they matter:
//!
//! * Nothing here runs on the success path. `record` is called from the guard's
//!   refusal arms only, so a clean parse pays one branch and no allocation.
//! * Counters live in a thread-local fixed-size array indexed by the reason's
//!   discriminant; there is no map, no lock and no per-file allocation. Paths
//!   are interned only once a fallback actually happens.
//! * Codes 1–50 are the syntax taxonomy from the FRCE specification. Codes
//!   51–55 are this engine's differential classes: they are what the corpora
//!   actually produce, because a fallback here is a *record disagreement*
//!   between two readers, not a syntax error. Both are reported; the
//!   differential classes keep `Unclassified` near zero instead of parking
//!   95% of the corpus in one bucket.
//! * Ranking is deterministic: LOC lost, then files, then code.
//!
//! Prior art this follows: rustc's stable error codes with an explanation per
//! code and structured JSON output (`--error-format=json`), clang's diagnostic
//! groups, tree-sitter's ERROR/MISSING nodes as recovery markers, and
//! ClusterFuzz's crash-state signatures — a canonical signature per bucket plus
//! one small sample kept and the rest discarded.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;

/// The canonical reason a file fell back to the lexer's record.
///
/// `#[repr(u8)]` because the stats table is indexed by it; the numbers are part
/// of the contract (they appear in reports and in lane handoffs).
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FallbackReason {
    InvalidUtf8Sequence = 1,
    UnterminatedStringLiteral = 2,
    UnterminatedMultilineString = 3,
    UnterminatedBlockComment = 4,
    UnknownEscapeSequence = 5,
    MalformedNumericLiteral = 6,
    UnrecognizedSigilOrToken = 7,
    StringInterpolationDepth = 8,
    HeredocDelimiterMismatch = 9,
    RegexLiteralAmbiguity = 10,
    UnmatchedClosingBrace = 11,
    UnclosedDelimiterAtEof = 12,
    DelimiterTypeMismatch = 13,
    IndentationDedentMismatch = 14,
    MissingStatementTerminator = 15,
    TernaryColonBlockConflict = 16,
    AngleBracketAmbiguity = 17,
    EmptyBlockUnexpected = 18,
    UnknownInfixOperator = 19,
    UnknownPrefixPostfixOp = 20,
    PrecedenceTableOverflow = 21,
    LambdaClosureSyntax = 22,
    DestructuringPatternFail = 23,
    SpreadRestOperatorContext = 24,
    RangeSliceSyntax = 25,
    PipelineOrComprehension = 26,
    InlineAssemblyBlock = 27,
    ComplexGenericBounds = 28,
    UnionIntersectionType = 29,
    ConditionalOrMappedType = 30,
    FunctionPointerOrCallableType = 31,
    AttributeOrDecoratorSyntax = 32,
    VisibilityOrStorageModifier = 33,
    MultipleReturnOrTupleSig = 34,
    DefaultOrNamedParameter = 35,
    VariadicParameterSyntax = 36,
    AnonymousStructOrInterface = 37,
    DynamicOrComputedImport = 38,
    AliasedOrWildcardImport = 39,
    PreprocessorDirective = 40,
    MacroInvocationSyntax = 41,
    TemplateOrJsxSyntax = 42,
    EmbeddedLanguageBlock = 43,
    ShebangOrFileHeaderDirective = 44,
    CompilerIntrinsicOrBuiltin = 45,
    MaxRecursionDepthExceeded = 46,
    MaxTokenLookaheadExceeded = 47,
    LoopProgressStallGuard = 48,
    CorruptOrMinifiedInput = 49,
    UnclassifiedSyntaxBailout = 50,
    // This engine's differential classes. A refusal here means two readers
    // disagree about a record, not that a token was illegal.
    DifferentialNameValue = 51,
    DifferentialCallValue = 52,
    DifferentialConstantZero = 53,
    DifferentialTargetsDiffer = 54,
    DifferentialMissingFact = 55,
}

impl FallbackReason {
    pub const COUNT: usize = 55;

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn index(self) -> usize {
        self as usize - 1
    }

    pub fn from_index(index: usize) -> Option<FallbackReason> {
        REASONS.get(index).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            FallbackReason::InvalidUtf8Sequence => "InvalidUtf8Sequence",
            FallbackReason::UnterminatedStringLiteral => "UnterminatedStringLiteral",
            FallbackReason::UnterminatedMultilineString => "UnterminatedMultilineString",
            FallbackReason::UnterminatedBlockComment => "UnterminatedBlockComment",
            FallbackReason::UnknownEscapeSequence => "UnknownEscapeSequence",
            FallbackReason::MalformedNumericLiteral => "MalformedNumericLiteral",
            FallbackReason::UnrecognizedSigilOrToken => "UnrecognizedSigilOrToken",
            FallbackReason::StringInterpolationDepth => "StringInterpolationDepth",
            FallbackReason::HeredocDelimiterMismatch => "HeredocDelimiterMismatch",
            FallbackReason::RegexLiteralAmbiguity => "RegexLiteralAmbiguity",
            FallbackReason::UnmatchedClosingBrace => "UnmatchedClosingBrace",
            FallbackReason::UnclosedDelimiterAtEof => "UnclosedDelimiterAtEof",
            FallbackReason::DelimiterTypeMismatch => "DelimiterTypeMismatch",
            FallbackReason::IndentationDedentMismatch => "IndentationDedentMismatch",
            FallbackReason::MissingStatementTerminator => "MissingStatementTerminator",
            FallbackReason::TernaryColonBlockConflict => "TernaryColonBlockConflict",
            FallbackReason::AngleBracketAmbiguity => "AngleBracketAmbiguity",
            FallbackReason::EmptyBlockUnexpected => "EmptyBlockUnexpected",
            FallbackReason::UnknownInfixOperator => "UnknownInfixOperator",
            FallbackReason::UnknownPrefixPostfixOp => "UnknownPrefixPostfixOp",
            FallbackReason::PrecedenceTableOverflow => "PrecedenceTableOverflow",
            FallbackReason::LambdaClosureSyntax => "LambdaClosureSyntax",
            FallbackReason::DestructuringPatternFail => "DestructuringPatternFail",
            FallbackReason::SpreadRestOperatorContext => "SpreadRestOperatorContext",
            FallbackReason::RangeSliceSyntax => "RangeSliceSyntax",
            FallbackReason::PipelineOrComprehension => "PipelineOrComprehension",
            FallbackReason::InlineAssemblyBlock => "InlineAssemblyBlock",
            FallbackReason::ComplexGenericBounds => "ComplexGenericBounds",
            FallbackReason::UnionIntersectionType => "UnionIntersectionType",
            FallbackReason::ConditionalOrMappedType => "ConditionalOrMappedType",
            FallbackReason::FunctionPointerOrCallableType => "FunctionPointerOrCallableType",
            FallbackReason::AttributeOrDecoratorSyntax => "AttributeOrDecoratorSyntax",
            FallbackReason::VisibilityOrStorageModifier => "VisibilityOrStorageModifier",
            FallbackReason::MultipleReturnOrTupleSig => "MultipleReturnOrTupleSig",
            FallbackReason::DefaultOrNamedParameter => "DefaultOrNamedParameter",
            FallbackReason::VariadicParameterSyntax => "VariadicParameterSyntax",
            FallbackReason::AnonymousStructOrInterface => "AnonymousStructOrInterface",
            FallbackReason::DynamicOrComputedImport => "DynamicOrComputedImport",
            FallbackReason::AliasedOrWildcardImport => "AliasedOrWildcardImport",
            FallbackReason::PreprocessorDirective => "PreprocessorDirective",
            FallbackReason::MacroInvocationSyntax => "MacroInvocationSyntax",
            FallbackReason::TemplateOrJsxSyntax => "TemplateOrJsxSyntax",
            FallbackReason::EmbeddedLanguageBlock => "EmbeddedLanguageBlock",
            FallbackReason::ShebangOrFileHeaderDirective => "ShebangOrFileHeaderDirective",
            FallbackReason::CompilerIntrinsicOrBuiltin => "CompilerIntrinsicOrBuiltin",
            FallbackReason::MaxRecursionDepthExceeded => "MaxRecursionDepthExceeded",
            FallbackReason::MaxTokenLookaheadExceeded => "MaxTokenLookaheadExceeded",
            FallbackReason::LoopProgressStallGuard => "LoopProgressStallGuard",
            FallbackReason::CorruptOrMinifiedInput => "CorruptOrMinifiedInput",
            FallbackReason::UnclassifiedSyntaxBailout => "UnclassifiedSyntaxBailout",
            FallbackReason::DifferentialNameValue => "DifferentialNameValue",
            FallbackReason::DifferentialCallValue => "DifferentialCallValue",
            FallbackReason::DifferentialConstantZero => "DifferentialConstantZero",
            FallbackReason::DifferentialTargetsDiffer => "DifferentialTargetsDiffer",
            FallbackReason::DifferentialMissingFact => "DifferentialMissingFact",
        }
    }

    /// One line a fix can act on. Kept short on purpose: a report with prose in
    /// every row is a report nobody scans.
    pub fn explanation(self) -> &'static str {
        match self {
            FallbackReason::InvalidUtf8Sequence => "bytes are not UTF-8, or the BOM is unknown",
            FallbackReason::UnterminatedStringLiteral => "quote closed neither by a quote nor by the line end",
            FallbackReason::UnterminatedMultilineString => "triple-quote or raw block string never closes",
            FallbackReason::UnterminatedBlockComment => "block comment opens and never closes",
            FallbackReason::UnknownEscapeSequence => "escape the language does not define",
            FallbackReason::MalformedNumericLiteral => "base prefix, digit separator or exponent suffix not read",
            FallbackReason::UnrecognizedSigilOrToken => "language sigil outside the lexer's table",
            FallbackReason::StringInterpolationDepth => "nested interpolation carries quotes or braces",
            FallbackReason::HeredocDelimiterMismatch => "heredoc opener and terminator disagree",
            FallbackReason::RegexLiteralAmbiguity => "slash read as division where the language means regex",
            FallbackReason::UnmatchedClosingBrace => "closing token with nothing open",
            FallbackReason::UnclosedDelimiterAtEof => "delimiter still open at end of file",
            FallbackReason::DelimiterTypeMismatch => "closing token does not match the open one",
            FallbackReason::IndentationDedentMismatch => "block ends on an indentation the language forbids",
            FallbackReason::MissingStatementTerminator => "statement boundary both readers can mean",
            FallbackReason::TernaryColonBlockConflict => "colon read as ternary, annotation or block",
            FallbackReason::AngleBracketAmbiguity => "angle read as generic, shift or comparison",
            FallbackReason::EmptyBlockUnexpected => "block with no body where one is required",
            FallbackReason::UnknownInfixOperator => "binary operator outside the precedence table",
            FallbackReason::UnknownPrefixPostfixOp => "unary operator the value walk does not place",
            FallbackReason::PrecedenceTableOverflow => "operator pair with no registered precedence",
            FallbackReason::LambdaClosureSyntax => "closure form the lexer does not open",
            FallbackReason::DestructuringPatternFail => "pattern on the left of an assignment",
            FallbackReason::SpreadRestOperatorContext => "spread or rest in a position the grammar rejects",
            FallbackReason::RangeSliceSyntax => "range or multi-dimensional slice",
            FallbackReason::PipelineOrComprehension => "comprehension or monadic do/for form",
            FallbackReason::InlineAssemblyBlock => "inline assembly block",
            FallbackReason::ComplexGenericBounds => "higher-ranked or where-clause bounds",
            FallbackReason::UnionIntersectionType => "union or intersection type expression",
            FallbackReason::ConditionalOrMappedType => "conditional or mapped type",
            FallbackReason::FunctionPointerOrCallableType => "function pointer or callable type",
            FallbackReason::AttributeOrDecoratorSyntax => "attribute or decorator with arguments",
            FallbackReason::VisibilityOrStorageModifier => "modifier order the reader does not accept",
            FallbackReason::MultipleReturnOrTupleSig => "multi-value return signature",
            FallbackReason::DefaultOrNamedParameter => "default or keyword-only parameter",
            FallbackReason::VariadicParameterSyntax => "variadic parameter",
            FallbackReason::AnonymousStructOrInterface => "inline struct or interface type",
            FallbackReason::DynamicOrComputedImport => "import of a computed name",
            FallbackReason::AliasedOrWildcardImport => "re-export or brace import set",
            FallbackReason::PreprocessorDirective => "preprocessor directive inside code",
            FallbackReason::MacroInvocationSyntax => "macro invocation form",
            FallbackReason::TemplateOrJsxSyntax => "embedded markup",
            FallbackReason::EmbeddedLanguageBlock => "SQL or CSS inside a template literal",
            FallbackReason::ShebangOrFileHeaderDirective => "file header directive",
            FallbackReason::CompilerIntrinsicOrBuiltin => "compiler intrinsic or builtin attribute",
            FallbackReason::MaxRecursionDepthExceeded => "nesting past the parser's stack guard",
            FallbackReason::MaxTokenLookaheadExceeded => "disambiguation scan past its lookahead cap",
            FallbackReason::LoopProgressStallGuard => "parser loop advanced zero tokens",
            FallbackReason::CorruptOrMinifiedInput => "minified or binary-looking source",
            FallbackReason::UnclassifiedSyntaxBailout => "no rule matched - add one",
            FallbackReason::DifferentialNameValue => "the lexer's value names a name the CST value lacks",
            FallbackReason::DifferentialCallValue => "the lexer's value calls or constants what the CST value lacks",
            FallbackReason::DifferentialConstantZero => "the CST wrote the value as the constant 0",
            FallbackReason::DifferentialTargetsDiffer => "the statement's assignment targets differ",
            FallbackReason::DifferentialMissingFact => "a fact the lexer recorded is absent from the CST record",
        }
    }
}

/// Every variant, in code order. Index here is the stats slot.
pub const REASONS: [FallbackReason; FallbackReason::COUNT] = [
    FallbackReason::InvalidUtf8Sequence,
    FallbackReason::UnterminatedStringLiteral,
    FallbackReason::UnterminatedMultilineString,
    FallbackReason::UnterminatedBlockComment,
    FallbackReason::UnknownEscapeSequence,
    FallbackReason::MalformedNumericLiteral,
    FallbackReason::UnrecognizedSigilOrToken,
    FallbackReason::StringInterpolationDepth,
    FallbackReason::HeredocDelimiterMismatch,
    FallbackReason::RegexLiteralAmbiguity,
    FallbackReason::UnmatchedClosingBrace,
    FallbackReason::UnclosedDelimiterAtEof,
    FallbackReason::DelimiterTypeMismatch,
    FallbackReason::IndentationDedentMismatch,
    FallbackReason::MissingStatementTerminator,
    FallbackReason::TernaryColonBlockConflict,
    FallbackReason::AngleBracketAmbiguity,
    FallbackReason::EmptyBlockUnexpected,
    FallbackReason::UnknownInfixOperator,
    FallbackReason::UnknownPrefixPostfixOp,
    FallbackReason::PrecedenceTableOverflow,
    FallbackReason::LambdaClosureSyntax,
    FallbackReason::DestructuringPatternFail,
    FallbackReason::SpreadRestOperatorContext,
    FallbackReason::RangeSliceSyntax,
    FallbackReason::PipelineOrComprehension,
    FallbackReason::InlineAssemblyBlock,
    FallbackReason::ComplexGenericBounds,
    FallbackReason::UnionIntersectionType,
    FallbackReason::ConditionalOrMappedType,
    FallbackReason::FunctionPointerOrCallableType,
    FallbackReason::AttributeOrDecoratorSyntax,
    FallbackReason::VisibilityOrStorageModifier,
    FallbackReason::MultipleReturnOrTupleSig,
    FallbackReason::DefaultOrNamedParameter,
    FallbackReason::VariadicParameterSyntax,
    FallbackReason::AnonymousStructOrInterface,
    FallbackReason::DynamicOrComputedImport,
    FallbackReason::AliasedOrWildcardImport,
    FallbackReason::PreprocessorDirective,
    FallbackReason::MacroInvocationSyntax,
    FallbackReason::TemplateOrJsxSyntax,
    FallbackReason::EmbeddedLanguageBlock,
    FallbackReason::ShebangOrFileHeaderDirective,
    FallbackReason::CompilerIntrinsicOrBuiltin,
    FallbackReason::MaxRecursionDepthExceeded,
    FallbackReason::MaxTokenLookaheadExceeded,
    FallbackReason::LoopProgressStallGuard,
    FallbackReason::CorruptOrMinifiedInput,
    FallbackReason::UnclassifiedSyntaxBailout,
    FallbackReason::DifferentialNameValue,
    FallbackReason::DifferentialCallValue,
    FallbackReason::DifferentialConstantZero,
    FallbackReason::DifferentialTargetsDiffer,
    FallbackReason::DifferentialMissingFact,
];

/// The 45 corpus languages, in a fixed order. A language id is its index + 1 so
/// that 0 can mean "unknown" without stealing a slot.
const LANG_NAMES: [&str; 45] = [
    "assembly", "awk", "bash", "c", "clojure", "cobol", "cpp", "crystal", "csharp",
    "d", "dart", "elixir", "elm", "erlang", "fortran", "fsharp", "go", "groovy",
    "haskell", "java", "javascript", "julia", "kotlin", "lua", "matlab", "nim",
    "objective-c", "ocaml", "pascal", "perl", "php", "powershell", "python", "r",
    "ruby", "rust", "scala", "solidity", "sql", "swift", "tcl", "typescript",
    "vbnet", "visual-basic", "zig",
];

/// Language id for a file path, from its suffix. 0 is unknown.
pub fn lang_id(path: &str) -> u16 {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let name = match ext.as_str() {
        "s" | "asm" => "assembly",
        "awk" => "awk",
        "sh" | "bash" => "bash",
        "c" | "h" => "c",
        "clj" | "cljs" | "cljc" => "clojure",
        "cob" | "cbl" => "cobol",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "cr" => "crystal",
        "cs" => "csharp",
        "d" => "d",
        "dart" => "dart",
        "ex" | "exs" => "elixir",
        "elm" => "elm",
        "erl" | "hrl" => "erlang",
        "f" | "f90" | "f95" => "fortran",
        "fs" | "fsi" | "fsx" => "fsharp",
        "go" => "go",
        "groovy" | "gradle" => "groovy",
        "hs" | "lhs" => "haskell",
        "java" => "java",
        "js" | "mjs" | "cjs" => "javascript",
        "jl" => "julia",
        "kt" | "kts" => "kotlin",
        "lua" => "lua",
        "m" => "matlab",
        "nim" => "nim",
        "mm" => "objective-c",
        "ml" | "mli" => "ocaml",
        "pas" | "pp" => "pascal",
        "pl" | "pm" => "perl",
        "php" => "php",
        "ps1" | "psm1" | "psd1" => "powershell",
        "py" => "python",
        "r" => "r",
        "rb" => "ruby",
        "rs" => "rust",
        "scala" | "sc" => "scala",
        "sol" => "solidity",
        "sql" => "sql",
        "swift" => "swift",
        "tcl" => "tcl",
        "ts" | "tsx" => "typescript",
        "vb" | "vbs" => "vbnet",
        "bas" => "visual-basic",
        "zig" => "zig",
        _ => return 0,
    };
    LANG_NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .map(|index| index as u16 + 1)
        .unwrap_or(0)
}

pub fn lang_name(id: u16) -> &'static str {
    LANG_NAMES.get(id as usize - 1).copied().unwrap_or("unknown")
}

/// Which part of a record the disagreement was about. The reason says what the
/// mechanism is; the channel says which fact kind to open first.
pub const CHANNELS: [&str; 8] = [
    "event",
    "call",
    "import",
    "symbol",
    "var",
    "values",
    "recovery",
    "other",
];

fn channel_index(kind: &str) -> usize {
    match kind {
        "event" => 0,
        "call" => 1,
        "import" => 2,
        "symbol" => 3,
        "var" => 4,
        "values" => 5,
        "recovery" => 6,
        _ => 7,
    }
}

/// Classify one guard refusal.
///
/// Deterministic and cheap: a handful of substring tests on a string that was
/// already built for the refusal message. Anything unrecognised is `Unclassified`
/// on purpose — that bucket is the signal to add a rule.
pub fn classify(kind: &str, detail: &str) -> FallbackReason {
    // Differential classes first: these strings come from the guard's own arms.
    if detail.contains("no event on a line both readers can mean") {
        return FallbackReason::MissingStatementTerminator;
    }
    if detail.contains("a name the lexer read is not in the CST's value") {
        return FallbackReason::DifferentialNameValue;
    }
    if detail.contains("a call or constant the lexer read is not in the CST's value") {
        return FallbackReason::DifferentialCallValue;
    }
    if detail.contains("the CST wrote the value as the constant 0") {
        return FallbackReason::DifferentialConstantZero;
    }
    if detail.contains("the statement's target list differs") {
        return FallbackReason::DifferentialTargetsDiffer;
    }
    if detail.contains("more than 1% is recovery noise") {
        return FallbackReason::UnclassifiedSyntaxBailout;
    }
    // A plain "the CST lost a <kind>" refusal is a missing fact.
    match kind {
        "call" | "import" | "symbol" | "var" | "event" => FallbackReason::DifferentialMissingFact,
        _ => FallbackReason::UnclassifiedSyntaxBailout,
    }
}

/// One captured fallback, small enough to keep three of.
#[derive(Clone, Copy, Default)]
pub struct Sample {
    pub line: u32,
    pub col: u16,
    pub loc: u32,
    /// Index into the thread-local path table.
    pub path: u16,
    pub channel: u8,
}

#[derive(Clone, Copy)]
struct ReasonStats {
    files: u64,
    loc: u64,
    channels: [u32; CHANNELS.len()],
    samples: [Sample; 3],
    kept: u8,
}

impl Default for ReasonStats {
    fn default() -> Self {
        ReasonStats {
            files: 0,
            loc: 0,
            channels: [0; CHANNELS.len()],
            samples: [Sample::default(); 3],
            kept: 0,
        }
    }
}

thread_local! {
    static STATS: RefCell<[ReasonStats; FallbackReason::COUNT]> =
        const { RefCell::new([ReasonStats { files: 0, loc: 0, channels: [0; CHANNELS.len()], samples: [Sample { line: 0, col: 0, loc: 0, path: 0, channel: 0 }; 3], kept: 0 }; FallbackReason::COUNT]) };
    /// Per-language counts: language id (stable, 0 = unknown) then reason.
    static LANGS: RefCell<HashMap<u16, [u64; FallbackReason::COUNT]>> = RefCell::new(HashMap::new());
    static PATHS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PATH_IDS: RefCell<HashMap<String, u16>> = RefCell::new(HashMap::new());
}

fn python_like_line(detail: &str) -> (u32, u16) {
    // The guard writes "line 12" into every detail; take the first one.
    let mut line = 0u32;
    let mut col = 0u16;
    let bytes = detail.as_bytes();
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..i + 5] == *b"line " {
            let mut n = 0u32;
            let mut j = i + 5;
            let mut any = false;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                n = n.saturating_mul(10).saturating_add((bytes[j] - b'0') as u32);
                j += 1;
                any = true;
            }
            if any {
                line = n;
                // "line 12 col 7" is the guard's column form when it has one.
                if j + 5 <= bytes.len() && bytes[j..j + 5] == *b" col " {
                    let mut c = 0u16;
                    let mut k = j + 5;
                    while k < bytes.len() && bytes[k].is_ascii_digit() {
                        c = c.saturating_mul(10).saturating_add((bytes[k] - b'0') as u16);
                        k += 1;
                    }
                    col = c;
                }
                break;
            }
        }
        i += 1;
    }
    (line, col)
}

fn path_id(path: &str) -> u16 {
    if path.is_empty() {
        return u16::MAX;
    }
    if let Some(id) = PATH_IDS.with(|ids| ids.borrow().get(path).copied()) {
        return id;
    }
    PATHS.with(|paths| {
        let mut paths = paths.borrow_mut();
        if paths.len() >= u16::MAX as usize {
            return u16::MAX;
        }
        let id = paths.len() as u16;
        paths.push(path.to_string());
        PATH_IDS.with(|ids| {
            ids.borrow_mut().insert(path.to_string(), id);
        });
        id
    })
}

/// Record one fallback. Called only from a refusal arm.
///
/// `loc` is the file's line count: a corpus of many small files and a corpus of
/// few large ones can carry the same file count and a different amount of code,
/// so both are counted.
pub fn record(path: &str, kind: &str, detail: &str, loc: u32, lang_id: u16) {
    let reason = classify(kind, detail);
    let channel = channel_index(kind);
    let (line, col) = python_like_line(detail);
    let sample = Sample {
        line,
        col,
        loc,
        path: path_id(path),
        channel: channel as u8,
    };
    let index = reason.index();
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        let bucket = &mut stats[index];
        bucket.files += 1;
        bucket.loc += loc as u64;
        bucket.channels[channel] += 1;
        if (bucket.kept as usize) < bucket.samples.len() {
            bucket.samples[bucket.kept as usize] = sample;
            bucket.kept += 1;
        }
    });
    if lang_id != 0 {
        LANGS.with(|langs| {
            let mut langs = langs.borrow_mut();
            let slot = langs.entry(lang_id).or_insert([0; FallbackReason::COUNT]);
            slot[index] += 1;
        });
    }
}

/// Wipe every counter. Tests only.
pub fn reset() {
    STATS.with(|stats| {
        let mut stats = stats.borrow_mut();
        for bucket in stats.iter_mut() {
            *bucket = ReasonStats::default();
        }
    });
    LANGS.with(|langs| langs.borrow_mut().clear());
    PATHS.with(|paths| paths.borrow_mut().clear());
    PATH_IDS.with(|ids| ids.borrow_mut().clear());
}

/// Total fallbacks recorded. Used by the acceptance test that every fallback
/// counter in the run has a matching event.
pub fn total() -> u64 {
    STATS.with(|stats| stats.borrow().iter().map(|bucket| bucket.files).sum())
}

fn sorted_reasons(snapshot: &[ReasonStats; FallbackReason::COUNT]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..FallbackReason::COUNT)
        .filter(|index| snapshot[*index].files > 0)
        .collect();
    order.sort_unstable_by(|left, right| {
        snapshot[*right]
            .loc
            .cmp(&snapshot[*left].loc)
            .then(snapshot[*right].files.cmp(&snapshot[*left].files))
            .then(left.cmp(right))
    });
    order
}

/// The `--report-fallbacks` table: ranked by LOC lost, then by files.
pub fn report() -> String {
    let snapshot = STATS.with(|stats| *stats.borrow());
    let total_files: u64 = snapshot.iter().map(|bucket| bucket.files).sum();
    let total_loc: u64 = snapshot.iter().map(|bucket| bucket.loc).sum();
    let mut out = String::new();
    out.push_str("FRCE — fallback root causes (ranked by LOC affected)\n");
    out.push_str(&format!(
        "  {total_files} fallbacks, {total_loc} LOC affected, {} distinct reasons\n\n",
        snapshot.iter().filter(|bucket| bucket.files > 0).count()
    ));
    if total_files == 0 {
        out.push_str("  none: every file took the CST path\n");
        return out;
    }
    out.push_str(&format!(
        "{:>4}  {:<34} {:>7} {:>7} {:>9}  {:<12} {}\n",
        "code", "reason", "files", "share", "loc_lost", "channel", "what a fix does"
    ));
    for index in sorted_reasons(&snapshot) {
        let bucket = &snapshot[index];
        let reason = FallbackReason::from_index(index).expect("index is in range");
        let share = 100.0 * bucket.files as f64 / total_files as f64;
        let channel = bucket
            .channels
            .iter()
            .enumerate()
            .max_by_key(|(i, count)| (**count, std::cmp::Reverse(*i)))
            .map(|(i, _)| CHANNELS[i])
            .unwrap_or("other");
        out.push_str(&format!(
            "{:>4}  {:<34} {:>7} {:>6.2}% {:>9}  {:<12} {}\n",
            reason.code(),
            reason.name(),
            bucket.files,
            share,
            bucket.loc,
            channel,
            reason.explanation()
        ));
    }
    let per_language = LANGS.with(|langs| {
        let langs = langs.borrow();
        let mut rows: Vec<(u16, u64, usize, u64)> = langs
            .iter()
            .map(|(id, counters)| {
                let files: u64 = counters.iter().sum();
