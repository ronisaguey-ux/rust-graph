//! Lua: the language of configuration files and game scripts.
//!
//! Statements are separated by nothing at all, so a block is a plain sequence
//! of statements and an expression statement carries its own optional `=`
//! assignment: `x = 1` and `f(x)` are both one `expression_statement`, and the
//! call itself is a `call` node made by the expression table.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **`local x <const>`, `<close>`** attributes are not modelled.
//! - **A table field may be separated by `;` as Lua allows, but a `;` after the
//!   last field is not modelled: the field list's separator is `,` alone.
//! - **Long brackets** `[[ ... ]]` and `[=[ ... ]=]` are one string token, and
//!   so are their comments.
//! - **`goto`/`::label::`** are statement keywords without a node of their own:
//!   `goto` takes a name, a label is a statement of one token.
//! - **Operator precedence of `..` against unary `#`** follows Lua's table; a
//!   `..` chain associates to the right as Lua's manual says.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { LUA {
    name = "lua";
    lex = lex_lua;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        COMMA = ",",
        SEMI = ";",
        DOT = ".",
        COLON = ":",
        LABEL_COLON = "::",
        ASSIGN = "=",
        EQ = "==",
        NE = "~=",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        SLASHSLASH = "//",
        PERCENT = "%",
        CARET = "^",
        HASH = "#",
        AMP = "&",
        PIPE = "|",
        TILDE = "~",
        SHL = "<<",
        SHR = ">>",
        CONCAT = "..",
        ELLIPSIS = "...",
        NAME = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_AND = "and",
        KW_BREAK = "break",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ELSEIF = "elseif",
        KW_END = "end",
        KW_FALSE = "false",
        KW_FOR = "for",
        KW_FUNCTION = "function",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_IN = "in",
        KW_LOCAL = "local",
        KW_NIL = "nil",
        KW_NOT = "not",
        KW_OR = "or",
        KW_REPEAT = "repeat",
        KW_RETURN = "return",
        KW_THEN = "then",
        KW_TRUE = "true",
        KW_UNTIL = "until",
        KW_WHILE = "while",
    }

    chunk: _statement*,

    _statement:
        local_function
        | local_declaration
        | function_declaration
        | if_statement
        | for_statement
        | while_statement
        | repeat_statement
        | do_statement
        | return_statement
        | break_statement
        | goto_statement
        | label_statement
        | expression_statement,

    block: _statement*,

    local_function: "local" "function" NAME function_body,
    local_declaration: "local" namelist ["=" expression_list],
    namelist: NAME ("," NAME)*,

    function_declaration: "function" function_name function_body,
    function_name: NAME ("." NAME)* [":" NAME],
    function_body: parameters block "end",
    parameters: "(" [parameter ("," parameter)* ["," "..."]] ")",
    parameter: NAME | "...",

    if_statement: "if" _expression "then" block (elseif_clause)* [else_clause] "end",
    elseif_clause: "elseif" _expression "then" block,
    else_clause: "else" block,

    for_statement: for_numeric | for_generic,
    for_numeric: "for" NAME "=" _expression "," _expression ["," _expression] "do" block "end",
    for_generic: "for" namelist "in" expression_list "do" block "end",

    while_statement: "while" _expression "do" block "end",
    repeat_statement: "repeat" block "until" _expression,
    do_statement: "do" block "end",

    return_statement: "return" [expression_list],
    break_statement: "break",
    goto_statement: "goto" NAME,
    label_statement: "::" NAME "::",

    expression_statement: _expression [assignment_tail],
    assignment_tail: "=" expression_list,
    expression_list: _expression ("," _expression)*,

    _expression: precedence _primary {
        left "or" => binary_expression,
        left "and" => binary_expression,
        left "==" | "~=" | "<" | ">" | "<=" | ">=" => comparison,
        left "|" => binary_expression,
        left "~" => binary_expression,
        left "&" => binary_expression,
        left "<<" | ">>" => binary_expression,
        right ".." => concatenation,
        left "+" | "-" => arithmetic,
        left "*" | "/" | "//" | "%" => arithmetic,
        prefix "not" | "-" | "#" => unary_expression,
        right "^" => arithmetic,
        postfix call_arguments => call,
        postfix method_call_tail => method_call,
        postfix index_expression => index,
        postfix member_expression => member,
    },

    method_call_tail: ":" NAME call_arguments,
    index_expression: "[" _expression "]",
    member_expression: "." NAME,

    _primary:
        NAME
        | NUMBER
        | STRING
        | "nil"
        | "true"
        | "false"
        | "..."
        | table_constructor
        | function_expression
        | paren_expression,

    paren_expression: "(" _expression ")",
    call_arguments: paren_arguments | table_constructor | STRING,
    paren_arguments: "(" [expression_list] ")",

    table_constructor: "{" [_field_list] "}",
    _field_list: _field %? ",",
    _field: named_field | keyed_field | _expression,
    named_field: NAME "=" _expression,
    keyed_field: bracket_key "=" _expression,
    bracket_key: "[" _expression "]",

    function_expression: "function" function_body,
} }

