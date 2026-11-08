//! Nim: `import`/`include`, `type`/`const`/`var`/`let` sections, `proc`,
//! `func`, `method`, `iterator`, `template`, `macro` and `converter`
//! definitions, the `object`/`enum`/`tuple` type bodies, and the statements and
//! expressions a security scanner looks at.
//!
//! Nim is indentation-scoped, so the grammar uses the runtime's layout pass
//! (`layout = NEWLINE INDENT DEDENT`, the same machinery Python uses): the
//! lexer emits one `NEWLINE` per physical line and the runtime infers `INDENT`
//! and `DEDENT` from the columns, so `suite: NEWLINE INDENT _statement* DEDENT`
//! is a block and an inline `if x: y` is `_expression NEWLINE`.
//!
//! # What is modelled
//!
//! - **Imports.** `import std/[a, b]`, `from os import getEnv`, `include`,
//!   `export`.
//! - **Declarations.** `type` with `object` (and `ref object of Base`),
//!   `enum`, `tuple`, `distinct` and aliases; `const`/`var`/`let` sections,
//!   inline or indented, with export `*` markers and types.
//! - **Routines.** `proc`, `func`, `method`, `iterator`, `template`, `macro`,
//!   `converter`: generic parameters, parameters with defaults, a return type,
//!   a pragma and an inline or indented body.
//! - **Statements.** `if`/`elif`/`else`, `when`, `case`/`of`/`else`, `while`,
//!   `for ... in`, `try`/`except`/`finally`, `block`, `defer`, `discard`,
//!   `raise`, `return`, `yield`, `break`, `continue`, and command calls.
//! - **Expressions.** One precedence table with Nim's word operators (`and`,
//!   `or`, `not`, `div`, `mod`, `shl`, `shr`, `xor`, `in`, `notin`, `is`,
//!   `isnot`, `of`) and its punctuation ones; calls, indexing and field access.
//! - **The lexer** knows `#` and nested `#[ ]#` comments, `"..."`, `"""..."""`
//!   and `r"..."`, characters, `0x`/`0b`/`0o` numbers with `_`, `{.pragma.}` as
//!   one token, and backtick-quoted identifiers.
//!
//! # What is missing, named here rather than accepted silently
//!
//! - **`if`/`case` as expressions.** `let x = if c: a else: b` is not an
//!   expression here; the assignment stops at `if`.
//! - **Generic instantiation in an expression** (`foo[int](1)`) reads the `[`
//!   as indexing, so the call is not a call node.
//! - **`do` blocks and `async` macros** are not modelled; `->`/`=>` are only
//!   tokens in a type position.
//! - **Pragma contents are opaque.** `{.inline, raises: [].}` is one token, so
//!   the pragma's name is not a node.
//! - **Operator definitions** (`proc `+`(a, b: T)`) parse only through the
//!   backtick identifier, not as a named routine.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { NIM {
    name = "nim";
    lex = lex;
    layout = NEWLINE INDENT DEDENT;

    tokens {
        NEWLINE = "newline",
        INDENT = "indent" open block,
        DEDENT = "dedent" close block,

        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,

        IDENT = "identifier",
        BACKTICK_IDENT = "operator_name",
        NUMBER = "number",
        STRING = "string",
        CHAR = "char",
        PRAGMA = "pragma",
        BAD_LITERAL = "bad_literal" bad,

        KW_ADDR = "addr",
        KW_AND = "and",
        KW_AS = "as",
        KW_ASM = "asm",
        KW_BIND = "bind",
        KW_BLOCK = "block",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CAST = "cast",
        KW_CONCEPT = "concept",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_CONVERTER = "converter",
        KW_DEFER = "defer",
        KW_DISCARD = "discard",
        KW_DISTINCT = "distinct",
        KW_DIV = "div",
        KW_DO = "do",
        KW_ELIF = "elif",
        KW_ELSE = "else",
        KW_END = "end",
        KW_ENUM = "enum",
        KW_EXCEPT = "except",
        KW_EXPORT = "export",
        KW_FINALLY = "finally",
        KW_FOR = "for",
        KW_FROM = "from",
        KW_FUNC = "func",
        KW_GENERIC = "generic",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_IN = "in",
        KW_INCLUDE = "include",
        KW_INTERFACE = "interface",
        KW_IS = "is",
        KW_ISNOT = "isnot",
        KW_ITERATOR = "iterator",
        KW_LET = "let",
        KW_MACRO = "macro",
        KW_METHOD = "method",
        KW_MIXIN = "mixin",
        KW_MOD = "mod",
        KW_NIL = "nil",
        KW_NOT = "not",
        KW_NOTIN = "notin",
        KW_OBJECT = "object",
        KW_OF = "of",
        KW_OR = "or",
        KW_OUT = "out",
        KW_PROC = "proc",
        KW_PTR = "ptr",
        KW_RAISE = "raise",
        KW_REF = "ref",
        KW_RETURN = "return",
        KW_SHL = "shl",
        KW_SHR = "shr",
        KW_STATIC = "static",
        KW_TEMPLATE = "template",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_TRY = "try",
        KW_TUPLE = "tuple",
        KW_TYPE = "type",
        KW_USING = "using",
        KW_VAR = "var",
        KW_WHEN = "when",
        KW_WHILE = "while",
        KW_WITH = "with",
        KW_XOR = "xor",
        KW_YIELD = "yield",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        DOTDOT = "..",
        COLON = ":",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        TILDE = "~",
        BANG = "!",
        DOLLAR = "$",
        AT = "@",
        QUESTION = "?",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        COLON_EQ = ":=",
        ARROW = "->",
        FATARROW = "=>",
    }

    program: _statement*,

    _statement:
        import_statement
        | from_statement
        | include_statement
        | export_statement
        | type_section
        | const_section
        | var_section
        | routine_definition
        | if_statement
        | when_statement
        | case_statement
        | while_statement
        | for_statement
        | try_statement
        | block_statement
        | defer_statement
        | discard_statement
        | raise_statement
        | return_statement
        | yield_statement
        | break_statement
        | continue_statement
        | &(_name _expression) command_statement
        | expression_statement
        | ";" ,

    // ---- imports ----

    import_statement: "import" import_target ("," import_target)* NEWLINE,
    export_statement: "export" import_target ("," import_target)* NEWLINE,
    include_statement: "include" import_target ("," import_target)* NEWLINE,
    import_target: [relative_prefix] _name ("/" (import_list | _name))* [import_list],
    relative_prefix: "." "/" | ".." "/",
    import_list: "[" (_name %? ",") "]",
    from_statement: "from" _name ("/" _name)* "import" (_name %? ",") NEWLINE,

    // ---- declarations ----

    type_section: "type" (NEWLINE type_block | type_declaration [NEWLINE]),
    type_block: INDENT (type_declaration [NEWLINE])+ DEDENT,
    type_declaration: _name ["*"] [generic_parameters] [PRAGMA] "=" type_body,
    type_body: object_type | enum_type | tuple_type | distinct_type | _type,
    object_type: ["ref"] "object" ["of" _type] [PRAGMA] NEWLINE object_block,
    object_block: INDENT (object_field NEWLINE)* DEDENT,
    object_field: _name ["*"] ("," _name ["*"])* [":" _type] ["=" _expression@assign],
    enum_type: "enum" [PRAGMA] NEWLINE enum_block,
    enum_block: INDENT (_enum_line NEWLINE)* DEDENT,
    _enum_line: enum_member ("," enum_member)* [","],
    enum_member: _name ["=" _expression@assign],
    tuple_type: "tuple" NEWLINE object_block,
    distinct_type: "distinct" _type [PRAGMA],

    const_section: "const" (NEWLINE value_block | value_declaration [NEWLINE]),
    var_section: ("var" | "let") (NEWLINE value_block | value_declaration [NEWLINE]),
    value_block: INDENT (value_declaration [NEWLINE])+ DEDENT,
    value_declaration: _name ["*"] ("," _name ["*"])* [":" _type] ["=" _expression@assign],

    routine_definition:
        routine_kind _name ["*"] [generic_parameters] [parameters] [":" _type] [PRAGMA] routine_body,
    routine_kind: "proc" | "func" | "method" | "iterator" | "template" | "macro" | "converter",
    generic_parameters: "[" (_name %? ",") "]",
    parameters: "(" (parameter %? ",") ")",
    parameter: ["var" | "out"] parameter_names [":" _type] ["=" _expression@assign],
    parameter_names: _name ("," _name)*,
    routine_body: "=" (inline_body | block_suite) | ":" term_suite,
    inline_body: _expression NEWLINE,

    // ---- statements ----

    if_statement: "if" _expression ":" suite elif_clause* [else_clause],
    elif_clause: "elif" _expression ":" suite,
    else_clause: "else" ":" suite,
    when_statement: "when" _expression ":" suite elif_clause* [else_clause],
    case_statement: "case" _expression [":"] NEWLINE case_body,
    case_body: case_indented | case_flat,
    case_indented: INDENT of_clause+ [else_clause] DEDENT,
    case_flat: of_clause+ [else_clause],
    of_clause: "of" (_expression % ",") ":" suite,
    while_statement: "while" _expression ":" suite,
    for_statement: "for" for_binding "in" _expression ":" suite,
    for_binding: _name ("," _name)*,
    try_statement: "try" ":" suite except_clause* [finally_clause],
    except_clause: "except" [(_type % ",")] ["as" _name] ":" suite,
    finally_clause: "finally" ":" suite,
    block_statement: "block" [IDENT] ":" suite,
    defer_statement: "defer" ":" suite,
    discard_statement: "discard" [_expression] NEWLINE,
    raise_statement: "raise" [_expression] NEWLINE,
    return_statement: "return" [_expression] NEWLINE,
    yield_statement: "yield" [_expression] NEWLINE,
    break_statement: "break" [IDENT] NEWLINE,
    continue_statement: "continue" NEWLINE,

    command_statement: command_call NEWLINE,
    command_call: _name (_expression % ","),
    expression_statement: _expression NEWLINE,

    suite: inline_suite | block_suite,
    inline_suite: inline_statement NEWLINE,
    inline_statement: &(_name _expression) command_call | _expression,
    block_suite: NEWLINE statement_block,
    term_suite: NEWLINE statement_block,
    statement_block: INDENT _statement+ DEDENT,

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "&=" | "|=" | "^=" => assignment,
        or_op: left "or" => binary_expression,
        and_op: left "and" => binary_expression,
        cmp: left "==" | "!=" | "<" | "<=" | ">" | ">=" | "in" | "notin" | "is" | "isnot" | "of" | "as" => comparison_expression,
        concat: left "&" => binary_expression,
        range: left ".." => range_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "div" | "mod" | "shl" | "shr" => binary_expression,
        xor_op: left "xor" => binary_expression,
        power: right "^" => binary_expression,
        unary: prefix "not" | "-" | "+" | "~" | "addr" | "ptr" | "ref" | "cast" | "@" | "$" => unary_expression,
        call: postfix arguments => call,
        index: postfix bracket_subscript => index_expression,
        field: postfix "." _field_name => field_expression,
    },

    _primary:
        _literal
        | _name
        | BACKTICK_IDENT
        | "nil"
        | "true"
        | "false"
        | parenthesized
        | array
        | table_constructor,
    _literal: NUMBER | STRING | CHAR | BAD_LITERAL,
    _name: IDENT,
    _field_name:
        IDENT | "type" | "end" | "object" | "tuple" | "enum" | "not" | "and" | "or"
        | "in" | "is" | "of" | "when" | "block" | "case" | "as" | "div" | "mod",
    parenthesized: "(" [(_expression %? ",")] ")",
    array: "[" [(_expression %? ",")] "]",
    table_constructor: "{" (pair_argument %? ",") "}",
    pair_argument: _expression@cmp ":" _expression@assign,
    arguments: "(" (argument %? ",") ")",
    argument: _name ":" _expression@assign | _expression@assign,
    bracket_subscript: "[" [(_expression %? ",")] "]",

    // ---- types ----

    _type: type_atom (type_tail)*,
    type_tail: "|" type_atom | "?" | type_atom,
    type_atom:
        _name
        | "nil"
        | "ptr" _type
        | "ref" _type
        | "var" _type
        | "static" _type
        | type_arguments
        | tuple_type_reference,
    type_arguments: "[" (type_argument %? ",") "]",
    type_argument: NUMBER | _type,
    tuple_type_reference: "tuple" bracket_arguments,
    bracket_arguments: "(" [(_type %? ",")] ")",
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\x0c' | 0x0b => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0c' | 0x0b));
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
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'#' if c.peek_at(1) == b'[' => {
            // A nesting doc/block comment: `#[ ... ]#`.
            lex::block_comment(c, b"#[", b"]#", true);
            COMMENT
        }
        b'#' => {
            lex::line_comment(c);
            COMMENT
        }
        b'{' if c.peek_at(1) == b'.' => pragma(c),
        b'"' => {
            // A triple-quoted string first, so the first two quotes are not
            // read as an empty string.
            if c.peek_at(1) == b'"' && c.peek_at(2) == b'"' {
                if triple_string(c) { STRING } else { BAD_LITERAL }
            } else if string(c) {
                STRING
            } else {
                BAD_LITERAL
            }
        }
        b'\'' => {
            c.bump();
            if lex::quoted(c, b'\'') { CHAR } else { BAD_LITERAL }
        }
        b'`' => backtick_ident(c),
        b'0'..=b'9' => {
            nim_number(c);
            NUMBER
        }
        b'.' if c.peek_at(1).is_ascii_digit() => {
            nim_number(c);
            NUMBER
        }
        byte if is_word_start(byte) => word(c),
        _ => operator(c),
    }
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// A word, or a raw string with the `r` prefix: `r"c:\temp"`.
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    if text == b"r" && c.peek() == b'"' {
        return if raw_string(c) { STRING } else { BAD_LITERAL };
    }
    keyword(text)
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"addr" => KW_ADDR,
        b"and" => KW_AND,
        b"as" => KW_AS,
        b"asm" => KW_ASM,
        b"bind" => KW_BIND,
        b"block" => KW_BLOCK,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"cast" => KW_CAST,
        b"concept" => KW_CONCEPT,
        b"const" => KW_CONST,
        b"continue" => KW_CONTINUE,
        b"converter" => KW_CONVERTER,
        b"defer" => KW_DEFER,
        b"discard" => KW_DISCARD,
        b"distinct" => KW_DISTINCT,
        b"div" => KW_DIV,
        b"do" => KW_DO,
        b"elif" => KW_ELIF,
        b"else" => KW_ELSE,
        b"end" => KW_END,
        b"enum" => KW_ENUM,
        b"except" => KW_EXCEPT,
        b"export" => KW_EXPORT,
        b"finally" => KW_FINALLY,
        b"for" => KW_FOR,
        b"from" => KW_FROM,
        b"func" => KW_FUNC,
        b"generic" => KW_GENERIC,
        b"if" => KW_IF,
        b"import" => KW_IMPORT,
        b"in" => KW_IN,
        b"include" => KW_INCLUDE,
        b"interface" => KW_INTERFACE,
        b"is" => KW_IS,
        b"isnot" => KW_ISNOT,
        b"iterator" => KW_ITERATOR,
        b"let" => KW_LET,
        b"macro" => KW_MACRO,
        b"method" => KW_METHOD,
        b"mixin" => KW_MIXIN,
        b"mod" => KW_MOD,
        b"nil" => KW_NIL,
        b"not" => KW_NOT,
        b"notin" => KW_NOTIN,
        b"object" => KW_OBJECT,
        b"of" => KW_OF,
        b"or" => KW_OR,
        b"out" => KW_OUT,
        b"proc" => KW_PROC,
        b"ptr" => KW_PTR,
        b"raise" => KW_RAISE,
        b"ref" => KW_REF,
        b"return" => KW_RETURN,
        b"shl" => KW_SHL,
        b"shr" => KW_SHR,
        b"static" => KW_STATIC,
        b"template" => KW_TEMPLATE,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        b"try" => KW_TRY,
        b"tuple" => KW_TUPLE,
        b"type" => KW_TYPE,
        b"using" => KW_USING,
        b"var" => KW_VAR,
        b"when" => KW_WHEN,
        b"while" => KW_WHILE,
        b"with" => KW_WITH,
        b"xor" => KW_XOR,
        b"yield" => KW_YIELD,
        _ => IDENT,
    }
}

