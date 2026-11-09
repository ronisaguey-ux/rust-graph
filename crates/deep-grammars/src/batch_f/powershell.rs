//! PowerShell: a script of pipelines, functions, classes and control flow.
//!
//! PowerShell is line-oriented: a statement ends at a newline or `;`. The lexer
//! emits `NEWLINE` and the grammar consumes it as `_statement_end`. `#` starts a
//! line comment and `<# ... #>` a block comment; a backtick continues a line.
//!
//! # Modes
//!
//! A double-quoted string is not opaque: `"$name"` and `"$(Get-Date)"` expand,
//! so the opening quote is a `string_open` token that enters `STRING`, the text
//! between expansions is `string_text`, and `$( )` inside it is a subexpression
//! node. A subexpression is code again: `$( ... )` pushes `CODE` (the PowerShell
//! lexer itself) and the `)` at its own bracket depth closes it, so
//! `$($item.Name)` is a `subexpression` holding a `member_access`.
//!
//! Also modelled: `@(...)` array literals, `@{ ... }` hashtables, `@args`
//! splatting, attributes with arguments (`[Parameter(Mandatory = $false)]`),
//! scope-qualified names (`global:deactivate`), and here-strings, whose closing
//! `"@` must sit at the start of a line.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A command and its arguments are two shapes.** `Get-Item -Path $p` is a
//!   `command_expression` (a bare name, then parameters and arguments), while
//!   `$p.Length` is an expression. The two are told apart by a predicate on what
//!   follows the name, so an invocation whose only argument is a script block
//!   (`Where-Object { ... }`) gets its own rule and is tried before the pipeline.
//! - **`-eq`, `-and` and friends are distinct tokens**, matched by the lexer, so
//!   an unknown dash-word is a parameter, not an operator.
//! - **`$x = 1` is an assignment expression**, because the same `=` is the
//!   operator in `$x = 1` and inside a `for` header.

