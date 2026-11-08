//! Zig: functions, `const`/`var` declarations, `struct`/`enum`/`union` types,
//! `comptime` blocks and the statements a scanner sees.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Comptime type expressions** are ordinary expressions; `comptime` is a
//!   keyword only where a statement or a parameter starts with it.
//! - **`@builtin(...)`** is one `builtin` node around a call, without a rule per
//!   builtin.
//! - **`T{ ... }` initialisers** are not modelled: the `. { ... }` form is, so
//!   `var c: Config = .{ .depth = 8 };` parses and `var c = Config{};` does not.
//! - **Payload captures** (`|x| ...`, `errdefer |e|`) are not modelled.
//! - **`usingnamespace`, `test`, `asm`** blocks are declarations; an `asm` body
//!   is not parsed as Zig.
//! - **Error sets and unions** (`!`, `error{...}`) are operators, not their own
//!   nodes.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { ZIG {
    name = "zig";
    lex = lex_zig;

    tokens {
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
        DOT2 = "..",
        ELLIPSIS = "...",
        AT = "@",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
        PLUS = "+",
        PLUS_EQ = "+=",
        MINUS = "-",
        MINUS_EQ = "-=",
        STAR = "*",
        STAR_EQ = "*=",
        SLASH = "/",
        SLASH_EQ = "/=",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        TILDE = "~",
        BANG = "!",
        QUESTION = "?",
        LSHIFT = "<<",
        RSHIFT = ">>",
        ARROW = "->",
        FAT_ARROW = "=>",
        OR = "or",
        AND = "and",
        IDENT = "identifier",
        /// `name:` — a parameter or a struct field, colon included.
        FIELD_NAME = "field_name",
        NUMBER = "number",
        STRING = "string",
        CHAR = "char_literal",
        BAD_STRING = "bad_string" bad,

        KW_FN = "fn",
        KW_CONST = "const",
        KW_VAR = "var",
        KW_PUB = "pub",
        KW_EXPORT = "export",
        KW_EXTERN = "extern",
        KW_INLINE = "inline",
        KW_STRUCT = "struct",
        KW_ENUM = "enum",
        KW_UNION = "union",
        KW_OPAQUE = "opaque",
        KW_ERROR = "error",
        KW_IF = "if",
        KW_ELSE = "else",
        KW_WHILE = "while",
        KW_FOR = "for",
        KW_SWITCH = "switch",
        KW_RETURN = "return",
        KW_BREAK = "break",
        KW_CONTINUE = "continue",
        KW_DEFER = "defer",
        KW_ERRDEFER = "errdefer",
        KW_TRY = "try",
        KW_CATCH = "catch",
        KW_COMPTIME = "comptime",
        KW_TEST = "test",
        KW_USINGNAMESPACE = "usingnamespace",
        KW_ASYNC = "async",
        KW_AWAIT = "await",
        KW_ORELSE = "orelse",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_NULL = "null",
        KW_UNDEFINED = "undefined",
    }

    program: _declaration*,

    _declaration: [_visibility] _declaration_body,
    _declaration_body:
        fn_declaration
        | container_declaration
        | var_declaration
        | test_declaration
        | usingnamespace_declaration
        | comptime_declaration
        | expression_statement,

    fn_declaration: "fn" IDENT parameters [_expression] block,
    parameters: "(" [(parameter %? ",")] ")",
    parameter: ["comptime"] _name_and_type ["..."],
    _name_and_type: FIELD_NAME _expression | IDENT,

    container_declaration: ("struct" | "enum" | "union") container_body,
    container_body: "{" _container_item* "}",
    _container_item: _declaration | field_declaration | ",",
    field_declaration: FIELD_NAME _expression,
    var_declaration: ("const" | "var") _name_and_type ["=" _initializer] [";"],
    _initializer: container_declaration | _expression,
    test_declaration: "test" [STRING] block,
    usingnamespace_declaration: "usingnamespace" _expression [";"],
    comptime_declaration: "comptime" block,

    _visibility: ["pub"] ["export"] ["extern"] ["inline"],

    block: "{" _statement* "}",

    _statement:
        _declaration
        | if_statement
        | while_statement
        | for_statement
        | switch_statement
        | return_statement
        | break_statement
        | continue_statement
        | defer_statement,

    if_statement: "if" _expression block ["else" (if_statement | block)],
    while_statement: "while" paren_expression [":" paren_expression] block,
    for_statement: "for" paren_expression block,
    switch_statement: "switch" _expression switch_body,
    switch_case: _expression "=>" _statement,
    return_statement: "return" [_expression] [";"],
    break_statement: "break" [":" IDENT] [";"],
    continue_statement: "continue" [";"],
    defer_statement: ("defer" | "errdefer") _statement,

    expression_statement: _expression [";"],

    _expression: precedence _primary {
        right "=" | "+=" | "-=" | "*=" | "/=" => assignment,
        left "or" | "orelse" => logical_expression,
        left "and" => logical_expression,
        left "==" | "!=" | "<" | ">" | "<=" | ">=" => comparison,
        left "&" => bitwise,
        left "|" => bitwise,
        left "^" => bitwise,
        left "<<" | ">>" => shift,
        left "+" | "-" => arithmetic,
        left "*" | "/" | "%" => arithmetic,
        prefix "!" | "-" | "~" | "try" | "await" | "&" | "*" => unary,
        postfix call_arguments => call,
        postfix index_arguments => index,
        postfix "." IDENT => member,
    },

    call_arguments: "(" [(argument %? ",")] ")",
    argument: field_initializer | _expression,
    field_initializer: "." IDENT "=" _expression,
    index_arguments: "[" [_expression] "]",
    builtin_name: "@" IDENT,
    builtin_call: builtin_name call_arguments,

    _primary:
        IDENT
        | NUMBER
        | STRING
        | CHAR
        | "true"
        | "false"
        | "null"
        | "undefined"
        | paren_expression
        | array_literal
        | struct_literal
        | builtin_call,

    paren_expression: "(" [(argument %? ",")] ")",
    array_literal: bracket_array [type_suffix],
    bracket_array: "[" [array_items] "]",
    type_suffix: ["const"] _primary,
    array_items: _expression [( ":" _expression)* ] [";" _expression],
    struct_literal: "." struct_body,
    struct_body: "{" [(argument %? ",")] "}",
    switch_body: "{" switch_case* "}",
} }

