//! Perl: packages, subroutines, the control-flow statements, and expressions
//! over sigiled variables.
//!
//! A statement ends with `;`; a block (`sub`, `if`, `while`, `foreach`, `do`)
//! does not, and its braces are the rule's own group.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Regex literals.** `=~ /pattern/` is not modelled: `/` is division here.
//!   `qr{...}` and `=~ $regex` work. The lexer's one context hook is not enough
//!   for Perl's `/` ambiguity.
//! - **Contexts (`scalar`, `list`)** and prototypes beyond `()` are not nodes;
//!   a prototype is accepted and dropped.
//! - **Heredocs** are not modelled: `<<EOF` is two `<` operators.
//! - **A sigil is part of a name.** `$x`, `@x` and `%x` are three different
//!   `variable` tokens, so a symbol table can tell them apart.
//! - **Postfix `if/unless/while`** are not modelled: `print $x if $y;` does not
//!   parse.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { PERL {
    name = "perl";
    lex = lex_perl;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        COMMA = ",",
        SEMI = ";",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        STRCMP = "eq",
        NUMCMP = "cmp",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        DOT = ".",
        ARROW = "->",
        NAMESPACE = "::",
        FAT_COMMA = "=>",
        AND = "&&",
        OR = "||",
        NOT = "!",
        BIT_AND = "&",
        BIT_OR = "|",
        BIT_XOR = "^",
        TILDE = "~",
        BACKSLASH = "\\",
        SHL = "<<",
        SHR = ">>",
        VARIABLE = "variable",
        NAME = "identifier",
        NUMBER = "number",
        STRING = "string",
        /// `-d`, `-s`, `-e`: a file test operator.
        FILE_TEST = "file_test",
        BAD_STRING = "bad_string" bad,

        KW_SUB = "sub",
        KW_MY = "my",
        KW_OUR = "our",
        KW_LOCAL = "local",
        KW_IF = "if",
        KW_ELSIF = "elsif",
        KW_ELSE = "else",
        KW_UNLESS = "unless",
        KW_WHILE = "while",
        KW_FOR = "for",
        KW_FOREACH = "foreach",
        KW_RETURN = "return",
        KW_USE = "use",
        KW_REQUIRE = "require",
        KW_PACKAGE = "package",
        KW_LAST = "last",
        KW_NEXT = "next",
        KW_DO = "do",
        KW_UNDEF = "undef",
        KW_PRINT = "print",
        KW_SAY = "say",
        KW_OR = "or",
        KW_AND = "and",
        KW_PUSH = "push",
        KW_POP = "pop",
        KW_SHIFT = "shift",
        KW_UNSHIFT = "unshift",
        KW_DIE = "die",
        KW_WARN = "warn",
        KW_DEFINED = "defined",
        KW_SCALAR = "scalar",
    }

    program: _statement*,

    _statement:
        my_declaration
        | package_statement
        | use_statement
        | sub_definition
        | if_statement
        | unless_statement
        | while_statement
        | foreach_statement
        | for_statement
        | return_statement
        | last_statement
        | next_statement
        | do_block
        | print_statement
        | list_statement
        | expression_statement,

    block: "{" _statement* "}",

    my_declaration: ("my" | "our" | "local") variable_declarator ["=" _expression] [";"],
    variable_declarator: paren_variables | VARIABLE ("," VARIABLE)*,
    paren_variables: "(" [VARIABLE ("," VARIABLE)*] ")",

    package_statement: "package" module_name ";",
    use_statement: ("use" | "require") module_name [";"],
    module_name: NAME ("::" NAME)*,
    sub_definition: "sub" NAME [prototype_list] block,
    prototype_list: "(" [prototype] ")",
    prototype: VARIABLE,

    if_statement: "if" paren_expression block elsif_clause* [else_clause],
    elsif_clause: "elsif" paren_expression block,
    else_clause: "else" block,
    unless_statement: "unless" paren_expression block,
    while_statement: "while" paren_expression block,
    foreach_statement: "foreach" ["my" | "our" | "local"] VARIABLE paren_expression block,
    for_statement: "for" classic_for block,
    classic_for: "(" [expression_statement] ";" [_expression] ";" [_expression] ")",

    return_statement: "return" [_expression] ";",
    last_statement: "last" [";"],
    next_statement: "next" [";"],
    do_block: "do" block,
    print_statement: ("print" | "say") [_expression ("," _expression)*] [";"],
    list_statement: ("push" | "pop" | "shift" | "unshift" | "die" | "warn") [_expression ("," _expression)*] [";"],

    expression_statement: _expression [";"],
    paren_expression: "(" [(_expression %? ",")] ")",

    _expression: precedence _primary {
        right "=" => assignment,
        left "||" | "or" => logical_expression,
        left "&&" | "and" => logical_expression,
        left "==" | "!=" | "<" | ">" | "<=" | ">=" => comparison,
        left "eq" | "cmp" => string_comparison,
        left "." => concat,
        left "=>" => pair,
        left "+" | "-" => arithmetic,
        left "*" | "/" | "%" => arithmetic,
        prefix "!" | "-" | "+" | "~" => unary,
        prefix "defined" | "scalar" => builtin_prefix,
        prefix "file_test" => stat_test,
        left "<<" | ">>" => shift,
        left "&" | "|" | "^" => bitwise,
        postfix call_arguments => call,
        postfix index_arguments => index,
        postfix hash_index => hash_access,
        postfix arrow_member => member,
    },

    call_arguments: "(" [(argument %? ",")] ")",
    argument: _expression,
    index_arguments: "[" [_expression] "]",
    arrow_member: "->" (NAME | array_index | hash_index),
    array_index: "[" [_expression] "]",
    hash_index: "{" [_expression] "}",

    _primary:
        VARIABLE
        | reference
        | NAME
        | NUMBER
        | STRING
        | paren_expression
        | array_literal
        | hash_literal
        | "undef",

    reference: "\\" _primary,
    array_literal: "[" [(argument %? ",")] "]",
    hash_literal: "{" [(argument %? ",")] "}",
} }

