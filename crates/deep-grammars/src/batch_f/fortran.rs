//! Fortran, fixed form: the column rules are the lexer's job.
//!
//! A fixed-form line is five fields: columns 1-5 are a statement label,
//! column 6 is the continuation mark, columns 7-72 are the statement, and
//! anything after column 72 is a comment to the interpreter. This lexer
//! consumes columns 1-6 at every line start: column 1 holding `C`, `c`, `*` or
//! `!` makes the line a comment, a non-blank non-zero column 6 makes the line a
//! continuation of the statement before it (so no `NEWLINE` is produced), and
//! otherwise the prefix is skipped and a `NEWLINE` token ends the previous
//! statement.
//!
//! Keywords are matched after folding to upper case; Fortran itself is
//! case-insensitive and the tree keeps the tokens as written.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Columns 73 and beyond are not dropped.** The step lexer has no way back
//!   to the start of the line it is inside, so it cannot compute the current
//!   column; a comment starting in column 73 would be lexed as code.
//! - **Free form is not modelled.** `.f90` sources with `&` continuations are
//!   not this grammar.
//! - **A statement label in columns 1-5** is skipped with the rest of the
//!   prefix: `100 CONTINUE` has no label node.
//! - **`format` statements, `interface` blocks and coarrays** are not modelled.
//! - **Array constructors `(/ ... /)`** and complex literals `(1.0, 2.0)` are
//!   parenthesised expressions, not their own nodes.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { FORTRAN {
    name = "fortran";
    lex = lex_fortran;

    tokens {
        NEWLINE = "newline",
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        COMMA = ",",
        COLON = ":",
        DOUBLE_COLON = "::",
        ASSIGN = "=",
        EQ = "==",
        NE = "/=",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        STARSTAR = "**",
        SLASHSLASH = "//",
        PERCENT = "%",
        AMP = "&",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,
        DOT_AND = ".and.",
        DOT_OR = ".or.",
        DOT_NOT = ".not.",
        DOT_EQ = ".eq.",
        DOT_NE = ".ne.",
        DOT_LT = ".lt.",
        DOT_GT = ".gt.",
        DOT_LE = ".le.",
        DOT_GE = ".ge.",

        KW_PROGRAM = "program",
        KW_MODULE = "module",
        KW_SUBROUTINE = "subroutine",
        KW_FUNCTION = "function",
        KW_END = "end",
        KW_USE = "use",
        KW_ONLY = "only",
        KW_IMPLICIT = "implicit",
        KW_NONE = "none",
        KW_INTEGER = "integer",
        KW_REAL = "real",
        KW_DOUBLE = "double",
        KW_PRECISION = "precision",
        KW_CHARACTER = "character",
        KW_LOGICAL = "logical",
        KW_COMPLEX = "complex",
        KW_DIMENSION = "dimension",
        KW_PARAMETER = "parameter",
        KW_INTENT = "intent",
        KW_IN = "in",
        KW_OUT = "out",
        KW_INOUT = "inout",
        KW_ALLOCATABLE = "allocatable",
        KW_SAVE = "save",
        KW_TYPE = "type",
        KW_CALL = "call",
        KW_PRINT = "print",
        KW_WRITE = "write",
        KW_READ = "read",
        KW_IF = "if",
        KW_THEN = "then",
        KW_ELSE = "else",
        KW_ELSEIF = "elseif",
        KW_ELSEIF_SPELLED = "else if",
        KW_DO = "do",
        KW_WHILE = "while",
        KW_SELECT = "select",
        KW_CASE = "case",
        KW_DEFAULT = "default",
        KW_RETURN = "return",
        KW_STOP = "stop",
        KW_CONTINUE = "continue",
        KW_CONTAINS = "contains",
        KW_RESULT = "result",
        KW_EXTERNAL = "external",
        KW_GO = "go",
        KW_TO = "to",
        KW_CYCLE = "cycle",
        KW_EXIT = "exit",
        KW_ALLOCATE = "allocate",
        KW_DEALLOCATE = "deallocate",
        KW_TRUE = ".true.",
        KW_FALSE = ".false.",
    }

    program: _item*,

    _item: NEWLINE | _statement,

    /// The line break is reported at the start of the next line, so a file may
    /// end without one: the terminator is optional and the next statement
    /// begins after it.
    _statement: _statement_kind [NEWLINE] | end_statement [NEWLINE],
    /// The items of a `then`/`else`/`do` body: an `END` there closes the block,
    /// so it is not one of them.
    _body_item: NEWLINE | _body_statement,
    _body_statement: _statement_kind [NEWLINE],

    _statement_kind:
        program_statement
        | module_statement
        | subroutine_statement
        | function_statement
        | use_statement
        | implicit_statement
        | declaration
        | call_statement
        | io_statement
        | if_statement
        | do_statement
        | return_statement
        | stop_statement
        | contains_statement
        | assignment_statement,

    program_statement: "program" _name,
    module_statement: "module" _name,
    subroutine_statement: "subroutine" _name [parameter_list],
    function_statement: "function" _name [parameter_list] ["result" paren_name],
    end_statement: "end" [end_kind],
    end_kind:
        "program" | "module" | "subroutine" | "function" | "do" | "if" | "select" | "type" | "while",

    use_statement: "use" _name [use_only_list],
    use_only_list: "," "only" ":" _name ("," _name)*,

    implicit_statement: "implicit" "none",

    declaration: type_spec [attribute_clause] [declaration_separator] entity_list,
    attribute_clause: "," attribute ("," attribute)*,
    type_spec:
        "integer" [paren_expression]
        | "real" [paren_expression]
        | "double" "precision"
        | "character" [paren_expression]
        | "logical"
        | "complex"
        | "type" paren_name,
    attribute:
        "dimension" paren_expression
        | "intent" paren_expression
        | "parameter"
        | "allocatable"
        | "save"
        | "external"
        | "in"
        | "out"
        | "inout",
    declaration_separator: "," | "::",
    entity_list: entity ("," entity)*,
    entity: _name [paren_expression] ["=" _expression],
    paren_name: "(" _name ")",

    parameter_list: "(" [_name ("," _name)*] ")",

    call_statement: "call" _name [argument_list],
    io_statement: ("print" | "write" | "read") io_control [","] [io_items],
    io_control: io_control_list | "*",
    io_control_list: "(" io_control_item ("," io_control_item)* ")",
    io_control_item: "*" | _expression,
    io_items: _expression_list,

    if_statement:
        "if" paren_expression "then" NEWLINE _body_item* [else_clause] "end" "if",
    else_clause: "else" NEWLINE _body_item*,
    do_statement: "do" [do_header] NEWLINE _body_item* "end" "do",
    do_header: _name "=" _expression "," _expression ["," _expression] | "while" paren_expression,

    return_statement: "return",
    stop_statement: "stop",
    contains_statement: "contains",

    assignment_statement: _expression,

    argument_list: "(" [_argument %? ","] ")",
    _argument: _expression,
    _expression_list: _expression ("," _expression)*,

    _expression: precedence _primary {
        right "=" => assignment,
        left ".or." => logical_or,
        left ".and." => logical_and,
        left "==" | "/=" | "<" | ">" | "<=" | ">=" | ".eq." | ".ne." | ".lt." | ".gt." | ".le." | ".ge." => comparison,
        left "//" => concatenation,
        left "+" | "-" => additive,
        left "*" | "/" => multiplicative,
        right "**" => power,
        prefix "-" | "+" | ".not." => unary,
        postfix argument_list => call,
        postfix "%" _name => member_access,
    },

    _primary:
        _name
        | NUMBER
        | STRING
        | ".true."
        | ".false."
        | parenthesized_expression,

    /// `RESULT` is a keyword only in `function f(...) result(r)`; a program is
    /// free to call a variable `result`.
    _name: IDENT | "result",
    paren_expression: "(" _expression ")",
    parenthesized_expression: "(" _expression ")",
} }

/// True where a physical line starts: the first byte of the file or one after