pub fn lex_zig(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => { lex::line_comment(c); COMMENT }
        b'\\' if c.peek_at(1) == b'\\' => { lex::line_comment(c); COMMENT }
        b'"' => if lex::quoted(c, b'"') { STRING } else { BAD_STRING },
        b'\'' => {
            let closed = {
                c.bump();
                c.quoted(b'\'', true)
            };
            if closed { CHAR } else { BAD_STRING }
        }
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'@' => { c.bump(); AT }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
            let kind = keyword(&start[..length]);
            if kind == IDENT && c.peek() == b':' && c.peek_at(1) != b':' {
                c.bump();
                return FIELD_NAME;
            }
            kind
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b':' => { c.bump(); COLON }
        b'.' if c.peek_at(1) == b'.' && c.peek_at(2) == b'.' => { c.bump_n(3); ELLIPSIS }
        b'.' if c.peek_at(1) == b'.' => { c.bump_n(2); DOT2 }
        b'.' => { c.bump(); DOT }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' if c.peek_at(1) == b'>' => { c.bump_n(2); FAT_ARROW }
        b'=' => { c.bump(); ASSIGN }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' => { c.bump(); BANG }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'<' => { c.bump_n(2); LSHIFT }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' if c.peek_at(1) == b'>' => { c.bump_n(2); RSHIFT }
        b'>' => { c.bump(); GT }
        b'+' if c.peek_at(1) == b'=' => { c.bump_n(2); PLUS_EQ }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'-' if c.peek_at(1) == b'=' => { c.bump_n(2); MINUS_EQ }
        b'-' => { c.bump(); MINUS }
        b'*' if c.peek_at(1) == b'=' => { c.bump_n(2); STAR_EQ }
        b'*' => { c.bump(); STAR }
        b'/' if c.peek_at(1) == b'=' => { c.bump_n(2); SLASH_EQ }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'&' => { c.bump(); AMP }
        b'|' => { c.bump(); PIPE }
        b'^' => { c.bump(); CARET }
        b'~' => { c.bump(); TILDE }
        b'?' => { c.bump(); QUESTION }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"fn" => KW_FN,
        b"const" => KW_CONST,
        b"var" => KW_VAR,
        b"pub" => KW_PUB,
        b"export" => KW_EXPORT,
        b"extern" => KW_EXTERN,
        b"inline" => KW_INLINE,
        b"struct" => KW_STRUCT,
        b"enum" => KW_ENUM,
        b"union" => KW_UNION,
        b"opaque" => KW_OPAQUE,
        b"error" => KW_ERROR,
        b"if" => KW_IF,
        b"else" => KW_ELSE,
        b"while" => KW_WHILE,
        b"for" => KW_FOR,
        b"switch" => KW_SWITCH,
        b"return" => KW_RETURN,
        b"break" => KW_BREAK,
        b"continue" => KW_CONTINUE,
        b"defer" => KW_DEFER,
        b"errdefer" => KW_ERRDEFER,
        b"try" => KW_TRY,
        b"catch" => KW_CATCH,
        b"comptime" => KW_COMPTIME,
        b"test" => KW_TEST,
        b"usingnamespace" => KW_USINGNAMESPACE,
        b"async" => KW_ASYNC,
        b"await" => KW_AWAIT,
        b"orelse" => KW_ORELSE,
        b"or" => OR,
        b"and" => AND,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        b"null" => KW_NULL,
        b"undefined" => KW_UNDEFINED,
        _ => IDENT,
    }
}
