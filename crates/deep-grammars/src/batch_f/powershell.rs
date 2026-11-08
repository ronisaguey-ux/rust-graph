//! PowerShell: a script of pipelines, functions, classes and control flow.
//!
//! PowerShell is line-oriented: a statement ends at a newline or `;`. The lexer
//! emits `NEWLINE` and the grammar consumes it as `_statement_end`. `#` starts a
//! line comment and `<# ... #>` a block comment; a backtick continues a line.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A command and its arguments are two shapes.** `Get-Item -Path $p` is a
//!   `command_invocation` (a bare name, then parameters and arguments), while
//!   `$p.Length` is an expression. The two are told apart by a three-token
//!   predicate on what follows the name, so an invocation whose first argument
//!   is a script block (`Where-Object { ... }`) is not modelled: a predicate may
//!   not name a brace delimiter. `Where-Object -FilterScript { ... }` works.
//! - **A string is one token.** `"$name"` and `$(...)` inside a string are not
//!   expressions; here-strings (`@" ... "@`) are one token too.
//! - **`-eq`, `-and` and friends are distinct tokens**, matched by the lexer, so
//!   an unknown dash-word is a parameter, not an operator.
//! - **`$x = 1` is an assignment expression**, because the same `=` is the
//!   operator in `$x = 1` and inside a `for` header.
//! - **Splatting (`@args`)** and `@{}`/`@()` literals are not modelled.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { POWERSHELL {
    name = "powershell";
    lex = lex_powershell;

    tokens {
        NEWLINE = "newline",
        SEMI = ";",
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        COMMA = ",",
        DOT = ".",
        DOUBLE_COLON = "::",
        PIPE = "|",
        ASSIGN = "=",
        PLUS_ASSIGN = "+=",
        MINUS_ASSIGN = "-=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        BANG = "!",
        INCR = "++",
        DECR = "--",
        EQ = "-eq",
        NE = "-ne",
        LT = "-lt",
        GT = "-gt",
        LE = "-le",
        GE = "-ge",
        LIKE = "-like",
        NOTLIKE = "-notlike",
        MATCH_OP = "-match",
        NOTMATCH = "-notmatch",
        CONTAINS = "-contains",
        IN = "-in",
        AND = "-and",
        OR = "-or",
        NOT = "-not",
        REPLACE = "-replace",
        IDENT = "identifier",
        PARAM = "parameter",
        VARIABLE = "variable",
        NUMBER = "number",
        STRING = "string_literal",
        BAD_STRING = "bad_string" bad,

        KW_FUNCTION = "function",
        KW_FILTER = "filter",
        KW_CLASS = "class",
        KW_ENUM = "enum",
        KW_EXTENDS = "extends",
        KW_USING = "using",
        KW_PARAM = "param",
        KW_RETURN = "return",
        KW_THROW = "throw",
        KW_BREAK = "break",
        KW_CONTINUE = "continue",
        KW_IF = "if",
        KW_ELSEIF = "elseif",
        KW_ELSE = "else",
        KW_FOREACH = "foreach",
        KW_FOR = "for",
        KW_WHILE = "while",
        KW_DO = "do",
        KW_UNTIL = "until",
        KW_IN = "in",
        KW_SWITCH = "switch",
        KW_DEFAULT = "default",
        KW_TRY = "try",
        KW_CATCH = "catch",
        KW_FINALLY = "finally",
        KW_TRAP = "trap",
        KW_BEGIN = "begin",
        KW_PROCESS = "process",
        KW_END = "end",
        KW_TRUE = "$true",
        KW_FALSE = "$false",
        KW_NULL = "$null",
        KW_NAMESPACE = "namespace",
        KW_MODULE = "module",
        KW_ASSEMBLY = "assembly",
        KW_STRING_TYPE = "string",
        KW_INT_TYPE = "int",
        KW_BOOL_TYPE = "bool",
        KW_OBJECT_TYPE = "object",
    }

    script: (_statement_end | _line_item)*,

    _statement_end: NEWLINE | ";",
    /// A statement and its terminator, which a closing brace may stand in for
    /// when the block is written on one line: `{ Write-Host "x" }`.
    _line_item: _statement_kind [_statement_end],

    _statement_kind:
        function_definition
        | class_definition
        | enum_definition
        | using_statement
        | param_declaration
        | if_statement
        | foreach_statement
        | for_statement
        | while_statement
        | do_statement
        | switch_statement
        | try_statement
        | return_statement
        | throw_statement
        | break_statement
        | continue_statement
        | pipeline,

    function_definition: ("function" | "filter") _name [parameter_list] block,
    class_definition: "class" _name ["extends" _type] class_body,
    class_body: "{" (_statement_end | class_member | _line_item)* "}",
    class_member: type_annotation class_member_tail,
    class_member_tail: _name parameter_list block | VARIABLE ["=" _expression],
    enum_definition: "enum" _name enum_body,
    enum_body: "{" (_statement_end | _line_item)* "}",
    using_statement: "using" ("namespace" | "module" | "assembly") _name,

    /// PowerShell allows line breaks inside a parameter list.
    parameter_list: "(" _nl [parameter ("," _nl parameter)*] _nl ")",
    _nl: NEWLINE*,
    parameter: [type_annotation] VARIABLE ["=" _expression],
    param_declaration: "param" parameter_list,

    type_annotation: "[" _type "]",
    _type: _name | "string" | "int" | "bool" | "object",

    block: "{" (_statement_end | _line_item)* "}",

    if_statement: "if" paren_expression block elseif_clause* [else_clause],
    elseif_clause: "elseif" paren_expression block,
    else_clause: "else" block,
    foreach_statement: "foreach" foreach_header block,
    foreach_header: "(" VARIABLE "in" _expression ")",
    for_statement: "for" for_header block,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression] ")",
    _for_init: pipeline,
    while_statement: "while" paren_expression block,
    do_statement: "do" block ("while" | "until") paren_expression,
    switch_statement: "switch" paren_expression switch_body,
    switch_body: "{" (_statement_end | switch_clause)* "}",
    switch_clause: switch_label block,
    switch_label: _expression | "default",
    try_statement: "try" block catch_clause* [finally_clause],
    catch_clause: "catch" [catch_type] block,
    catch_type: "[" _type "]",
    finally_clause: "finally" block,
    return_statement: "return" [_expression],
    throw_statement: "throw" [_expression],
    break_statement: "break",
    continue_statement: "continue",

    paren_expression: "(" _expression ")",

    pipeline: _expression ("|" _expression)*,
    /// A bare command name with at least one argument is an expression here:
    /// `$x = Get-Item -Path "."` is an assignment whose right side is this node.
    /// The predicate that picks it lives in `_primary`, because a script-block
    /// argument starts with a brace a predicate may not name.
    command_expression: IDENT _command_argument+,
    _command_argument: parameter_argument | _expression,
    parameter_argument: PARAM [[PARAM] _expression],

    _expression: precedence _primary {
        right "=" | "+=" | "-=" => assignment,
        left "-or" => logical_or,
        left "-and" => logical_and,
        left "-eq" | "-ne" | "-lt" | "-gt" | "-le" | "-ge" | "-like" | "-notlike" | "-match" | "-notmatch" | "-contains" | "-in" | "-replace" => comparison,
        left "+" | "-" => additive,
        left "*" | "/" | "%" => multiplicative,
        prefix "!" | "-not" | "-" | "+" => unary,
        postfix "++" | "--" => update,
        prefix "++" | "--" => update,
        postfix argument_list => call,
        postfix "." _name => member_access,
        postfix index_expression => index,
        postfix static_access => static_member,
    },

    argument_list: "(" _nl [_argument ("," _nl _argument)*] _nl ")",
    _argument: _expression,
    index_expression: "[" _expression "]",
    static_access: "::" _name,

    _primary:
        &(IDENT PARAM) command_expression
        | &(IDENT STRING) command_expression
        | &(IDENT NUMBER) command_expression
        | &(IDENT VARIABLE) command_expression
        | &(IDENT IDENT) command_expression
        | _name
        | VARIABLE
        | NUMBER
        | STRING
        | parenthesized_expression
        | script_block,

    _name: IDENT,
    parenthesized_expression: "(" _expression ")",
    script_block: "{" (_statement_end | _line_item)* "}",
} }

