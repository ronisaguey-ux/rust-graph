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
