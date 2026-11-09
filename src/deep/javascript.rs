//! JavaScript, TypeScript and TSX through one `grammar!`: Slice 4. TypeScript is
//! a superset of JavaScript, so the grammar is TypeScript's, and the three
//! languages differ only in the lexer: JavaScript and TSX read `<` where an
//! operand starts as a JSX element, TypeScript as a type assertion `<T>x`.
//! Node names follow Tree-sitter's TypeScript (a superset of its JavaScript)
//! where that costs nothing; the tree is not the same tree.
//!
//! Choices that make it LL(3):
//!
//! - Semicolons are optional everywhere a statement may end, and an expression
//!   takes every operator that can continue it, across lines. That is automatic
//!   semicolon insertion for real code, where `a\n(b)` is a call too. A
//!   restricted production (`return` and a value on the next line) reads as one
//!   statement here, not two.
//! - A template literal and a JSX element are one token each; the lexer scans
//!   their `${...}` and `{...}` parts with the braces balanced. A regular
//!   expression is a token where an operand may start: after anything that
//!   cannot end an operand ([`Cursor::previous`]).
//! - `>` right before `>` is `GT_JOINED`, so `Map<K, Set<V>>` closes two lists
//!   and `a >> b` is `GT_JOINED ">"`, as in the Rust grammar.
//! - What only a longer look tells apart is decided by predicates: an arrow
//!   function's head `(a, b) =>`, a generic call `f<T>(x)`, `for (x of y)`,
//!   statements that start with a contextual keyword (`type X = ...`), and a
//!   member's modifiers (`get x() {}` against a method named `get`).
//! - Statements use `_expression_st`, whose first operand is not `{`,
//!   `function` or `class`: those start a block or a declaration there.

use super::lexer::Cursor;
use super::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { TYPESCRIPT {
    name = "typescript";
    lex = lex_typescript;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        PRIVATE = "private_property_identifier",
        NUMBER = "number",
        STRING = "string",
        TEMPLATE = "template_string",
        REGEX = "regex",
        /// A whole JSX element or fragment, in JavaScript and TSX.
        JSX = "jsx_element",
        /// A line break that ends a statement: after `break`, `continue`, `return`
        /// or `yield`, and after an operand when the next line starts with `[`,
        /// `++` or `--`. Elsewhere a line break is whitespace.
        LINE_END = "line_end",
        /// A string, template, regular expression or JSX element with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CLASS = "class",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DEBUGGER = "debugger",
        KW_DEFAULT = "default",
        KW_DELETE = "delete",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXPORT = "export",
        KW_EXTENDS = "extends",
        KW_FALSE = "false",
        KW_FINALLY = "finally",
        KW_FOR = "for",
        KW_FUNCTION = "function",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_IN = "in",
        KW_INSTANCEOF = "instanceof",
        KW_NEW = "new",
        KW_NULL = "null",
        KW_RETURN = "return",
        KW_SUPER = "super",
        KW_SWITCH = "switch",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPEOF = "typeof",
        KW_VAR = "var",
        KW_VOID = "void",
        KW_WHILE = "while",
        KW_WITH = "with",
        KW_LET = "let",
        KW_YIELD = "yield",
        KW_AWAIT = "await",
        /// Contextual keywords: names everywhere but where they mean something.
        KW_ASYNC = "async",
        KW_GET = "get",
        KW_SET = "set",
        KW_STATIC = "static",
        KW_OF = "of",
        KW_FROM = "from",
        KW_AS = "as",
        KW_TYPE = "type",
        KW_INTERFACE = "interface",
        KW_NAMESPACE = "namespace",
        KW_MODULE = "module",
        KW_DECLARE = "declare",
        KW_ABSTRACT = "abstract",
        KW_READONLY = "readonly",
        KW_PUBLIC = "public",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_IMPLEMENTS = "implements",
        KW_KEYOF = "keyof",
        KW_INFER = "infer",
        KW_IS = "is",
        KW_ASSERTS = "asserts",
        KW_UNIQUE = "unique",
        KW_SATISFIES = "satisfies",
        KW_OVERRIDE = "override",
        KW_ACCESSOR = "accessor",
        KW_GLOBAL = "global",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        ELLIPSIS = "...",
        OPTIONAL_CHAIN = "?.",
        QUESTION = "?",
        NULLISH = "??",
        COLON = ":",
        ARROW = "=>",