pub fn lex_powershell(c: &mut Cursor) -> Kind {
    use crate::batch_d::lex;
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0c'));
            WHITESPACE
        }
        b'\r' => {
            c.bump();
            c.eat(b'\n');
            NEWLINE
        }
        b'\n' => { c.bump(); NEWLINE }
        b'`' if matches!(c.peek_at(1), b'\n' | b'\r') => {
            c.bump();
            c.eat(b'\r');
            c.eat(b'\n');
            WHITESPACE
        }
        b'#' => {
            lex::line_comment(c);
            COMMENT
        }
        b'<' if c.peek_at(1) == b'#' => {
            lex::block_comment(c, b"<#", b"#>", false);
            COMMENT
        }
        b'\'' => single_quoted(c),
        b'"' => double_quoted(c),
        b'@' if matches!(c.peek_at(1), b'"' | b'\'') && matches!(c.peek_at(2), b'\n' | b'\r') => {
            here_string(c)
        }
        b'$' => variable(c),
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => word(c),
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'.' => { c.bump(); DOT }
        b'|' => { c.bump(); PIPE }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'+' if c.peek_at(1) == b'=' => { c.bump_n(2); PLUS_ASSIGN }
        b'+' if c.peek_at(1) == b'+' => { c.bump_n(2); INCR }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'=' => { c.bump_n(2); MINUS_ASSIGN }
        b'-' if c.peek_at(1) == b'-' => { c.bump_n(2); DECR }
        b'-' => dash_word(c),
        b'=' => { c.bump(); ASSIGN }
        b'*' => { c.bump(); STAR }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'!' => { c.bump(); BANG }
        b':' if c.peek_at(1) == b':' => { c.bump_n(2); DOUBLE_COLON }
        _ => UNKNOWN,
    }
}