use std::cell::RefCell;

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { POWERSHELL {
    name = "powershell";
    lex = lex_powershell;

    modes {
        CODE = lex_powershell,
        STRING = lex_string_text,
    }
    switch STRING_OPEN => push STRING;
    switch STRING_CLOSE => pop;
    switch STRING_BAD => pop;
    switch SUBEXP_OPEN => push CODE;
    switch SUBEXP_CLOSE => pop;

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
        RANGE = "..",
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
        SPLIT_OP = "-split",
        JOIN_OP = "-join",
        IS_OP = "-is",
        ISNOT_OP = "-isnot",
        AS_OP = "-as",
        IDENT = "identifier",
        PARAM = "parameter",
        VARIABLE = "variable",
        NUMBER = "number",
        STRING = "string_literal",
        /// A single-quoted string: no expansion, so one token.
        LITERAL_STRING = "literal_string",
        BAD_STRING = "bad_string" bad,
        /// The `"` that opens an expanding string: the switch pushes `STRING`.
        STRING_OPEN = "string_open",
        STRING_CLOSE = "string_close",
        /// The text of a string between its expansions.
        STRING_TEXT = "string_text",
        /// `[Severity]` in `[Severity]::Low`: one token, so the `[` of a type
        /// annotation stays distinguishable from the `[` of a declaration.
        TYPE_NAME = "type_name",
        /// `hidden` or `static` before a member: a token, so a class member
        /// cannot be read as an index expression.
        CLASS_MODIFIER = "class_modifier",
        /// A string that cannot close: the mode pops on it.
        STRING_BAD = "bad_expanding_string" bad,
        /// `$(`: the switch pushes `CODE` so the inside is a subtree.
        SUBEXP_OPEN = "$(",
        /// The `)` that closes a subexpression at its own bracket depth.
        SUBEXP_CLOSE = "subexp_close",
        /// `@(`: an array literal.
        AT_PAREN = "@(",
        /// `@{`: a hashtable literal.
        AT_BRACE = "@{",
        /// `@args`: splatting.
        SPLAT = "splat",

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

    /// `function global:deactivate ([switch]$x) { }`: the name may be
    /// scope-qualified, and the parameter list may sit before the body.
    function_definition: ("function" | "filter") _name [parameter_list] block,
    class_definition: "class" _name ["extends" _type_name] class_body,
    class_body: "{" (_statement_end | class_member | _line_item)* "}",
    class_member: [CLASS_MODIFIER] type_annotation class_member_tail,
    class_member_tail: _name parameter_list block | VARIABLE ["=" _expression],
    enum_definition: "enum" _name enum_body,
    enum_body: "{" (_statement_end | _line_item)* "}",
    using_statement: "using" ("namespace" | "module" | "assembly") _name,

    /// PowerShell allows line breaks inside a parameter list.
    parameter_list: "(" _nl [parameter ("," _nl parameter)*] _nl ")",
    _nl: NEWLINE*,
    parameter: (type_annotation _nl)* VARIABLE ["=" _expression],
    param_declaration: "param" parameter_list,

    /// `[string]`, `[System.IO.Path]`, and an attribute such as
    /// `[Parameter(Mandatory = $false)]`, which is a type reference with an
    /// argument list.
    type_annotation: "[" _type_reference "]",
    _type_reference: _type_name [attribute_suffix],
    attribute_suffix: "(" _attribute_arguments ")",
    /// `[int]$x`, `[Severity]::Low`, `[string]::Empty`: a type name used as an
    /// expression, optionally with a static member or a cast operand.
    static_type_member: TYPE_NAME DOUBLE_COLON _name,
    _type_name: _name ("." _name)*,
    _attribute_arguments: (_attribute_argument % ","),
    _attribute_argument: &(_name "=") named_argument | _expression,
    named_argument: _name "=" _expression,

    block: "{" (_statement_end | _line_item)* "}",

    if_statement: "if" paren_expression block _nl (elseif_clause)* [else_clause],
    elseif_clause: "elseif" paren_expression block _nl,
    else_clause: "else" block,
    foreach_statement: "foreach" foreach_header block,
    foreach_header: "(" VARIABLE "in" _expression ")",
    for_statement: "for" for_header block,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression] ")",
    _for_init: pipeline,
    while_statement: "while" paren_expression block,
    do_statement: "do" block _nl ("while" | "until") paren_expression,
    switch_statement: "switch" paren_expression switch_body,
    switch_body: "{" (_statement_end | switch_clause)* "}",
    switch_clause: switch_label block,
    switch_label: _expression | "default",
    try_statement: "try" block _nl (catch_clause)* [finally_clause],
    catch_clause: "catch" [catch_type] block _nl,
    catch_type: "[" _type_name "]",
    finally_clause: "finally" block,
    return_statement: "return" [_expression],
    throw_statement: "throw" [_expression],
    break_statement: "break",
    continue_statement: "continue",

    paren_expression: "(" _expression ")",

    pipeline: _expression ("," _nl _expression)* ("|" _nl _expression)*,
    /// A bare command name with at least one argument is an expression here:
    /// `$x = Get-Item -Path "."` is an assignment whose right side is this node.
    command_expression: IDENT _command_argument+,
    _command_argument: parameter_argument | _expression,
    parameter_argument: PARAM [[PARAM] _expression],

    _expression: precedence _primary {
        right "=" | "+=" | "-=" => assignment,
        left "-or" => logical_or,
        left "-and" => logical_and,
        left "-eq" | "-ne" | "-lt" | "-gt" | "-le" | "-ge" | "-like" | "-notlike" | "-match" | "-notmatch" | "-contains" | "-in" | "-replace" | "-is" | "-isnot" | "-as" => comparison,
        left "-split" | "-join" => comparison,
        left ".." => range_expression,
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
        | &(IDENT LITERAL_STRING) command_expression
        | &(IDENT NUMBER) command_expression
        | &(IDENT VARIABLE) command_expression
        | &(IDENT IDENT) command_expression
        | &(IDENT SPLAT) command_expression
        | _name
        | VARIABLE
        | NUMBER
        | "$true"
        | "$false"
        | "$null"
        | string_literal
        | splat_argument
        | array_literal
        | hashtable_literal
        | subexpression
        | static_type_member
        | parenthesized_expression
        | path_dot
        | script_block,

    _name: IDENT | "string" | "int" | "bool" | "object",
    /// `.` on its own is the current directory, and a valid argument.
    path_dot: ".",
    parenthesized_expression: "(" _nl pipeline _nl ")",
    script_block: "{" (_statement_end | _line_item)* "}",
    array_literal: AT_PAREN (_statement_end | _array_element)* RPAREN,
    _array_element: _expression [","],
    hashtable_literal: AT_BRACE (_statement_end | hashtable_entry)* RBRACE,
    hashtable_entry: hashtable_key [ASSIGN _expression],
    hashtable_key: IDENT | NUMBER | string_literal | parenthesized_expression,

    /// `$($item.Name)` in code and in a string: the inside is ordinary code.
    subexpression: SUBEXP_OPEN (_statement_end | _line_item)* SUBEXP_CLOSE,
    splat_argument: SPLAT,

    string_literal: LITERAL_STRING | STRING | expanding_string,
    expanding_string: STRING_OPEN (_string_part)* (STRING_CLOSE | STRING_BAD),
    _string_part: STRING_TEXT | VARIABLE | subexpression,
} }

