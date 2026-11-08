//! Elixir: modules, `def`/`defp`/`defmacro`, the `do ... end` blocks, and
//! expressions over calls, atoms and the pipe operator.
//!
//! A statement ends where the next token cannot continue the expression, so the
//! newline is whitespace and a module body is a plain statement list. Blocks
//! (`defmodule`, `def`, `if`, `case`, `fn`) end with `end`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **No `x[i]` subscript.** Elixir has none (access is `Access.get/2` or a
//!   map's `.key`), and leaving `[` out of the postfix set is what lets one
//!   `case` clause end where the next clause's pattern begins.
//! - **Sigils** (`~r/.../`, `~s{...}`) are not modelled: a `~` is an operator.
//! - **`|>` is left-associative here** as Elixir has it; a pipe chain is a
//!   left-deep `pipe` tree rather than a list of stages.
//! - **The one-line `def f(x), do: expr` form is not modelled**; a definition's
//!   body is a `do ... end` block.
//! - **A `case`/`fn` clause body is one expression.** The clause head cannot be
//!   told from the next clause's head without a line terminator the DSL does
//!   not have, so `-> a` is one expression; a clause with several statements
//!   is not modelled.
//! - **Guards (`when`)** after a definition head and in `case` clauses are
//!   modelled as an optional expression, not a guard node.
//! - **`fn ... -> ... end`** takes `->`-clauses only, with no guard.
//! - **Structs (`%Name{}`)** are a map with a leading `%` atom, not a struct
//!   node.
//! - **A keyword list** (`[a: 1]`) is a call-free pair through the `pair` row.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { ELIXIR {
    name = "elixir";
    lex = lex_elixir;

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
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        IDENTICAL = "===",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        CONCAT = "<>",
        LIST_CONCAT = "++",
        LIST_SUB = "--",
        ARROW = "->",
        PIPE = "|>",
        PIPE_SEP = "|",
        AND = "&&",
        OR = "||",
        BANG = "!",
        QUESTION = "?",
        AT = "@",
        AMP = "&",
        PERCENT = "%",
        BACKSLASH = "\\\\",
        ATOM = "atom",
        NAME = "identifier",
        /// `name:` — the key of a keyword list or a map, colon included.
        KEYWORD_KEY = "keyword_key",
        VARIABLE = "variable",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_DEFMODULE = "defmodule",
        KW_DEF = "def",
        KW_DEFP = "defp",
        KW_DEFMACRO = "defmacro",
        KW_DEFMACROP = "defmacrop",
        KW_DO = "do",
        KW_END = "end",
        KW_IF = "if",
        KW_UNLESS = "unless",
        KW_ELSE = "else",
        KW_CASE = "case",
        KW_COND = "cond",
        KW_WHEN = "when",
        KW_FN = "fn",
        KW_IMPORT = "import",
        KW_USE = "use",
        KW_ALIAS = "alias",
        KW_REQUIRE = "require",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_NIL = "nil",
        KW_AND = "and",
        KW_OR = "or",
        KW_NOT = "not",
        KW_IN = "in",
    }

    program: _statement*,

    _statement:
        module_definition
        | definition
        | if_expression
        | unless_expression
        | case_expression
        | cond_expression
        | import_statement
        | expression_statement,

    body: _statement*,
    do_body: "do" body "end",

    attribute: "@" NAME [_expression],
    module_definition: "defmodule" module_name "do" body "end",
    module_name: _alias ("." _alias)*,
    _alias: NAME | VARIABLE,

    definition: definition_head call_head [guard_clause] (do_body | "," "do" ":" _expression),
    definition_head: "def" | "defp" | "defmacro" | "defmacrop",
    call_head: NAME [parameters],
    parameters: "(" [parameter ("," parameter)*] ")",
    parameter: _expression ["\\\\" _expression],
    guard_clause: "when" _expression,

    if_expression: "if" _expression "do" body [else_clause] "end",
    unless_expression: "unless" _expression "do" body [else_clause] "end",
    else_clause: "else" body,
    case_expression: "case" _expression "do" case_clause* "end",
    cond_expression: "cond" "do" case_clause* "end",
    case_clause: _expression ("," _expression)* ["when" _expression] "->" _expression,

    import_statement: ("import" | "use" | "alias" | "require") module_name [import_options],
    import_options: "," (NAME ":" _expression)*,

    expression_statement: _expression,

    _expression: precedence _primary {
        right "=" => match,
        left "||" | "or" => logical_expression,
        left "&&" | "and" => logical_expression,
        left "==" | "!=" | "===" | "<" | ">" | "<=" | ">=" => comparison,
        left "|>" => pipe_expression,
        left "<>" => concatenation,
        left "++" | "--" => list_operator,
        right "|" => cons,
        left "+" | "-" => arithmetic,
        left "*" | "/" => arithmetic,
        prefix "!" | "not" | "-" | "+" => unary,
        postfix call_arguments => call,
        postfix "." NAME => member,
    },

    call_arguments: "(" [(argument %? ",")] ")",
    argument: keyword_argument | _expression,
    keyword_argument: KEYWORD_KEY _expression,

    _primary:
        NAME
        | VARIABLE
        | ATOM
        | NUMBER
        | STRING
        | "true"
        | "false"
        | "nil"
        | paren_expression
        | list_literal
        | tuple_literal
        | map_literal
        | fn_literal
        | attribute,

    paren_expression: "(" [(argument %? ",")] ")",
    list_literal: "[" [(argument %? ",")] "]",
    tuple_literal: "{" [(argument %? ",")] "}",
    map_literal: "%" [map_name] map_body,
    map_name: VARIABLE | NAME | ATOM,
    map_body: "{" [(argument %? ",")] "}",
    fn_literal: "fn" fn_clause+ "end",
    fn_clause: _expression ("," _expression)* "->" _expression,

} }