/// `{. ... .}`: one token, through the closing `.}`. Its contents are not
/// parsed, which is what keeps `raises: []` from needing a grammar.
fn pragma(c: &mut Cursor) -> Kind {
    c.bump_n(2);
    while !c.at_end() {
        if c.starts_with(b".}") {
            c.bump_n(2);
            return PRAGMA;
        }
        c.bump_char();
    }
    BAD_LITERAL
}

/// A quoted string with escapes, on one line.
fn string(c: &mut Cursor) -> bool {
    c.bump();
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'\\' => c.bump_n(2),
            b'"' => {
                c.bump();
                return true;
            }
            b'\n' | b'\r' => return false,
            _ => c.bump_char(),
        }
    }
}

/// `"""..."""`: a string that spans lines, ending at three quotes.
fn triple_string(c: &mut Cursor) -> bool {
    c.bump_n(3);
    while !c.at_end() {
        if c.peek() == b'"' && c.peek_at(1) == b'"' && c.peek_at(2) == b'"' {
            c.bump_n(3);
            return true;
        }
        c.bump_char();
    }
    false
}

/// `r"..."`: a backslash is a plain byte.
fn raw_string(c: &mut Cursor) -> bool {
    c.bump();
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'"' => {
                c.bump();
                return true;
            }
            b'\n' | b'\r' => return false,
            _ => c.bump_char(),
        }
    }
}

