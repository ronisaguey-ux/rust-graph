//! COBOL, fixed columns: divisions, sections, paragraphs and sentences.
//!
//! A COBOL source line has the same five fields as fixed-form Fortran, with a
//! different meaning: columns 1-6 are the sequence number, column 7 is the
//! indicator (`*` or `/` for a comment line, `-` for a continuation), columns
//! 8-72 are the code, and columns 73-80 are ignored. The step lexer consumes
//! columns 1-7 at every line start and reports the line as `COMMENT`,
//! `WHITESPACE` (a continuation) or `NEWLINE`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Columns 73 and beyond are not dropped**, for the same reason as the
//!   fixed-form Fortran lexer: a step function cannot see the start of the line
//!   it stands in.
//! - **`PIC` is not validated.** `PIC X(10)` is a picture clause node holding
//!   the letter and its count; the editing characters (`Z`, `,`, `.`, `CR`) are
//!   not modelled.
//! - **The generic verb is not modelled.** Only the verbs named in
//!   `_statement_kind` are sentences; an unlisted verb at the start of a line is
//!   an error, not a silently accepted word soup.
//! - **`IF` covers the `END-IF` form and the inline form**, not the classic
//!   "scope terminator is the next period" form with nested sentences.
//! - **`COPY` and `REPLACE`** (the preprocessor) are not modelled.
//! - **A paragraph's body is flat**: `PERFORM MAIN-PARA` is a name reference,
//!   and the paragraph itself is a sibling sentence, not a block.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { COBOL {
    name = "cobol";
    lex = lex_cobol;

    tokens {
        NEWLINE = "newline",
        PERIOD = ".",
        COMMA = ",",
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        ASSIGN = "=",
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
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_IDENTIFICATION = "IDENTIFICATION",
        KW_PROGRAM_ID = "PROGRAM-ID",
        KW_ENVIRONMENT = "ENVIRONMENT",
        KW_DATA = "DATA",
        KW_PROCEDURE = "PROCEDURE",
        KW_DIVISION = "DIVISION",
        KW_SECTION = "SECTION",
        KW_WORKING_STORAGE = "WORKING-STORAGE",
        KW_LINKAGE = "LINKAGE",
        KW_FILE = "FILE",
        KW_CONTAINS = "CONTAINS",
        KW_CONFIGURATION = "CONFIGURATION",
        KW_FILLER = "FILLER",
        KW_IS = "IS",
        KW_AUTHOR = "AUTHOR",
        KW_DATE_WRITTEN = "DATE-WRITTEN",
        KW_MOVE = "MOVE",
        KW_TO = "TO",
        KW_DISPLAY = "DISPLAY",
        KW_PERFORM = "PERFORM",
        KW_THRU = "THRU",
        KW_THROUGH = "THROUGH",
        KW_UNTIL = "UNTIL",
        KW_TIMES = "TIMES",
        KW_ACCEPT = "ACCEPT",
        KW_CALL = "CALL",
        KW_USING = "USING",
        KW_RETURNING = "RETURNING",
        KW_COMPUTE = "COMPUTE",
        KW_ADD = "ADD",
        KW_SUBTRACT = "SUBTRACT",
        KW_MULTIPLY = "MULTIPLY",
        KW_DIVIDE = "DIVIDE",
        KW_GIVING = "GIVING",
        KW_FROM = "FROM",
        KW_BY = "BY",
        KW_INTO = "INTO",
        KW_IF = "IF",
        KW_THEN = "THEN",
        KW_ELSE = "ELSE",
        KW_END_IF = "END-IF",
        KW_END_PERFORM = "END-PERFORM",
        KW_STOP = "STOP",
        KW_RUN = "RUN",
        KW_GOBACK = "GOBACK",
        KW_EXIT = "EXIT",
        KW_CONTINUE = "CONTINUE",
        KW_OPEN = "OPEN",
        KW_CLOSE = "CLOSE",
        KW_READ = "READ",
        KW_WRITE = "WRITE",
        KW_SET = "SET",
        KW_INITIALIZE = "INITIALIZE",
        KW_PIC = "PIC",
        KW_PICTURE = "PICTURE",
        KW_VALUE = "VALUE",
        KW_VALUES = "VALUES",
        KW_ZERO = "ZERO",
        KW_ZEROS = "ZEROS",
        KW_SPACE = "SPACE",
        KW_SPACES = "SPACES",
        KW_NOT = "NOT",
        KW_AND = "AND",
        KW_OR = "OR",
    }

    program: _line*,

    _line: NEWLINE | _sentence,

    _sentence:
        division_header
        | section_header
        | paragraph_header
        | _statement,

    division_header: division_name "DIVISION" PERIOD [NEWLINE],
    division_name:
        "IDENTIFICATION" | "ENVIRONMENT" | "DATA" | "PROCEDURE",
    section_header: section_name "SECTION" PERIOD [NEWLINE],
    section_name:
        IDENT | "WORKING-STORAGE" | "LINKAGE" | "FILE" | "CONFIGURATION",
    paragraph_header: paragraph_name PERIOD [NEWLINE],
    paragraph_name: IDENT | "PROGRAM-ID",

    _statement: _statement_kind PERIOD [NEWLINE],

    _statement_kind:
        move_statement
        | display_statement
        | perform_statement
        | accept_statement
        | call_statement
        | compute_statement
        | arithmetic_statement
        | if_statement
        | stop_statement
        | goback_statement
        | continue_statement
        | io_statement
        | set_statement
        | initialize_statement
        | data_description,

    move_statement: "MOVE" _operand+ "TO" _operand+,
    display_statement: "DISPLAY" _operand+,
    perform_statement: "PERFORM" IDENT [perform_range] [perform_times],
    perform_range: ("THRU" | "THROUGH") IDENT,
    perform_times: IDENT "TIMES",
    accept_statement: "ACCEPT" _operand,
    call_statement: "CALL" _operand ["USING" _operand+],
    compute_statement: "COMPUTE" IDENT "=" _expression,
    arithmetic_statement:
        ("ADD" | "SUBTRACT") _operand+ ("TO" | "FROM") _operand+
        | "MULTIPLY" _operand+ "BY" _operand+
        | "DIVIDE" _operand+ "INTO" _operand+ ["GIVING" _operand+],
    if_statement: "IF" _expression ["THEN"] _if_item* [else_clause] ["END-IF"],
    else_clause: "ELSE" _if_item*,
    _if_item: NEWLINE | _statement_kind,
    stop_statement: "STOP" "RUN",
    goback_statement: "GOBACK",
    continue_statement: "CONTINUE" | "EXIT",
    io_statement:
        ("OPEN" | "CLOSE" | "READ" | "WRITE") _operand+,
    set_statement: "SET" _operand+ "TO" _operand+,
    initialize_statement: "INITIALIZE" _operand+,

    data_description: NUMBER data_name+ [picture_clause] [value_clause],
    data_name: IDENT | "FILLER",
    picture_clause: ("PIC" | "PICTURE") ["IS"] picture,
    picture: (IDENT | NUMBER) [picture_count],
    picture_count: "(" NUMBER ")",
    value_clause: ("VALUE" | "VALUES") ["IS"] _operand+,

    _operand: _expression,
    _expression: precedence _primary {
        right "=" => assignment,
        left "==" | "!=" | "<" | ">" | "<=" | ">=" => comparison,
        left "AND" | "OR" => logical,
        left "+" | "-" => additive,
        left "*" | "/" => multiplicative,
        prefix "-" | "NOT" => unary,
        postfix argument_list => call,
    },

    _argument: _expression,
    argument_list: "(" [_argument %? ","] ")",
    _primary:
        IDENT
        | NUMBER
        | STRING
        | "ZERO"
        | "ZEROS"
        | "SPACE"
        | "SPACES"
        | parenthesized_expression,
    parenthesized_expression: "(" _expression ")",
} }

fn at_line_start(c: &Cursor) -> bool {
    c.pos() == 0 || c.behind(1) == b'\n'
}

pub fn lex_cobol(c: &mut Cursor) -> Kind {
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
            WHITESPACE
        }
        b'"' | b'\'' => {
            let quote = c.peek();
            c.bump();
            loop {
                if c.at_end() || c.peek() == b'\n' {
                    return BAD_STRING;
                }
                if c.peek() == quote {
