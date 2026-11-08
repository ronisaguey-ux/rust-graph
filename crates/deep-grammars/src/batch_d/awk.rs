//! AWK: rules (`pattern { action }`), `BEGIN`/`END`, functions, and the
//! statements inside an action.
//!
//! AWK has no statement terminator at a line end; the lexer treats a newline as
//! whitespace and an expression stops where the next token cannot continue it,
//! exactly as an action's statements do. `;` is accepted between statements.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Regular-expression literals** are not modelled: `/re/` is division in
//!   this grammar. `$0 ~ "re"` works; `$0 ~ /re/` does not.
//! - **`getline` forms** are calls to the name `getline` in a statement, not the
//!   redirection special form.
//! - **A rule with a range pattern** (`NR == 1, NR == 10`) parses its two
//!   expressions only because `,` separates them in a `print` argument list;
//!   the range itself is not a node.
//! - **Implicit `$0`** in a pattern with no expression is a block statement:
//!   `{ print }` alone is modelled, `{ print }` after a pattern is a
//!   `pattern_action`.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { AWK {
    name = "awk";
    lex = lex_awk;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        COMMA = ",",
        SEMI = ";",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
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
        BANG = "!",
        AND = "&&",
        OR = "||",
        MATCH = "~",
        NOT_MATCH = "!~",
        INCR = "++",
        DECR = "--",
        PLUS_ASSIGN = "+=",
        MINUS_ASSIGN = "-=",
        STAR_ASSIGN = "*=",
        SLASH_ASSIGN = "/=",
        PIPE = "|",
        QUESTION = "?",
        COLON = ":",
        DOLLAR = "$",
        APPEND = ">>",
        NAME = "identifier",
        NUMBER = "number",
        STRING = "string",
        REGEX = "regex",
        BAD_STRING = "bad_string" bad,

        KW_BEGIN = "BEGIN",
        KW_END = "END",
        KW_FUNCTION = "function",
        KW_IF = "if",
        KW_ELSE = "else",
        KW_WHILE = "while",
        KW_FOR = "for",
        KW_DO = "do",
        KW_BREAK = "break",
        KW_CONTINUE = "continue",
        KW_NEXT = "next",
        KW_NEXTFILE = "nextfile",
        KW_EXIT = "exit",
        KW_RETURN = "return",
        KW_DELETE = "delete",
        KW_IN = "in",
        KW_PRINT = "print",
        KW_PRINTF = "printf",
        KW_GETLINE = "getline",
    }

    program: _item*,

    _item: function_definition | begin_block | end_block | pattern_rule | bare_block,

    function_definition: "function" NAME parameters block,
    parameters: "(" [parameter ("," parameter)*] ")",
    parameter: NAME,
    begin_block: "BEGIN" block,
    end_block: "END" block,

    pattern_rule: _expression ["," _expression] [block],
    bare_block: block,
    block: "{" _statement* "}",

    _statement: _statement_kind ";"*,
    _statement_kind:
        block
        | if_statement
        | while_statement
        | do_statement
        | for_statement
        | print_statement
        | return_statement
        | next_statement
        | exit_statement
        | break_statement
        | continue_statement
        | delete_statement
        | expression_statement,

    if_statement: "if" paren_expression _statement [else_clause],
    else_clause: "else" _statement,
    while_statement: "while" paren_expression _statement,
    do_statement: "do" _statement "while" paren_expression,
    for_statement: "for" for_header _statement,
    for_header: "(" classic_for_body ")",
    classic_for_body: [expression_statement] [";" [_expression] ";" [_expression]],

    print_statement: ("print" | "printf") [_print_arguments] [redirection],
    _print_arguments: _expression ("," _expression)*,
    redirection: [">" | ">>" | "|"] _expression,

    return_statement: "return" [_expression],
    next_statement: "next",
    exit_statement: "exit" [_expression],
    break_statement: "break",
    continue_statement: "continue",
    delete_statement: "delete" (NAME | array_reference),

    expression_statement: _expression,
    paren_expression: "(" _expression ")",

    _expression: precedence _primary {
        right "=" | "+=" | "-=" | "*=" | "/=" => assignment,
        left "?" _expression ":" => ternary,
        left "||" => logical_expression,
        left "&&" => logical_expression,
        left "in" => membership,
        left "~" | "!~" => match_expression,
        left "==" | "!=" | "<" | ">" | "<=" | ">=" => comparison,
        left "+" | "-" => arithmetic,
        left "*" | "/" | "%" => arithmetic,
        right "^" => arithmetic,
        prefix "!" | "-" | "+" => unary,
        postfix "++" | "--" => increment,
        prefix "++" | "--" => increment,
        postfix call_arguments => call,
        postfix array_reference => index,
        prefix "$" => field,
    },

    call_arguments: "(" [(argument %? ",")] ")",
    argument: _expression,
    array_reference: "[" [_expression] "]",

    _primary:
        NAME
        | NUMBER
        | STRING
        | REGEX
        | paren_expression,
} }

