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
/// a line break.
fn at_line_start(c: &Cursor) -> bool {
    c.pos() == 0 || c.behind(1) == b'\n'
}

/// Column 1 of a line: `C`, `c`, `*` or `!` makes it a comment line.
fn comment_line(c: &Cursor) -> bool {
    matches!(c.peek(), b'c' | b'C' | b'*' | b'!')
}

pub fn lex_fortran(c: &mut Cursor) -> Kind {
    if at_line_start(c) {
        return line_prefix(c);
    }
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\x0c'));
            WHITESPACE
        }
        b'\n' => {
            c.bump();
            // A line break is reported at the start of the next line, where the
            // continuation column is known.
            WHITESPACE
        }
        b'!' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'"' | b'\'' => {
            let quote = c.peek();
            c.bump();
            if c.quoted(quote, true) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'0'..=b'9' => { crate::batch_d::lex::number(c); NUMBER }
        b'.' if c.peek_at(1).is_ascii_alphabetic() => dotted_word(c),
        b'.' if c.peek_at(1).is_ascii_digit() => { c.bump(); crate::batch_d::lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
            keyword(&crate::batch_d::lex::upper(&start[..length]))
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b',' => { c.bump(); COMMA }
        b':' if c.peek_at(1) == b':' => { c.bump_n(2); DOUBLE_COLON }
        b':' => { c.bump(); COLON }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' => { c.bump(); ASSIGN }
        b'/' if c.peek_at(1) == b'/' => { c.bump_n(2); SLASHSLASH }
        b'/' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'/' => { c.bump(); SLASH }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' => { c.bump(); GT }
        b'*' if c.peek_at(1) == b'*' => { c.bump_n(2); STARSTAR }
        b'*' => { c.bump(); STAR }
        b'+' => { c.bump(); PLUS }
        b'-' => { c.bump(); MINUS }
        b'%' => { c.bump(); PERCENT }
        b'&' => { c.bump(); AMP }
        _ => UNKNOWN,
    }
}

/// The fixed-form prefix at a line start: a comment, a continuation, or a
/// `NEWLINE` for the statement the previous line held.
fn line_prefix(c: &mut Cursor) -> Kind {
    if comment_line(c) {
        c.eat_while(|byte| byte != b'\n');
        return COMMENT;
    }
    // Columns 1-6, stopping at a line that ends inside the prefix.
    let mut column = 0;
    let mut continuation = false;
    while column < 6 {
        match c.peek() {
            // A line shorter than the prefix field: its line break is this
            // token's text, so the step advances.
            b'\n' | b'\r' => {
                c.bump();
                c.eat(b'\n');
                return NEWLINE;
            }
            byte => {
                if column == 5 {
                    continuation = byte != b' ' && byte != b'0' && byte != b'\t';
                }
                c.bump();
                column += 1;
            }
        }
    }
    if continuation {
        // The statement goes on: no `NEWLINE` and the prefix is whitespace.
        WHITESPACE
    } else {
        NEWLINE
    }
}

