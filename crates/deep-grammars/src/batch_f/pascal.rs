//! Pascal (and the Object Pascal dialect Free Pascal accepts): a program or
//! unit file made of declarations followed by a `begin ... end.` main block.
//!
//! Pascal is case-insensitive and separated by `;`, so the lexer folds words to
//! upper case for keyword lookup and the grammar is the familiar
//! declaration/statement split. Both comment forms — `{ ... }` and `(* ... *)` —
//! and the Free Pascal `//` are modelled, as are `'...'` strings with `''`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Units are flat.** `interface` and `implementation` are section markers in
//!   the item list, not nodes with their own bodies, because where one section
//!   ends is the next marker, and the DSL has no "until a token of this set".
//! - **Nested routines** (a `procedure` inside another) parse as items of the
//!   enclosing body only when the body is a routine this grammar already
//!   recognises; a nested `begin ... end` inside a statement list is a block.
//! - **Generics, properties, class bodies and operator overloads** are not
//!   modelled: `class` is lexed as a name, so an Object Pascal class declaration
//!   falls into the `var`/`type` path and is reported as an error rather than
//!   half-parsed.
//! - **A `case` label range** (`1..10`) parses as two constants and `..`.
//! - **Assembler blocks** (`asm ... end`) are not modelled.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { PASCAL {
    name = "pascal";
    lex = lex_pascal;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        COMMA = ",",
        SEMI = ";",
        COLON = ":",
        DOT = ".",
        DOTDOT = "..",
        ASSIGN = ":=",
        EQ = "=",
        NE = "<>",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        AT = "@",
        CARET = "^",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_PROGRAM = "program",
        KW_UNIT = "unit",
        KW_INTERFACE = "interface",
        KW_IMPLEMENTATION = "implementation",
        KW_INITIALIZATION = "initialization",
        KW_FINALIZATION = "finalization",
        KW_USES = "uses",
        KW_CONST = "const",
        KW_TYPE = "type",
        KW_VAR = "var",
        KW_BEGIN = "begin",
        KW_END = "end",
        KW_IF = "if",
        KW_THEN = "then",
        KW_ELSE = "else",
        KW_WHILE = "while",
        KW_DO = "do",
        KW_FOR = "for",
        KW_TO = "to",
        KW_DOWNTO = "downto",
        KW_REPEAT = "repeat",
        KW_UNTIL = "until",
        KW_CASE = "case",
        KW_OF = "of",
        KW_SET = "set",
        KW_WITH = "with",
        KW_RECORD = "record",
        KW_ARRAY = "array",
        KW_PROCEDURE = "procedure",
        KW_FUNCTION = "function",
        KW_FORWARD = "forward",
        KW_OUT = "out",
        KW_IN = "in",
        KW_NIL = "nil",
        KW_AND = "and",
        KW_OR = "or",
        KW_NOT = "not",
        KW_DIV = "div",
        KW_MOD = "mod",
        KW_SHL = "shl",
        KW_SHR = "shr",
        KW_TRUE = "true",
        KW_FALSE = "false",
    }

    file: _item*,

    _item: _declaration | block | unit_end | ".",

    /// Everything a routine body may hold before its own `begin`. A `block` is
    /// deliberately absent: the repetition would swallow the routine's body.
    _declaration:
        program_header
        | unit_header
        | section_marker
        | uses_clause
        | const_section
        | type_section
        | var_section
        | routine_declaration,

    program_header: "program" _name [program_parameters] ";",
    program_parameters: "(" identifier_list ")",
    unit_header: "unit" dotted_name ";",
    section_marker: ("interface" | "implementation" | "initialization" | "finalization"),
    uses_clause: "uses" dotted_name ("," dotted_name)* ";",
    dotted_name: _name ("." _name)*,
    identifier_list: _name ("," _name)*,

    const_section: "const" const_declaration+,
    const_declaration: _name ["=" _expression] ";",
    type_section: "type" type_declaration+,
    type_declaration: _name "=" _type ";",
    var_section: "var" variable_declaration+,
    variable_declaration: identifier_list ":" _type ["=" _expression] ";",

    _type:
        "record" field_list "end"
        | array_type
        | enumeration_type
        | "set" "of" _type
        | "^" _type
        | _name,

    field_list: (identifier_list ":" _type ";")*,
    parameters: "(" [_parameter %? ";"] ")",
    _parameter: [("const" | "var" | "out")] identifier_list ":" _type,
    routine_declaration:
        ("procedure" | "function")
        _name
        [parameters]
        [":" _type]
        ";"
        _declaration*
        [body],

    body: "begin" _statements "end" ";" | "forward" ";",
    unit_end: "end" ".",

    block: "begin" _statements "end",
    /// Every statement ends with `;`, including the last before `end`: the
    /// separator is that uniform, and a repetition that may stop on a token it
    /// has already committed to would report the trailing one as an error.
    _statements: (_statement ";")*,
    _statement:
        block
        | if_statement
        | while_statement
        | for_statement
        | repeat_statement
        | case_statement
        | with_statement
        | expression_statement,

    if_statement: "if" _expression "then" _statement ["else" _statement],
    while_statement: "while" _expression "do" _statement,
    for_statement: "for" _name ":=" _expression ("to" | "downto") _expression "do" _statement,
    repeat_statement: "repeat" _statements "until" _expression,
    case_statement: "case" _expression "of" case_branch* ["else" _statements] "end",
    case_branch: case_label ("," case_label)* ":" _statement ";",
    case_label: _expression,
    with_statement: "with" _expression ("," _expression)* "do" _statement,
    expression_statement: _expression,

    _expression: precedence _primary {
        right ":=" => assignment,
        left "=" | "<>" | "<" | ">" | "<=" | ">=" | "in" => comparison,
        left "or" => logical,
        left "and" => logical,
        left "+" | "-" => additive,
        left "*" | "/" | "div" | "mod" | "shl" | "shr" => multiplicative,
        prefix "not" | "-" | "+" | "@" => unary,
        postfix index_expression => index,
        postfix call_arguments => call,
        postfix "." _name => member,
        postfix "^" => dereference,
    },

    _argument: _expression | _name ":" _expression,
    array_type: "array" array_indices "of" _type,
    array_indices: "[" _expression (".." _expression)? ("," _expression (".." _expression)?)* "]",
    enumeration_type: "(" _name ("," _name)* ")",
    index_expression: "[" _expression ("," _expression)* "]",
    call_arguments: "(" [_argument %? ","] ")",
    _primary:
        _name
        | NUMBER
        | STRING
        | "nil"
        | "true"
        | "false"
        | parenthesized_expression,
    _name: IDENT,
    parenthesized_expression: "(" _expression ")",
} }