pub fn lex_elixir(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'#' => { lex::line_comment(c); COMMENT }
        b'"' => {
            if c.peek_at(1) == b'"' && c.peek_at(2) == b'"' {
                c.bump_n(3);
                loop {
                    if c.at_end() { return BAD_STRING; }
                    if c.starts_with(b"\"\"\"") { c.bump_n(3); return STRING; }
                    c.bump_char();
                }
            }
            if lex::quoted(c, b'"') { STRING } else { BAD_STRING }
        }
        b'\'' => if lex::quoted(c, b'\'') { STRING } else { BAD_STRING },
        b':' => {
            c.bump();
            c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@' | b'?' | b'!'));
            ATOM
        }
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let mut length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'?' | b'!'));
            // `foo?` and `save!` are one name in Elixir.
            if matches!(c.peek(), b'?' | b'!') && !matches!(c.peek_at(1), b'=') {
                c.bump();
                length += 1;
            }
            let text = &start[..length];
            if text.first().is_some_and(u8::is_ascii_uppercase) {
                return VARIABLE;
            }
            // A reserved word stays a reserved word even before `:`; other
            // names are one key token, `name:`, which is what a keyword list
            // and a map hold.
            let kind = keyword(text);
            if kind != NAME {
                return kind;
            }
            if c.peek() == b':' && c.peek_at(1) != b':' {
                c.bump();
                return KEYWORD_KEY;
            }
            NAME
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'\\' if c.peek_at(1) == b'\\' => { c.bump_n(2); BACKSLASH }
        b'\\' => { c.bump(); BACKSLASH }
        b'.' => { c.bump(); DOT }
        b'=' if c.peek_at(1) == b'=' && c.peek_at(2) == b'=' => { c.bump_n(3); IDENTICAL }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' => { c.bump(); ASSIGN }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' => { c.bump(); BANG }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'>' => { c.bump_n(2); CONCAT }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' => { c.bump(); GT }
        b'+' if c.peek_at(1) == b'+' => { c.bump_n(2); LIST_CONCAT }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'-' if c.peek_at(1) == b'-' => { c.bump_n(2); LIST_SUB }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' => { c.bump(); SLASH }
        b'|' if c.peek_at(1) == b'>' => { c.bump_n(2); PIPE }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); OR }
        b'|' => { c.bump(); PIPE_SEP }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'&' => { c.bump(); AMP }
        b'%' => { c.bump(); PERCENT }
        b'?' => { c.bump(); QUESTION }
        b'@' => { c.bump(); AT }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"defmodule" => KW_DEFMODULE,
        b"def" => KW_DEF,
        b"defp" => KW_DEFP,
        b"defmacro" => KW_DEFMACRO,
        b"defmacrop" => KW_DEFMACROP,
        b"do" => KW_DO,
        b"end" => KW_END,
        b"if" => KW_IF,
        b"unless" => KW_UNLESS,
        b"else" => KW_ELSE,
        b"case" => KW_CASE,
        b"cond" => KW_COND,
        b"when" => KW_WHEN,
        b"fn" => KW_FN,
        b"import" => KW_IMPORT,
        b"use" => KW_USE,
        b"alias" => KW_ALIAS,
        b"require" => KW_REQUIRE,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        b"nil" => KW_NIL,
        b"and" => KW_AND,
        b"or" => KW_OR,
        b"not" => KW_NOT,
        b"in" => KW_IN,
        _ => NAME,
    }
}