/// `-eq`, `-and`, or a parameter `-Path`. The operator table is checked first,
/// so an operator name is never a parameter.
fn dash_word(c: &mut Cursor) -> Kind {
    use crate::batch_d::lex;
    let start = c.rest();
    c.bump();
    let length = 1 + c.eat_while(|byte| byte.is_ascii_alphanumeric());
    let text = &start[..length];
    match text {
        b"-eq" => EQ,
        b"-ne" => NE,
        b"-lt" => LT,
        b"-gt" => GT,
        b"-le" => LE,
        b"-ge" => GE,
        b"-like" => LIKE,
        b"-notlike" => NOTLIKE,
        b"-match" => MATCH_OP,
        b"-notmatch" => NOTMATCH,
        b"-contains" => CONTAINS,
        b"-in" => IN,
        b"-and" => AND,
        b"-or" => OR,
        b"-not" => NOT,
        b"-replace" => REPLACE,
        _ => {
            let _ = lex::is_word_byte;
            PARAM
        }
    }
}

fn word(c: &mut Cursor) -> Kind {
    let begin = c.pos();
    let rest = c.rest();
    c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    // A cmdlet name keeps its dash: `Get-Item`, `Write-Output`.
    while c.peek() == b'-' && c.peek_at(1).is_ascii_alphabetic() {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    }
    let length = c.pos() - begin;
    keyword(&rest[..length])
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"function" => KW_FUNCTION,
        b"filter" => KW_FILTER,
        b"class" => KW_CLASS,
        b"enum" => KW_ENUM,
        b"extends" => KW_EXTENDS,
        b"using" => KW_USING,
        b"param" => KW_PARAM,
        b"return" => KW_RETURN,
        b"throw" => KW_THROW,
        b"break" => KW_BREAK,
        b"continue" => KW_CONTINUE,
        b"if" => KW_IF,
        b"elseif" => KW_ELSEIF,
        b"else" => KW_ELSE,
        b"foreach" => KW_FOREACH,
        b"for" => KW_FOR,
        b"while" => KW_WHILE,
        b"do" => KW_DO,
        b"until" => KW_UNTIL,
        b"in" => KW_IN,
        b"switch" => KW_SWITCH,
        b"default" => KW_DEFAULT,
        b"try" => KW_TRY,
        b"catch" => KW_CATCH,
        b"finally" => KW_FINALLY,
        b"trap" => KW_TRAP,
        b"begin" => KW_BEGIN,
        b"process" => KW_PROCESS,
        b"end" => KW_END,
        b"namespace" => KW_NAMESPACE,
        b"module" => KW_MODULE,
        b"assembly" => KW_ASSEMBLY,
        b"string" => KW_STRING_TYPE,
        b"int" => KW_INT_TYPE,
        b"bool" => KW_BOOL_TYPE,
        b"object" => KW_OBJECT_TYPE,
        _ => IDENT,
    }
}

