//! Erlang: forms (attributes and functions, each ending with `.`), function
//! clauses (separated by `;`), and expressions (separated by `,`).
//!
//! Erlang's punctuation does the work a line-based language does with layout:
//! `.` ends a form, `;` ends a clause, `,` separates expressions, and `end`
//! closes `case`/`if`/`receive`/`fun`/`begin`. That makes every body a plain
//! repetition with an explicit terminator, so the grammar needs no lookahead
//! tricks.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Records (`#rec{field = V}`)** and **binaries (`<<A:8>>`)** are not
//!   modelled; a `#` or `<<` is an error where it stands.
//! - **`fun` clauses and `receive`** are modelled as keyword expressions with a
//!   clause list, without the message patterns.
//! - **A guard sequence** is a `;`-separated list inside a clause head, not a
//!   guard node.
//! - **Bit syntax inside a binary** is not modelled at all.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { ERLANG {
    name = "erlang";
    lex = lex_erlang;

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
        ARROW = "->",
        PIPE = "|",
        LIST_SEP = "||",
        ASSIGN = "=",
        ASSOC = "=>",
        UPDATE = ":=",
        DOUBLE_COLON = "::",
        GENERATOR = "<-",
        EQ = "==",
        EXACTLY = "=:=",
        NE = "/=",
        NOT_EXACTLY = "=/=",
        LE = "=<",
        LT = "<",
        GE = ">=",
        GT = ">",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        DIV = "div",
        REM = "rem",
        AND = "and",
        OR = "or",
        BAND = "band",
        BOR = "bor",
        BXOR = "bxor",
        BSL = "bsl",
        BSR = "bsr",
        NOT = "not",
        PLUS_PLUS = "++",
        MINUS_MINUS = "--",
        SEND = "!",
        HASH = "#",
        QUESTION = "?",
        ATOM = "atom",
        NAME = "name",
        VARIABLE = "variable",
        QUOTED_ATOM = "quoted_atom",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_MODULE = "module",
        KW_EXPORT = "export",
        KW_IMPORT = "import",
        KW_BEHAVIOUR = "behaviour",
        KW_INCLUDE = "include",
        KW_DEFINE = "define",
        KW_IFDEF = "ifdef",
        KW_ENDIF = "endif",
        KW_SPEC = "spec",
        KW_TYPE = "type",
        KW_RECORD = "record",
        KW_WHEN = "when",
        KW_OF = "of",
        KW_END = "end",
        KW_CASE = "case",
        KW_IF = "if",
        KW_RECEIVE = "receive",
        KW_AFTER = "after",
        KW_TRY = "try",
        KW_CATCH = "catch",
        KW_FUN = "fun",
        KW_BEGIN = "begin",
        KW_TRUE = "true",
        KW_FALSE = "false",
    }

    forms: _form*,

    _form: attribute | function_definition,

    attribute: "-" attribute_name [declaration_name] [attribute_group] [attribute_tail] ".",
    declaration_name: ATOM | QUOTED_ATOM,
    attribute_tail: attribute_type | spec_clause,
    spec_clause: [guard] "->" _expression,
    attribute_type: "::" _expression,
    attribute_group: "(" [attribute_arguments] ")",
    attribute_name: ATOM | NAME,
    attribute_arguments: _expression ("," _expression)*,

    function_definition: function_head (";" function_head)* ".",
    function_head: function_name parameter_group [guard] "->" body,
    parameter_group: "(" [parameters] ")",
    function_name: ATOM | QUOTED_ATOM,
    parameters: _expression ("," _expression)*,

    guard: "when" guard_sequence,
    guard_sequence: _expression (";" _expression)*,
    body: _expression ("," _expression)*,

    _expression: precedence _primary {
        right "=" => match_expression,
        left "!" => send_expression,
        left "or" | "and" => logical_expression,
        left "==" | "/=" | "=:=" | "=/=" | "<" | ">" | "=<" | ">=" => comparison,
        left "++" | "--" => list_operation,
        left "+" | "-" => arithmetic,
        left "bor" | "bxor" | "band" | "bsl" | "bsr" => bitwise,
        left "*" | "/" => arithmetic,
        left "div" | "rem" => arithmetic,
        prefix "not" | "-" | "+" => unary,
        postfix call_arguments => call,
        postfix module_call => remote_call,
        postfix "." NAME => record_access,
    },

    call_arguments: "(" [(argument %? ",")] ")",
    argument: _expression,
    module_call: ":" (ATOM | VARIABLE | QUOTED_ATOM) call_arguments,

    _primary:
        ATOM
        | VARIABLE
        | QUOTED_ATOM
        | NUMBER
        | STRING
        | "true"
        | "false"
        | NAME
        | paren_expression
        | list_literal
        | tuple_literal
        | case_expression
        | if_expression
        | fun_expression
        | begin_expression
        | receive_expression
        | map_literal
        | macro_reference,

    paren_expression: "(" _expression ")",
    list_literal: "[" [(list_item %? ",")] [list_tail] [list_comp_tail] "]",
    list_comp_tail: "||" qualifier ("," qualifier)*,
    list_item: _expression,
    list_tail: "|" _expression,
    tuple_literal: "{" [(_expression %? ",")] "}",
    qualifier: _expression ["<-" _expression],
    case_expression: "case" _expression "of" case_clauses "end",
    case_clauses: case_clause (";" case_clause)*,
    case_clause: _expression [guard] "->" body,
    if_expression: "if" case_clauses "end",
    fun_expression: "fun" [NAME] anonymous_clauses "end",
    anonymous_clauses: anonymous_clause (";" anonymous_clause)*,
    anonymous_clause: parameter_group [guard] "->" body,
    begin_expression: "begin" body "end",
    receive_expression: "receive" case_clauses "end",
    map_literal: "#" map_body,
    map_body: "{" [(map_field %? ",")] "}",
    map_field: _expression [map_assoc],
    map_assoc: ("=>" | ":=") _expression,
    macro_reference: "?" (ATOM | VARIABLE | NAME),
} }

