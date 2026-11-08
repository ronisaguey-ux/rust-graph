//! R: function definitions, control flow, calls, indexing and the assignment
//! operators.
//!
//! A newline is whitespace: R ends a statement where the next token cannot
//! continue the expression, and that is exactly how a statement list ends here.
//! `;` is the explicit separator and is accepted after any statement.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A definition is an `assignment` whose right side is a `function_literal`.**
//!   R has no named function form, so `f <- function(x) x` has no node of its
//!   own; the name, the parameters and the body are all still there.
//! - **A named argument is an assignment.** `f(x = 1)` parses `x = 1` through
//!   the assignment row, not as a separate argument node.
//! - **`if`/`else` is an expression**, as R has it, so a conditional statement
//!   is an `expression_statement` holding an `if_expression`; there is no
//!   separate `if_statement` kind.
//! - **Formulas** (`y ~ x + z`) are an operator here, not a formula node.
//! - **S4/S3 dispatch and `R6` classes** are ordinary calls: `setClass` and
//!   `R6Class` are not modelled as class declarations.
//! - **A missing separator between two statements is accepted** when the first
//!   expression cannot continue: `x <- 1 y <- 2` on one line is two statements.
//! - **`%op%` custom infix operators** are lexed as one `%...%` token and are
//!   not accepted where an operator is expected.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { R {
    name = "r";
    lex = lex_r;

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
        ASSIGN = "<-",
        ASSIGN_ALT = "=",
        SUPERASSIGN = "<<-",
        TILDE = "~",
        OR = "||",
        AND = "&&",
        VECTOR_OR = "|",
        VECTOR_AND = "&",
        EQ = "==",
        NE = "!=",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        CARET = "^",
        COLON = ":",
        DOLLAR = "$",
        AT = "@",
        NAMESPACE = "::",
        NAMESPACE_INNER = ":::",
        BANG = "!",
        QUESTION = "?",
        BACKSLASH = "\\",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_FUNCTION = "function",
        KW_IF = "if",
        KW_ELSE = "else",
        KW_FOR = "for",
        KW_IN = "in",
        KW_WHILE = "while",
        KW_REPEAT = "repeat",
        KW_BREAK = "break",
        KW_NEXT = "next",
        KW_RETURN = "return",
        KW_TRUE = "TRUE",
        KW_FALSE = "FALSE",
        KW_NULL = "NULL",
        KW_NA = "NA",
        KW_INF = "Inf",
        KW_NAN = "NaN",
    }

    program: _statement*,

    _statement: (_statement_kind) ";"*,
    _statement_kind:
        for_statement
        | while_statement
        | repeat_statement
        | return_statement
        | break_statement
        | next_statement
        | expression_statement,

    block: "{" _statement* "}",
    _terminator: ";",

    expression_statement: _expression _terminator*,
    return_statement: "return" [_expression] _terminator*,
    break_statement: "break" _terminator*,
    next_statement: "next" _terminator*,

    function_literal: "function" parameters _body,
    parameters: "(" [parameter ("," parameter)*] ")",
    parameter: IDENT ["=" _expression],
    _body: block | (_statement_kind ";"*),

    if_expression: "if" paren_expression _body [else_clause],
    else_clause: "else" _body,
    for_statement: "for" for_header _body,
    for_header: "(" IDENT "in" _expression ")",
    while_statement: "while" paren_expression _body,
    repeat_statement: "repeat" _body,

    paren_expression: "(" _expression ")",

    _expression: precedence _primary {
        right "<-" | "=" | "<<-" => assignment,
        left "~" => formula,
        left "||" | "&&" => logical_operator,
        left "|" | "&" => logical_operator,
        left "==" | "!=" | "<" | ">" | "<=" | ">=" => comparison,
        left "+" | "-" => arithmetic,
        left "*" | "/" => arithmetic,
        left "%" => arithmetic,
        prefix "-" | "+" | "!" => unary,
        right "^" => arithmetic,
        left ":" => sequence,
        postfix call_arguments => call,
        postfix index_arguments => index,
        postfix "$" IDENT => member,
        postfix "." IDENT => member,
        postfix "@" IDENT => member,
        postfix "::" IDENT => qualified_name,
    },

    call_arguments: "(" [argument ("," argument)*] ")",
    argument: _expression,
    index_arguments: "[" [argument ("," argument)*] "]"
    ,
    /// `x[[i]]`: the inner bracket is a value, so the outer index sees one.
    bracket_expression: "[" [argument ("," argument)*] "]",

    _primary:
        IDENT
        | NUMBER
        | STRING
        | "TRUE"
        | "FALSE"
        | "NULL"
        | "NA"
        | "Inf"
        | "NaN"
        | paren_expression
        | function_literal
        | if_expression
        | bracket_expression,
} }

pub fn lex_r(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'#' => { lex::line_comment(c); COMMENT }
        b'\'' | b'"' => if lex::quoted(c, byte) { STRING } else { BAD_STRING },
        b'`' => {
            c.bump();
            c.eat_while(|byte| byte != b'`' && byte != b'\n');
            c.eat(b'`');
            IDENT
        }
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'.' if c.peek_at(1).is_ascii_digit() => { c.bump(); lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
            keyword(&start[..length])
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b'.' => { c.bump(); DOT }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'<' if c.peek_at(1) == b'<' && c.peek_at(2) == b'-' => { c.bump_n(3); SUPERASSIGN }
        b'<' if c.peek_at(1) == b'-' => { c.bump_n(2); ASSIGN }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' => { c.bump(); LT }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' => { c.bump(); ASSIGN_ALT }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' => { c.bump(); BANG }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' => { c.bump(); GT }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); OR }
        b'|' => { c.bump(); VECTOR_OR }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'&' => { c.bump(); VECTOR_AND }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ASSIGN }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' => { c.bump(); SLASH }
        b'%' => {
            c.bump();
            if c.peek() != b'%' {
                c.eat_while(|byte| byte != b'%' && byte != b'\n');
                c.eat(b'%');
            } else {
                c.bump();
            }
            PERCENT
        }
        b'^' => { c.bump(); CARET }
        b':' if c.peek_at(1) == b':' && c.peek_at(2) == b':' => { c.bump_n(3); NAMESPACE_INNER }
        b':' if c.peek_at(1) == b':' => { c.bump_n(2); NAMESPACE }
        b':' => { c.bump(); COLON }
        b'$' => { c.bump(); DOLLAR }
        b'@' => { c.bump(); AT }
        b'~' => { c.bump(); TILDE }
        b'?' => { c.bump(); QUESTION }
        b'\\' => { c.bump(); BACKSLASH }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"function" => KW_FUNCTION,
        b"if" => KW_IF,
        b"else" => KW_ELSE,
        b"for" => KW_FOR,
        b"in" => KW_IN,
        b"while" => KW_WHILE,
        b"repeat" => KW_REPEAT,
        b"break" => KW_BREAK,
        b"next" => KW_NEXT,
        b"return" => KW_RETURN,
        b"TRUE" | b"T" => KW_TRUE,
        b"FALSE" | b"F" => KW_FALSE,
        b"NULL" => KW_NULL,
        b"NA" => KW_NA,
        b"Inf" | b"inf" => KW_INF,
        b"NaN" => KW_NAN,
        _ => IDENT,
    }
}