/// What one open frame is: a string's mode state, or a `$( ... )` whose `)`
/// closes it at its own bracket depth.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    String,
    Subexpression,
}

struct Frame {
    kind: FrameKind,
    /// Openers minus closers inside a subexpression.
    depth: u32,
}

thread_local! {
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

fn with_frames<T>(action: impl FnOnce(&mut Vec<Frame>) -> T) -> T {
    FRAMES.with(|frames| action(&mut frames.borrow_mut()))
}

fn push_frame(kind: FrameKind) {
    with_frames(|frames| frames.push(Frame { kind, depth: 0 }));
}

fn pop_frame() {
    with_frames(|frames| {
        frames.pop();
    });
}

fn top_kind() -> Option<FrameKind> {
    with_frames(|frames| frames.last().map(|frame| frame.kind))
}

fn adjust_depth(change: i32) {
    with_frames(|frames| {
        if let Some(frame) = frames.last_mut() {
            if frame.kind == FrameKind::Subexpression {
                frame.depth = if change < 0 { frame.depth.saturating_sub(1) } else { frame.depth + 1 };
            }
        }
    });
}

pub fn lex_powershell(c: &mut Cursor) -> Kind {
    use crate::batch_d::lex;
    if c.pos() == 0 {
        with_frames(|frames| frames.clear());
    }
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
        b'\n' => {
            c.bump();
            NEWLINE
        }
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
        b'"' => expanding_string(c),
        b'@' if matches!(c.peek_at(1), b'"' | b'\'') && matches!(c.peek_at(2), b'\n' | b'\r') => here_string(c),
        b'@' if c.peek_at(1) == b'(' => {
            c.bump_n(2);
            AT_PAREN
        }
        b'@' if c.peek_at(1) == b'{' => {
            c.bump_n(2);
            AT_BRACE
        }
        b'@' if is_word_start(c.peek_at(1)) => {
            c.bump();
            c.eat_while(is_name_byte);
            SPLAT
        }
        b'$' => variable(c),
        b'(' => {
            c.bump();
            adjust_depth(1);
            LPAREN
        }
        b')' => {
            if top_kind() == Some(FrameKind::Subexpression) && frame_depth() == 0 {
                c.bump();
                pop_frame();
                SUBEXP_CLOSE
            } else {
                c.bump();
                adjust_depth(-1);
                RPAREN
            }
        }
        b'0'..=b'9' => {
            lex::number(c);
            NUMBER
        }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => word(c),
        b'[' => {
            // `[Severity]::Low`: a bracketed type immediately followed by `::`
            // is one token, so a declaration's `[` stays distinct from it.
            if type_before_static(c) {
                while !c.at_end() && c.peek() != b']' && c.peek() != b'\n' {
                    c.bump_char();
                }
                c.eat(b']');
                return TYPE_NAME;
            }
            c.bump();
            LBRACKET
        }
        b']' => {
            c.bump();
            RBRACKET
        }
        b'{' => {
            c.bump();
            LBRACE
        }
        b'}' => {
            c.bump();
            RBRACE
        }
        b',' => {
            c.bump();
            COMMA
        }
        b';' => {
            c.bump();
            SEMI
        }
        b'.' if c.peek_at(1) == b'.' => {
            c.bump_n(2);
            RANGE
        }
        b'.' => {
            c.bump();
            DOT
        }
        b'|' => {
            c.bump();
            PIPE
        }
        b'&' if c.peek_at(1) == b'&' => {
            c.bump_n(2);
            AND
        }
        b'+' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            PLUS_ASSIGN
        }
        b'+' if c.peek_at(1) == b'+' => {
            c.bump_n(2);
            INCR
        }
        b'+' => {
            c.bump();
            PLUS
        }
        b'-' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            MINUS_ASSIGN
        }
        b'-' if c.peek_at(1) == b'-' => {
            c.bump_n(2);
            DECR
        }
        b'-' => dash_word(c),
        b'=' => {
            c.bump();
            ASSIGN
        }
        b'*' | b'?' if wildcard(c) => wildcard_word(c),
        b'*' => {
            c.bump();
            STAR
        }
        b'/' => {
            c.bump();
            SLASH
        }
        b'%' => {
            c.bump();
            PERCENT
        }
        b'!' => {
            c.bump();
            BANG
        }
        b':' if c.peek_at(1) == b':' => {
            c.bump_n(2);
            DOUBLE_COLON
        }
        _ => UNKNOWN,
    }
}