pub fn lex_pascal(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'{' => {
            c.eat_while(|byte| byte != b'}');
            c.eat(b'}');
            COMMENT
        }
        b'(' if c.peek_at(1) == b'*' => {
            lex::block_comment(c, b"(*", b"*)", false);
            COMMENT
        }
        b'/' if c.peek_at(1) == b'/' => {
            lex::line_comment(c);
            COMMENT
        }
        b'\'' => {
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
        b'0'..=b'9' => {
            c.eat_while(|byte| byte.is_ascii_digit());
            if c.peek() == b'.' && c.peek_at(1) != b'.' {
                c.bump();
                c.eat_while(|byte| byte.is_ascii_digit());
            }
            if matches!(c.peek(), b'e' | b'E') {
                let sign = usize::from(matches!(c.peek_at(1), b'+' | b'-'));
                if c.peek_at(1 + sign).is_ascii_digit() {
                    c.bump_n(1 + sign);
                    c.eat_while(|byte| byte.is_ascii_digit());
                }
            }
            NUMBER
        }
        b'#' => {
            c.bump();
            c.eat_while(|byte| byte.is_ascii_digit());
            NUMBER
        }
        b'$' => {
            c.bump();
            c.eat_while(|byte| byte.is_ascii_hexdigit());
            NUMBER
        }
        b'&' => {
            c.bump();
            c.eat_while(|byte| byte.is_ascii_digit());
            NUMBER
        }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
            keyword(&lex::upper(&start[..length]))
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b':' if c.peek_at(1) == b'=' => { c.bump_n(2); ASSIGN }
        b':' => { c.bump(); COLON }
        b'.' if c.peek_at(1) == b'.' => { c.bump_n(2); DOTDOT }
        b'.' => { c.bump(); DOT }
        b'=' => { c.bump(); EQ }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'>' => { c.bump_n(2); NE }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' => { c.bump(); GT }
        b'+' => { c.bump(); PLUS }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' => { c.bump(); SLASH }
        b'@' => { c.bump(); AT }
        b'^' => { c.bump(); CARET }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"PROGRAM" => KW_PROGRAM,
        b"UNIT" => KW_UNIT,
        b"INTERFACE" => KW_INTERFACE,
        b"IMPLEMENTATION" => KW_IMPLEMENTATION,
        b"INITIALIZATION" => KW_INITIALIZATION,
        b"FINALIZATION" => KW_FINALIZATION,
        b"USES" => KW_USES,
        b"CONST" => KW_CONST,
        b"TYPE" => KW_TYPE,
        b"VAR" => KW_VAR,
        b"BEGIN" => KW_BEGIN,
        b"END" => KW_END,
        b"IF" => KW_IF,
        b"THEN" => KW_THEN,
        b"ELSE" => KW_ELSE,
        b"WHILE" => KW_WHILE,
        b"DO" => KW_DO,
        b"FOR" => KW_FOR,
        b"TO" => KW_TO,
        b"DOWNTO" => KW_DOWNTO,
        b"REPEAT" => KW_REPEAT,
        b"UNTIL" => KW_UNTIL,
        b"CASE" => KW_CASE,
        b"OF" => KW_OF,
        b"WITH" => KW_WITH,
        b"RECORD" => KW_RECORD,
        b"ARRAY" => KW_ARRAY,
        b"PROCEDURE" => KW_PROCEDURE,
        b"FUNCTION" => KW_FUNCTION,
        b"FORWARD" => KW_FORWARD,
        b"IN" => KW_IN,
        b"NIL" => KW_NIL,
        b"AND" => KW_AND,
        b"OR" => KW_OR,
        b"NOT" => KW_NOT,
        b"DIV" => KW_DIV,
        b"MOD" => KW_MOD,
        b"SHL" => KW_SHL,
        b"SHR" => KW_SHR,
        b"TRUE" => KW_TRUE,
        b"FALSE" => KW_FALSE,
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_f::harness;

    const VALID: &[(&str, &[&str])] = &[
        (
            "program.pas",
            &[
                "program_header",
                "uses_clause",
                "const_section",
                "var_section",
                "routine_declaration",
                "routine_declaration",
                "block",
            ],
        ),
        (
            "shapes.pas",
            &[
                "program_header",
                "uses_clause",
                "type_section",
                "var_section",
                "routine_declaration",
                "routine_declaration",
                "block",
            ],
        ),
        (
            "unit.pas",
            &[
                "unit_header",
                "section_marker",
                "uses_clause",
                "type_section",
                "var_section",
                "routine_declaration",
                "unit_end",
            ],
        ),
    ];

    #[test]
    fn valid_fixtures() {
        harness::check_valid("pascal", &PASCAL, VALID);
    }

    #[test]
    fn malformed_keeps_the_root() {
        harness::check_malformed(
            "malformed",
            "pascal_broken.pas",
            &PASCAL,
            &["program_header", "var_section", "block"],
        );
    }

    #[test]
    #[ignore]
    fn dump_top_level_kinds() {
        harness::dump("pascal", &PASCAL);
        harness::dump("malformed", &PASCAL);
    }

    #[test]
    fn lexes_a_case_insensitive_keyword_and_a_string() {
        use crate::deep::lexer::{lex, Tokens};
        let src = b"BeGiN s := 'it''s'; EnD";
        let mut tokens = Tokens::default();
        lex(src, &PASCAL, &mut tokens);
        let kinds: Vec<&str> = (0..tokens.len())
            .map(|i| PASCAL.name(tokens.kind[i]))
            .filter(|kind| *kind != "whitespace")
            .collect();
        assert_eq!(kinds, ["begin", "identifier", ":=", "string", ";", "end"]);
    }
}