pub fn lex_lua(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'-' if c.peek_at(1) == b'-' => {
            c.bump_n(2);
            if c.peek() == b'[' {
                long_bracket(c);
            }
            lex::line_comment(c);
            COMMENT
        }
        b'[' if matches!(c.peek_at(1), b'[' | b'=') => {
            if long_bracket(c) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'\'' | b'"' => {
            if lex::quoted(c, byte) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'0'..=b'9' => {
            lex::number(c);
            NUMBER
        }
        b'.' if c.peek_at(1).is_ascii_digit() => {
            c.bump();
            lex::number(c);
            NUMBER
        }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
            keyword(&start[..length], NAME)
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'.' if c.peek_at(1) == b'.' && c.peek_at(2) == b'.' => {
            c.bump_n(3);
            ELLIPSIS
        }
        b'.' if c.peek_at(1) == b'.' => {
            c.bump_n(2);
            CONCAT
        }
        b'.' => { c.bump(); DOT }
        b':' if c.peek_at(1) == b':' => {
            c.bump_n(2);
            LABEL_COLON
        }
        b':' => { c.bump(); COLON }
        b'=' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            EQ
        }
        b'=' => { c.bump(); ASSIGN }
        b'~' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            NE
        }
        b'~' => { c.bump(); TILDE }
        b'<' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            LE
        }
        b'<' if c.peek_at(1) == b'<' => {
            c.bump_n(2);
            SHL
        }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            GE
        }
        b'>' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            SHR
        }
        b'>' => { c.bump(); GT }
        b'+' => { c.bump(); PLUS }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' if c.peek_at(1) == b'/' => {
            c.bump_n(2);
            SLASHSLASH
        }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'^' => { c.bump(); CARET }
        b'#' => { c.bump(); HASH }
        b'&' => { c.bump(); AMP }
        b'|' => { c.bump(); PIPE }
        _ => UNKNOWN,
    }
}

/// From `[`, through the matching `]=*]` of a long bracket. True if it closed.
fn long_bracket(c: &mut Cursor) -> bool {
    let start = c.rest();
    let mut level = 0;
    while start.get(1 + level) == Some(&b'=') {
        level += 1;
    }
    if start.get(1 + level) != Some(&b'[') {
        return false;
    }
    c.bump_n(2 + level);
    loop {
        if c.at_end() {
            return false;
        }
        if c.peek() == b']' {
            let mut close = 0;
            while c.peek_at(1 + close) == b'=' {
                close += 1;
            }
            if close == level && c.peek_at(1 + close) == b']' {
                c.bump_n(2 + close);
                return true;
            }
        }
        c.bump_char();
    }
}

fn keyword(text: &[u8], otherwise: Kind) -> Kind {
    match text {
        b"and" => KW_AND, b"break" => KW_BREAK, b"do" => KW_DO,
        b"else" => KW_ELSE, b"elseif" => KW_ELSEIF, b"end" => KW_END,
        b"false" => KW_FALSE, b"for" => KW_FOR, b"function" => KW_FUNCTION,
        b"goto" => KW_GOTO, b"if" => KW_IF, b"in" => KW_IN,
        b"local" => KW_LOCAL, b"nil" => KW_NIL, b"not" => KW_NOT,
        b"or" => KW_OR, b"repeat" => KW_REPEAT, b"return" => KW_RETURN,
        b"then" => KW_THEN, b"true" => KW_TRUE, b"until" => KW_UNTIL,
        b"while" => KW_WHILE,
        _ => otherwise,
    }
}