fn frame_depth() -> u32 {
    with_frames(|frames| frames.last().map_or(0, |frame| frame.depth))
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// `*.ps1` and `?.txt` are bare arguments, not operators: a wildcard after a
/// parameter (`-Filter *.cs`) or a command name starts a word.
fn wildcard(c: &Cursor) -> bool {
    let next = c.peek_at(1);
    let word_shaped = matches!(next, b'.' | b'*' | b'?') || is_name_byte(next);
    word_shaped && matches!(c.previous(), PARAM | IDENT | LITERAL_STRING | STRING | VARIABLE | COMMA | LPAREN)
}

fn wildcard_word(c: &mut Cursor) -> Kind {
    while !c.at_end() {
        match c.peek() {
            b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b')' | b'}' | b',' => break,
            _ => c.bump_char(),
        }
    }
    IDENT
}

/// Whether the `[` at the cursor opens a type name that is immediately followed
/// by `::` — `[Severity]::Low` — rather than a declaration's annotation.
fn type_before_static(c: &Cursor) -> bool {
    let mut at = 1;
    while at < 128 {
        match c.peek_at(at) {
            b']' => return c.peek_at(at + 1) == b':' && c.peek_at(at + 2) == b':',
            b'\n' | b'\r' | b' ' | b'\t' | b'(' | b'[' | 0 => return false,
            _ => at += 1,
        }
    }
    false
}

/// `-eq`, `-and`, or a parameter `-Path`. The operator table is checked first,
/// so an operator name is never a parameter.
fn dash_word(c: &mut Cursor) -> Kind {
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
        b"-split" => SPLIT_OP,
        b"-join" => JOIN_OP,
        b"-is" => IS_OP,
        b"-isnot" => ISNOT_OP,
        b"-as" => AS_OP,
        b"-" => MINUS,
        _ => PARAM,
    }
}

fn word(c: &mut Cursor) -> Kind {
    let begin = c.pos();
    let rest = c.rest();
    c.eat_while(is_name_byte);
    // A cmdlet name keeps its dash: `Get-Item`, `Write-Output`.
    while c.peek() == b'-' && c.peek_at(1).is_ascii_alphabetic() {
        c.bump();
        c.eat_while(is_name_byte);
    }
    // A scope qualifier keeps its colon: `global:deactivate`,
    // `Function:prompt`, `env:PATH` as a bare word.
    if c.peek() == b':' && c.peek_at(1) != b':' && is_word_start(c.peek_at(1)) {
        c.bump();
        c.eat_while(is_name_byte);
        while c.peek() == b'-' && c.peek_at(1).is_ascii_alphabetic() {
            c.bump();
            c.eat_while(is_name_byte);
        }
    }
    let length = c.pos() - begin;
    let text = &rest[..length];
    // `hidden [int]$x` and `static [string]$y`: a class modifier is a token of
    // its own, so the member cannot be read as `hidden[int]`.
    if matches!(text, b"hidden" | b"static") {
        let mut at = 0;
        while matches!(c.peek_at(at), b' ' | b'\t') {
            at += 1;
        }
        if c.peek_at(at) == b'[' {
            return CLASS_MODIFIER;
        }
    }
    // `[switch]$x`: inside brackets the type name is a name, not the keyword.
    if text == b"switch" && c.peek() == b']' {
        return IDENT;
    }
    keyword(text)
}