/// `.and.`, `.true.` and the rest: a dot, letters, a dot.
fn dotted_word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    c.bump();
    let length = 1 + c.eat_while(|byte| byte.is_ascii_alphabetic());
    if c.peek() == b'.' {
        c.bump();
    }
    let text: Vec<u8> = start[..length + 1].iter().map(u8::to_ascii_lowercase).collect();
    match &text[..] {
        b".and." => DOT_AND,
        b".or." => DOT_OR,
        b".not." => DOT_NOT,
        b".eq." => DOT_EQ,
        b".ne." => DOT_NE,
        b".lt." => DOT_LT,
        b".gt." => DOT_GT,
        b".le." => DOT_LE,
        b".ge." => DOT_GE,
        b".true." => KW_TRUE,
        b".false." => KW_FALSE,
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"PROGRAM" => KW_PROGRAM,
        b"MODULE" => KW_MODULE,
        b"SUBROUTINE" => KW_SUBROUTINE,
        b"FUNCTION" => KW_FUNCTION,
        b"END" => KW_END,
        b"USE" => KW_USE,
        b"ONLY" => KW_ONLY,
        b"IMPLICIT" => KW_IMPLICIT,
        b"NONE" => KW_NONE,
        b"INTEGER" => KW_INTEGER,
        b"REAL" => KW_REAL,
        b"DOUBLE" => KW_DOUBLE,
        b"PRECISION" => KW_PRECISION,
        b"CHARACTER" => KW_CHARACTER,
        b"LOGICAL" => KW_LOGICAL,
        b"COMPLEX" => KW_COMPLEX,
        b"DIMENSION" => KW_DIMENSION,
        b"PARAMETER" => KW_PARAMETER,
        b"INTENT" => KW_INTENT,
        b"IN" => KW_IN,
        b"OUT" => KW_OUT,
        b"INOUT" => KW_INOUT,
        b"ALLOCATABLE" => KW_ALLOCATABLE,
        b"SAVE" => KW_SAVE,
        b"TYPE" => KW_TYPE,
        b"CALL" => KW_CALL,
        b"PRINT" => KW_PRINT,
        b"WRITE" => KW_WRITE,
        b"READ" => KW_READ,
        b"IF" => KW_IF,
        b"THEN" => KW_THEN,
        b"ELSE" => KW_ELSE,
        b"ELSEIF" => KW_ELSEIF,
        b"DO" => KW_DO,
        b"WHILE" => KW_WHILE,
        b"SELECT" => KW_SELECT,
        b"CASE" => KW_CASE,
        b"DEFAULT" => KW_DEFAULT,
        b"RETURN" => KW_RETURN,
        b"STOP" => KW_STOP,
        b"CONTINUE" => KW_CONTINUE,
        b"CONTAINS" => KW_CONTAINS,
        b"RESULT" => KW_RESULT,
        b"EXTERNAL" => KW_EXTERNAL,
        b"GO" => KW_GO,
        b"TO" => KW_TO,
        b"CYCLE" => KW_CYCLE,
        b"EXIT" => KW_EXIT,
        b"ALLOCATE" => KW_ALLOCATE,
        b"DEALLOCATE" => KW_DEALLOCATE,
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_f::harness;

    const VALID: &[(&str, &[&str])] = &[
        (
            "main.f",
            &[
                "program_statement",
                "implicit_statement",
                "declaration",
                "declaration",
                "assignment_statement",
                "assignment_statement",
                "do_statement",
                "if_statement",
                "call_statement",
                "stop_statement",
                "end_statement",
                "subroutine_statement",
                "implicit_statement",
                "declaration",
                "io_statement",
                "return_statement",
                "end_statement",
            ],
        ),
        (
            "geometry.f",
            &[
                "module_statement",
                "implicit_statement",
                "declaration",
                "contains_statement",
                "function_statement",
                "implicit_statement",
                "declaration",
                "assignment_statement",
                "return_statement",
                "end_statement",
                "end_statement",
                "program_statement",
                "use_statement",
                "implicit_statement",
                "declaration",
                "assignment_statement",
                "assignment_statement",
                "io_statement",
                "end_statement",
            ],
        ),
        (
            "counter.f",
            &[
                "program_statement",
                "implicit_statement",
                "declaration",
                "declaration",
                "assignment_statement",
                "assignment_statement",
                "do_statement",
                "io_statement",
                "end_statement",
                "assignment_statement",
            ],
        ),
    ];

    #[test]
    fn valid_fixtures() {
        harness::check_valid("fortran", &FORTRAN, VALID);
    }

    #[test]
    fn malformed_keeps_the_root() {
        harness::check_malformed(
            "malformed",
            "fortran_broken.f",
            &FORTRAN,
            &[
                "program_statement",
                "implicit_statement",
                "declaration",
                "assignment_statement",
                "if_statement",
            ],
        );
    }

    #[test]
    #[ignore]
    fn dump_top_level_kinds() {
        harness::dump("fortran", &FORTRAN);
        harness::dump("malformed", &FORTRAN);
    }

    #[test]
    fn drops_the_fixed_form_prefix_and_joins_a_continuation() {
        use crate::deep::lexer::{lex, Tokens};
        let src = b"C comment line\n      PRINT *\n     1 'HI'\n";
        let mut tokens = Tokens::default();
        lex(src, &FORTRAN, &mut tokens);
        let kinds: Vec<&str> = (0..tokens.len())
            .map(|i| FORTRAN.name(tokens.kind[i]))
            .filter(|kind| !matches!(*kind, "whitespace"))
            .collect();
        assert_eq!(kinds, ["comment", "newline", "print", "*", "string"]);
    }
}