pub fn lex_perl(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'#' => { lex::line_comment(c); COMMENT }
        b'\'' => if lex::quoted(c, b'\'') { STRING } else { BAD_STRING },
        b'"' => if lex::quoted(c, b'"') { STRING } else { BAD_STRING },
        b'$' | b'@' | b'%' => {
            c.bump();
            if c.peek() == b'#' {
                c.bump();
            }
            if c.peek() == b'{' {
                c.bump();
                c.eat_while(|byte| byte != b'}' && byte != b'\n');
                c.eat(b'}');
            } else {
                c.eat_while(lex::is_word_byte);
            }
            VARIABLE
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
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'.' => { c.bump(); DOT }
        b':' if c.peek_at(1) == b':' => { c.bump_n(2); NAMESPACE }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' if c.peek_at(1) == b'>' => { c.bump_n(2); FAT_COMMA }
        b'=' => { c.bump(); ASSIGN }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' => { c.bump(); NOT }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'<' => { c.bump_n(2); SHL }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' if c.peek_at(1) == b'>' => { c.bump_n(2); SHR }
        b'>' => { c.bump(); GT }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'-' if c.peek_at(1).is_ascii_alphabetic() && !operand_before(c.previous()) => { c.bump_n(2); FILE_TEST }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'&' => { c.bump(); BIT_AND }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); OR }
        b'|' => { c.bump(); BIT_OR }
        b'^' => { c.bump(); BIT_XOR }
        b'~' => { c.bump(); TILDE }
        b'\\' => { c.bump(); BACKSLASH }
        _ => UNKNOWN,
    }
}

/// Whether the token before could end an operand, so a `-d` here is subtraction.
fn operand_before(previous: Kind) -> bool {
    matches!(
        previous,
        VARIABLE | NAME | NUMBER | STRING | RPAREN | RBRACKET | RBRACE | KW_UNDEF
    )
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"sub" => KW_SUB,
        b"my" => KW_MY,
        b"our" => KW_OUR,
        b"local" => KW_LOCAL,
        b"if" => KW_IF,
        b"elsif" => KW_ELSIF,
        b"else" => KW_ELSE,
        b"unless" => KW_UNLESS,
        b"while" => KW_WHILE,
        b"for" => KW_FOR,
        b"foreach" => KW_FOREACH,
        b"return" => KW_RETURN,
        b"use" => KW_USE,
        b"require" => KW_REQUIRE,
        b"package" => KW_PACKAGE,
        b"last" => KW_LAST,
        b"next" => KW_NEXT,
        b"do" => KW_DO,
        b"undef" => KW_UNDEF,
        b"print" => KW_PRINT,
        b"say" => KW_SAY,
        b"or" => KW_OR,
        b"and" => KW_AND,
        b"push" => KW_PUSH,
        b"pop" => KW_POP,
        b"shift" => KW_SHIFT,
        b"unshift" => KW_UNSHIFT,
        b"die" => KW_DIE,
        b"warn" => KW_WARN,
        b"defined" => KW_DEFINED,
        b"scalar" => KW_SCALAR,
        b"eq" => STRCMP,
        b"cmp" => NUMCMP,
        _ => NAME,
    }
}