pub fn lex_erlang(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'%' => { lex::line_comment(c); COMMENT }
        b'"' => if lex::quoted(c, b'"') { STRING } else { BAD_STRING },
        b'\'' => if lex::quoted(c, b'\'') { QUOTED_ATOM } else { BAD_STRING },
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'a'..=b'z' => {
            let start = c.rest();
            let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@'));
            // `-module`, `-export`, `-spec`: after `-` the word is a form name
            // whatever it spells elsewhere.
            if c.previous() == MINUS {
                return ATOM;
            }
            keyword(&start[..length])
        }
        b'A'..=b'Z' | b'_' => {
            c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@'));
            VARIABLE
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'.' => { c.bump(); DOT }
        b':' if c.peek_at(1) == b':' => { c.bump_n(2); DOUBLE_COLON }
        b':' if c.peek_at(1) == b'=' => { c.bump_n(2); UPDATE }
        b':' => { c.bump(); COLON }
        b'#' => { c.bump(); HASH }
        b'?' => { c.bump(); QUESTION }
        b'=' if c.peek_at(1) == b':' && c.peek_at(2) == b'=' => { c.bump_n(3); EXACTLY }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' if c.peek_at(1) == b'/' && c.peek_at(2) == b'=' => { c.bump_n(3); NOT_EXACTLY }
        b'=' if c.peek_at(1) == b'>' => { c.bump_n(2); ASSOC }
        b'=' if c.peek_at(1) == b'<' => { c.bump_n(2); LE }
        b'=' => { c.bump(); ASSIGN }
        b'/' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'/' => { c.bump(); SLASH }
        b'<' if c.peek_at(1) == b'-' => { c.bump_n(2); GENERATOR }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' => { c.bump(); GT }
        b'+' if c.peek_at(1) == b'+' => { c.bump_n(2); PLUS_PLUS }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'-' if c.peek_at(1) == b'-' => { c.bump_n(2); MINUS_MINUS }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); LIST_SEP }
        b'|' => { c.bump(); PIPE }
        b'!' => { c.bump(); SEND }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"module" => KW_MODULE,
        b"export" => KW_EXPORT,
        b"import" => KW_IMPORT,
        b"behaviour" => KW_BEHAVIOUR,
        b"include" => KW_INCLUDE,
        b"define" => KW_DEFINE,
        b"ifdef" => KW_IFDEF,
        b"endif" => KW_ENDIF,
        b"spec" => KW_SPEC,
        b"type" => KW_TYPE,
        b"record" => KW_RECORD,
        b"when" => KW_WHEN,
        b"of" => KW_OF,
        b"end" => KW_END,
        b"case" => KW_CASE,
        b"if" => KW_IF,
        b"receive" => KW_RECEIVE,
        b"after" => KW_AFTER,
        b"try" => KW_TRY,
        b"catch" => KW_CATCH,
        b"fun" => KW_FUN,
        b"begin" => KW_BEGIN,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        b"div" => DIV,
        b"rem" => REM,
        b"and" => AND,
        b"or" => OR,
        b"band" => BAND,
        b"bor" => BOR,
        b"bxor" => BXOR,
        b"bsl" => BSL,
        b"bsr" => BSR,
        b"not" => NOT,
        _ => ATOM,
    }
}