pub fn lex_awk(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'\\' if c.peek_at(1) == b'\n' => { c.bump_n(2); WHITESPACE }
        b'#' => { lex::line_comment(c); COMMENT }
        b'"' => if lex::quoted(c, b'"') { STRING } else { BAD_STRING },
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
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' => { c.bump(); ASSIGN }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' if c.peek_at(1) == b'~' => { c.bump_n(2); NOT_MATCH }
        b'!' => { c.bump(); BANG }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' if c.peek_at(1) == b'>' => { c.bump_n(2); APPEND }
        b'>' => { c.bump(); GT }
        b'~' => { c.bump(); MATCH }
        b'+' if c.peek_at(1) == b'+' => { c.bump_n(2); INCR }
        b'+' if c.peek_at(1) == b'=' => { c.bump_n(2); PLUS_ASSIGN }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'-' => { c.bump_n(2); DECR }
        b'-' if c.peek_at(1) == b'=' => { c.bump_n(2); MINUS_ASSIGN }
        b'-' => { c.bump(); MINUS }
        b'*' if c.peek_at(1) == b'=' => { c.bump_n(2); STAR_ASSIGN }
        b'*' => { c.bump(); STAR }
        b'/' if c.peek_at(1) == b'=' => { c.bump_n(2); SLASH_ASSIGN }
        b'/' if regex_position(c.previous()) => {
            c.bump();
            loop {
                if c.at_end() {
                    return BAD_STRING;
                }
                match c.peek() {
                    b'\\' => c.bump_n(2),
                    b'/' => { c.bump(); return REGEX; }
                    b'\n' => return BAD_STRING,
                    _ => c.bump_char(),
                }
            }
        }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'^' => { c.bump(); CARET }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); OR }
        b'|' => { c.bump(); PIPE }
        b'?' => { c.bump(); QUESTION }
        b':' => { c.bump(); COLON }
        b'$' => { c.bump(); DOLLAR }
        _ => UNKNOWN,
    }
}

/// `/` starts a regular expression where an operand may start.
fn regex_position(previous: Kind) -> bool {
    matches!(
        previous,
        crate::deep::syntax::UNKNOWN
            | LPAREN | COMMA | ASSIGN | MATCH | NOT_MATCH | BANG | AND | OR | QUESTION | COLON | LBRACE | RBRACE | SEMI
            | KW_IF | KW_WHILE | KW_PRINT | KW_PRINTF | KW_RETURN
    )
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"BEGIN" => KW_BEGIN,
        b"END" => KW_END,
        b"function" | b"func" => KW_FUNCTION,
        b"if" => KW_IF,
        b"else" => KW_ELSE,
        b"while" => KW_WHILE,
        b"for" => KW_FOR,
        b"do" => KW_DO,
        b"break" => KW_BREAK,
        b"continue" => KW_CONTINUE,
        b"next" => KW_NEXT,
        b"nextfile" => KW_NEXTFILE,
        b"exit" => KW_EXIT,
        b"return" => KW_RETURN,
        b"delete" => KW_DELETE,
        b"in" => KW_IN,
        b"print" => KW_PRINT,
        b"printf" => KW_PRINTF,
        b"getline" => KW_GETLINE,
        _ => NAME,
    }
}
