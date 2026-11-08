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
        b' ' | b'\t' | b'\r' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\x0c'));
            WHITESPACE
        }
        b'\n' => { c.bump(); NEWLINE }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        b'(' if c.peek_at(1) == b'*' => {
            crate::batch_d::lex::block_comment(c, b"(*", b"*)", true);
            COMMENT
        }
        b'"' => {
            if c.peek_at(1) == b'"' && c.peek_at(2) == b'"' {
                c.bump_n(3);
                loop {
                    if c.at_end() {
                        return BAD_STRING;
                    }
                    if c.starts_with(b"\"\"\"") {
                        c.bump_n(3);
                        return STRING;
                    }
                    c.bump_char();
                }
            }
            c.bump();
            if c.quoted(b'"', true) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'\'' => char_literal(c),
        b'0'..=b'9' => { crate::batch_d::lex::number(c); NUMBER }
        b'.' if c.peek_at(1).is_ascii_digit() => { c.bump(); crate::batch_d::lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'\'');
            keyword(&start[..length])
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b':' if c.peek_at(1) == b':' => { c.bump_n(2); COLON_COLON }
        b':' => { c.bump(); COLON }
        b'.' => { c.bump(); DOT }
        b'|' if c.peek_at(1) == b'>' => { c.bump_n(2); PIPE_RIGHT }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); BARBAR }
        b'|' => { c.bump(); BAR }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AMPAMP }
        b'&' => { c.bump(); AMP }
        b'=' => { c.bump(); ASSIGN }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'>' => { c.bump_n(2); NE }
        b'<' if c.peek_at(1) == b'-' => { c.bump_n(2); BACK_ARROW }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' if c.peek_at(1) == b'>' => { c.bump_n(2); COMPOSE }
        b'>' => { c.bump(); GT }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'^' => { c.bump(); CARET }
        _ => UNKNOWN,
    }
}

/// `'a'` and `'\n'`; a lone `'` inside a name is part of the name and never
/// reaches here.
fn char_literal(c: &mut Cursor) -> Kind {
    c.bump();
    if c.at_end() {
        return BAD_STRING;
    }
    if c.peek() == b'\\' {
        c.bump_n(2);
    } else {
        c.bump_char();
    }
    if c.peek() == b'\'' {
        c.bump();
        CHAR
    } else {
        BAD_STRING
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"module" => KW_MODULE,
        b"namespace" => KW_NAMESPACE,
        b"open" => KW_OPEN,
        b"let" => KW_LET,
        b"rec" => KW_REC,
        b"mutable" => KW_MUTABLE,
        b"type" => KW_TYPE,
        b"match" => KW_MATCH,
        b"with" => KW_WITH,
        b"if" => KW_IF,
        b"then" => KW_THEN,
        b"else" => KW_ELSE,
        b"elif" => KW_ELIF,
        b"for" => KW_FOR,
        b"in" => KW_IN,
        b"to" => KW_TO,
        b"downto" => KW_DOWNTO,
        b"do" => KW_DO,
        b"done" => KW_DONE,
        b"while" => KW_WHILE,
        b"fun" => KW_FUN,
        b"function" => KW_FUNCTION,
        b"try" => KW_TRY,
        b"finally" => KW_FINALLY,
        b"of" => KW_OF,
        b"as" => KW_AS,
        b"when" => KW_WHEN,
        b"and" => KW_AND,
        b"or" => KW_OR,
        b"not" => KW_NOT,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        b"null" => KW_NULL,
        b"yield" => KW_YIELD,
        b"return" => KW_RETURN,
        b"int" => KW_INT,
        b"float" => KW_FLOAT,
        b"string" => KW_STRING_TYPE,
        b"bool" => KW_BOOL,
        b"unit" => KW_UNIT,
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_f::harness;

    const VALID: &[(&str, &[&str])] = &[
        (
            "Program.fs",
            &[
                "module_declaration",
                "open_declaration",
                "let_binding",
                "let_binding",
                "let_binding",
                "expression_statement",
            ],
        ),
        (
            "Types.fs",
            &["module_declaration", "type_declaration", "type_declaration", "type_declaration", "let_binding"],
        ),
        (
            "Pipeline.fs",
            &[
                "module_declaration",
                "let_binding",
                "let_binding",
                "let_binding",
                "expression_statement",
                "expression_statement",
                "let_binding",
                "let_binding",
            ],
        ),
    ];

    #[test]
    fn valid_fixtures() {
        harness::check_valid("fsharp", &FSHARP, VALID);
    }

    #[test]
    fn malformed_keeps_the_root() {
        harness::check_malformed(
            "malformed",
            "fsharp_broken.fs",
            &FSHARP,
            &["module_declaration", "let_binding", "let_binding", "let_binding"],
        );
    }

    #[test]
    #[ignore]
    fn dump_top_level_kinds() {
        harness::dump("fsharp", &FSHARP);
        harness::dump("malformed", &FSHARP);
    }

    #[test]
    fn lexes_a_char_a_string_and_a_comment() {
        use crate::deep::lexer::{lex, Tokens};
        let src = b"let c = 'a'\nlet s = \"hi\" // note\n";
        let mut tokens = Tokens::default();
        lex(src, &FSHARP, &mut tokens);
        let kinds: Vec<&str> = (0..tokens.len())
            .map(|i| FSHARP.name(tokens.kind[i]))
            .filter(|kind| !matches!(*kind, "whitespace" | "newline" | "indent" | "dedent"))
            .collect();
        assert_eq!(kinds, ["let", "identifier", "=", "char", "let", "identifier", "=", "string_literal", "comment"]);
    }
}