/// A backtick identifier: `` `+` `` or `` `[]=` ``.
fn backtick_ident(c: &mut Cursor) -> Kind {
    c.bump();
    while !c.at_end() {
        match c.peek() {
            b'`' => {
                c.bump();
                return BACKTICK_IDENT;
            }
            b'\n' | b'\r' => return BAD_LITERAL,
            _ => c.bump_char(),
        }
    }
    BAD_LITERAL
}

/// A Nim number: decimal, `0x`/`0b`/`0o`, with `_`, a fraction, an exponent in
/// `e` or `'` form (`1e10`, `1.5e-3`, `2'f32`) and a `'`-suffixed type.
fn nim_number(c: &mut Cursor) {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return;
    }
    lex::number(c);
    // `'i32`, `'f64`, `'u8`: a type suffix after the number.
    if c.peek() == b'\'' && c.peek_at(1).is_ascii_alphabetic() {
        c.bump();
        c.eat_while(is_word_byte);
    }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'.', b'.', _) => (DOTDOT, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b':', b'=', _) => (COLON_EQ, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b'=', b'>', _) => (FATARROW, 2),
        (b'(', ..) => (LPAREN, 1),
        (b')', ..) => (RPAREN, 1),
        (b'[', ..) => (LBRACKET, 1),
        (b']', ..) => (RBRACKET, 1),
        (b'{', ..) => (LBRACE, 1),
        (b'}', ..) => (RBRACE, 1),
        (b';', ..) => (SEMI, 1),
        (b',', ..) => (COMMA, 1),
        (b'.', ..) => (DOT, 1),
        (b':', ..) => (COLON, 1),
        (b'=', ..) => (ASSIGN, 1),
        (b'<', ..) => (LT, 1),
        (b'>', ..) => (GT, 1),
        (b'+', ..) => (PLUS, 1),
        (b'-', ..) => (MINUS, 1),
        (b'*', ..) => (STAR, 1),
        (b'/', ..) => (SLASH, 1),
        (b'%', ..) => (PERCENT, 1),
        (b'&', ..) => (AMP, 1),
        (b'|', ..) => (PIPE, 1),
        (b'^', ..) => (CARET, 1),
        (b'~', ..) => (TILDE, 1),
        (b'!', ..) => (BANG, 1),
        (b'$', ..) => (DOLLAR, 1),
        (b'@', ..) => (AT, 1),
        (b'?', ..) => (QUESTION, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    c.bump_n(width);
    kind
}
