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
