//! F#: modules, `let` bindings, type declarations and `match` expressions, on
//! the runtime's inferred layout tokens.
//!
//! F# is offside-sensitive: a binding's body is the indented block under it, so
//! the grammar uses `layout = NEWLINE INDENT DEDENT;` exactly as the Python one
//! does and consumes `INDENT`/`DEDENT` in `block`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Application is juxtaposition only where a predicate can see it.** `f x`
//!   is an `application`, entered through `&(IDENT IDENT)`, `&(IDENT NUMBER)`
//!   and `&(IDENT STRING)`. `f (x)` is a call through the parenthesised form.
//!   A partial application whose argument starts with a bracket is not modelled.
//! - **`if`/`then`/`else` is the one-line form only.** The offside form (`then`
//!   on its own line with an indented body) is not modelled.
//! - **Classes, interfaces, members, `use`, `async`/`task` and computation
//!   expressions** are not modelled; `type` covers records, unions and
//!   abbreviations.
//! - **Active patterns, units of measure and inline IL** are not modelled.
//! - **A `match` pattern is parsed as an expression**, so binding patterns
//!   (`Some x`) are an application node rather than a pattern node.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { FSHARP {
    name = "fsharp";
    lex = lex_fsharp;
    layout = NEWLINE INDENT DEDENT;

    tokens {
        NEWLINE = "newline",
        INDENT = "indent" open block,
        DEDENT = "dedent" close block,
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        COMMA = ",",
        SEMI = ";",
        COLON = ":",
        DOT = ".",
        BAR = "|",
        ARROW = "->",
        ASSIGN = "=",
        NE = "<>",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        CARET = "^",
        AMP = "&",
        AMPAMP = "&&",
        BARBAR = "||",
        PIPE_RIGHT = "|>",
        COMPOSE = ">>",
        COLON_COLON = "::",
        BACK_ARROW = "<-",
        UNDERSCORE = "_",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string_literal",
        CHAR = "char",
        BAD_STRING = "bad_string" bad,

        KW_MODULE = "module",
        KW_NAMESPACE = "namespace",
        KW_OPEN = "open",
        KW_LET = "let",
        KW_REC = "rec",
        KW_MUTABLE = "mutable",
        KW_TYPE = "type",
        KW_MATCH = "match",
        KW_WITH = "with",
        KW_IF = "if",
        KW_THEN = "then",
        KW_ELSE = "else",
        KW_ELIF = "elif",
        KW_FOR = "for",
        KW_IN = "in",
        KW_TO = "to",
        KW_DOWNTO = "downto",
        KW_DO = "do",
        KW_DONE = "done",
        KW_WHILE = "while",
        KW_FUN = "fun",
        KW_FUNCTION = "function",
        KW_TRY = "try",
        KW_FINALLY = "finally",
        KW_OF = "of",
        KW_AS = "as",
        KW_WHEN = "when",
        KW_AND = "and",
        KW_OR = "or",
        KW_NOT = "not",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_NULL = "null",
        KW_YIELD = "yield",
        KW_RETURN = "return",
        KW_INT = "int",
        KW_FLOAT = "float",
        KW_STRING_TYPE = "string",
        KW_BOOL = "bool",
        KW_UNIT = "unit",
    }

    module_file: _line*,

    _line: NEWLINE | _declaration,

    _declaration:
        module_declaration
        | namespace_declaration
        | open_declaration
        | let_binding
        | type_declaration
        | expression_statement,

    module_declaration: "module" qualified_name NEWLINE,
    namespace_declaration: "namespace" qualified_name NEWLINE,
    open_declaration: "open" qualified_name NEWLINE,
    qualified_name: IDENT ("." IDENT)*,

    let_binding: "let" ["rec"] ["mutable"] binding_pattern "=" let_body,
    binding_pattern: IDENT _argument_pattern*,
    _argument_pattern: IDENT | UNDERSCORE | parenthesized_pattern,
    parenthesized_pattern: "(" [_argument_pattern %? ","] ")",

    let_body: _expression [NEWLINE] | NEWLINE block,
    block: INDENT _line* DEDENT,

    type_declaration: "type" IDENT [type_parameters] "=" type_body,
    type_parameters: "<" type_parameter ("," type_parameter)* ">",
    type_parameter: IDENT,
    type_body:
        record_type
        | NEWLINE type_block
        | &(IDENT "of") union_type
        | &("|" IDENT) union_type
        | abbreviation,
    record_type: record_body NEWLINE,
    type_block: INDENT (union_type | record_type | abbreviation) DEDENT,
    record_body: "{" (_line | record_field)* "}",
    record_field: IDENT ":" _type [NEWLINE | ";"],
    /// One case per line: a separator that may also be the line break cannot
    /// tell the last case's newline from the start of another.
    union_type: union_case_line+,
    union_case_line: ["|"] union_case NEWLINE,
    union_case: IDENT ["of" _type],
    abbreviation: _type NEWLINE,

    _type: qualified_name | "int" | "float" | "string" | "bool" | "unit",

    /// A `match` expression ends with the newline of its last clause, so the
    /// statement's own line break is optional here.
    expression_statement: _expression [NEWLINE],

    _expression: precedence _primary {
        left "|>" => pipeline,
        left ">>" => composition,
        left "||" => logical_or,
        left "&&" => logical_and,
        left "=" | "<>" | "<" | ">" | "<=" | ">=" => comparison,
        left "::" => cons,
        left "+" | "-" => additive,
        left "*" | "/" | "%" => multiplicative,
        right "^" => power,
        prefix "-" | "not" => unary,
        postfix argument_list => call,
        postfix "." IDENT => member_access,
    },

    argument_list: "(" [_argument %? ","] ")",
    _argument: _expression,

    _primary:
        &(IDENT IDENT) application
        | &(IDENT NUMBER) application
        | &(IDENT STRING) application
        | atom,

    application: qualified_name atom+,
    atom:
        qualified_name
        | NUMBER
        | STRING
        | CHAR
        | "true"
        | "false"
        | "null"
        | parenthesized_expression
        | list_literal
        | match_expression
        | if_expression
        | lambda,

    parenthesized_expression: "(" _expression ")",
    list_literal: "[" [_expression %? ";"] "]",
    lambda: "fun" binding_pattern "->" _expression,
    if_expression: "if" _expression "then" _expression ["else" _expression],
    /// A clause list may sit at the same column as `match` (no INDENT) or be
    /// indented under it, where the layout pass inserts both tokens.
    match_expression: "match" _expression "with" NEWLINE (indented_match_block | flat_match_block),
    indented_match_block: INDENT match_clause+ DEDENT,
    flat_match_block: match_clause+,
    match_clause: ["|"] _expression "->" clause_body,
    clause_body: _expression NEWLINE | NEWLINE block,
} }

pub fn lex_fsharp(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    match byte {