fn keyword(text: &[u8]) -> Kind {
    // PowerShell is case-insensitive: `If`, `Param` and `Function` are keywords
    // as much as `if`, `param` and `function`.
    let mut lower = [0u8; 16];
    let length = text.len().min(lower.len());
    for (slot, byte) in lower.iter_mut().zip(text.iter()) {
        *slot = byte.to_ascii_lowercase();
    }
    let text = if text.len() <= lower.len() { &lower[..length] } else { text };
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

/// `$name`, `$env:PATH`, `${weird name}`, `$true`, and `$(`: a subexpression,
/// which is code again until the `)` that matches it.
fn variable(c: &mut Cursor) -> Kind {
    if c.peek_at(1) == b'(' {
        c.bump_n(2);
        push_frame(FrameKind::Subexpression);
        return SUBEXP_OPEN;
    }
    c.bump();
    if c.peek() == b'{' {
        c.bump();
        c.eat_while(|byte| byte != b'}' && byte != b'\n');
        c.eat(b'}');
        return VARIABLE;
    }
    c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'?'));
    let text = c.since(0);
    if text.eq_ignore_ascii_case(b"$true") {
        return KW_TRUE;
    }
    if text.eq_ignore_ascii_case(b"$false") {
        return KW_FALSE;
    }
    if text.eq_ignore_ascii_case(b"$null") {
        return KW_NULL;
    }
    VARIABLE
}

/// `'...'`: no expansion, and `''` is an escaped quote.
fn single_quoted(c: &mut Cursor) -> Kind {
    c.bump();
    while !c.at_end() {
        if c.peek() == b'\'' {
            c.bump();
            if c.peek() == b'\'' {
                c.bump();
                continue;
            }
            return LITERAL_STRING;
        }
        c.bump_char();
    }
    BAD_STRING
}

/// `"`: the switch enters `STRING`, so `$name` and `$( )` inside are nodes.
fn expanding_string(c: &mut Cursor) -> Kind {
    c.bump();
    push_frame(FrameKind::String);
    STRING_OPEN
}

/// The `STRING` mode's step: text, one expansion, or the closing quote.
pub fn lex_string_text(c: &mut Cursor) -> Kind {
    if top_kind() != Some(FrameKind::String) {
        // The mode stack and this stack disagree; lexing code cannot lose a
        // token, and the parser still gets a tree.
        return lex_powershell(c);
    }
    if c.at_end() {
        pop_frame();
        return STRING_BAD;
    }
    match c.peek() {
        b'"' => {
            c.bump();
            if c.peek() == b'"' {
                // `""` inside an expanding string is one literal quote.
                c.bump();
                return STRING_TEXT;
            }
            pop_frame();
            STRING_CLOSE
        }
        b'$' if c.peek_at(1) == b'(' => {
            c.bump_n(2);
            push_frame(FrameKind::Subexpression);
            SUBEXP_OPEN
        }
        b'$' if is_word_start(c.peek_at(1)) || c.peek_at(1) == b'{' => {
            string_variable(c);
            VARIABLE
        }
        _ => {
            let start = c.pos();
            while !c.at_end() {
                match c.peek() {
                    b'"' => break,
                    b'`' => c.bump_n(2),
                    b'$' if is_word_start(c.peek_at(1)) || c.peek_at(1) == b'(' || c.peek_at(1) == b'{' => break,
                    _ => c.bump_char(),
                }
            }
            if c.pos() == start {
                c.bump_char();
            }
            STRING_TEXT
        }
    }
}

/// A `$name` inside a string: the bytes are the same as in code.
fn string_variable(c: &mut Cursor) {
    c.bump();
    if c.peek() == b'{' {
        c.bump();
        c.eat_while(|byte| byte != b'}' && byte != b'\n' && byte != b'"');
        c.eat(b'}');
        return;
    }
    c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'));
}

/// `@"` or `@'` at a line end, through the `"@` or `'@` that starts a line.
fn here_string(c: &mut Cursor) -> Kind {
    let quote = c.peek_at(1);
    let close = if quote == b'"' { b'"' } else { b'\'' };
    c.bump_n(2);
    while !c.at_end() {
        if c.behind(1) == b'\n' && c.peek() == close && c.peek_at(1) == b'@' {
            c.bump_n(2);
            return STRING;
        }
        c.bump_char();
    }
    BAD_STRING
}