fn variable(c: &mut Cursor) -> Kind {
    c.bump();
    if c.peek() == b'{' {
        c.bump();
        c.eat_while(|byte| byte != b'}' && byte != b'\n');
        c.eat(b'}');
        return VARIABLE;
    }
    c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'?'));
    let text = c.since(0);
    match &text[..text.len().min(6)] {
        b"$true" if text.len() == 5 => return KW_TRUE,
        b"$false" if text.len() == 6 => return KW_FALSE,
        b"$null" if text.len() == 5 => return KW_NULL,
        _ => {}
    }
    VARIABLE
}

fn single_quoted(c: &mut Cursor) -> Kind {
    c.bump();
    loop {
        if c.at_end() || c.peek() == b'\n' {
            return BAD_STRING;
        }
        if c.peek() == b'\'' {
            c.bump();
            if c.peek() == b'\'' {
                c.bump();
                continue;
            }
            return STRING;
        }
        c.bump_char();
    }
}

fn double_quoted(c: &mut Cursor) -> Kind {
    c.bump();
    loop {
        if c.at_end() || c.peek() == b'\n' {
            return BAD_STRING;
        }
        match c.peek() {
            b'`' => c.bump_n(2),
            b'"' => {
                c.bump();
                if c.peek() == b'"' {
                    c.bump();
                    continue;
                }
                return STRING;
            }
            _ => c.bump_char(),
        }
    }
}

/// `@"` or `@'` at a line end, through the matching `"@` / `'@` at a line start.
fn here_string(c: &mut Cursor) -> Kind {
    let quote = c.peek_at(1);
    let close = if quote == b'"' { b'"' } else { b'\'' };
    c.bump_n(2);
    loop {
        if c.at_end() {
            return BAD_STRING;
        }
        if c.peek() == close && c.peek_at(1) == b'@' {
            c.bump_n(2);
            return STRING;
        }
        c.bump_char();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_f::harness;

    const VALID: &[(&str, &[&str])] = &[
        (
            "Build.ps1",
            &[
                "param_declaration",
                "function_definition",
                "pipeline",
                "foreach_statement",
                "pipeline",
                "if_statement",
                "for_statement",
            ],
        ),
        ("Deploy.ps1", &["using_statement", "class_definition", "pipeline", "pipeline"]),
        (
            "Report.ps1",
            &[
                "function_definition",
                "pipeline",
                "do_statement",
                "while_statement",
                "pipeline",
                "pipeline",
            ],
        ),
    ];

    #[test]
    fn valid_fixtures() {
        harness::check_valid("powershell", &POWERSHELL, VALID);
    }

    #[test]
    fn malformed_keeps_the_root() {
        harness::check_malformed(
            "malformed",
            "powershell_broken.ps1",
            &POWERSHELL,
            &["function_definition"],
        );
    }

    #[test]
    #[ignore]
    fn dump_top_level_kinds() {
        harness::dump("powershell", &POWERSHELL);
        harness::dump("malformed", &POWERSHELL);
    }

    #[test]
    fn lexes_a_parameter_an_operator_and_a_variable() {
        use crate::deep::lexer::{lex, Tokens};
        let src = b"Get-Item -Path $p -eq 1";
        let mut tokens = Tokens::default();
        lex(src, &POWERSHELL, &mut tokens);
        let kinds: Vec<&str> = (0..tokens.len())
            .map(|i| POWERSHELL.name(tokens.kind[i]))
            .filter(|kind| *kind != "whitespace")
            .collect();
        assert_eq!(kinds, ["identifier", "parameter", "variable", "-eq", "number"]);
    }
}
