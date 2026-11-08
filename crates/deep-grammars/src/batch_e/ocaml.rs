//! OCaml: structure items (modules, types, `let` bindings, `external`,
//! exceptions, classes), expressions with one precedence table, patterns and
//! types with a table each, and the lexer for its comments, strings, character
//! literals, type variables and numeric forms.
//!
//! The shape is Tree-sitter's OCaml where that costs nothing; the tree is not
//! the same tree.
//!
//! # What is modelled
//!
//! - **Structure items.** `open`, `include`, `module`/`module type` with a
//!   `struct ... end` or `sig ... end`, `type` with records, variants and `and`
//!   groups, `external`, `exception`, `class ... object ... end`, and `let`
//!   (with or without `rec`).
//! - **Expressions.** One precedence table from `:=` down to application;
//!   `let ... in`, `if`/`then`/`else`, `match`/`with`, `function`, `fun`,
//!   `try`/`with`, `while`/`do`/`done`, `for ... to ... do ... done`,
//!   `begin ... end`, `;` sequences, literals, lists, arrays, records.
//! - **Juxtaposition.** Application is a row of the precedence table whose
//!   "operator" is a simple atom. That is what makes `f x.y` parse as
//!   `f (x.y)` and not `(f x).y`: the right operand of that row is parsed at a
//!   tighter level, where the projection row still applies.
//! - **Patterns and types** have their own tables, so `Some x :: rest` and
//!   `int -> string list` come out as trees rather than flat token runs.
//!
//! # What is missing, named here rather than accepted silently
//!
//! - **`let` inside an expression is not a statement prefix.** `let x = 1 in
//!   ...` is an atom; a bare `let x = 1` after another expression on the same
//!   line is not, so `e; let x = 1 in ...` is only accepted inside `begin`,
//!   parentheses or a `let` body. OCaml's own rules are the same.
//! - **Object and functor syntax is partial.** `object ... end` members are
//!   modelled (`method`, `val`, `initializer`); `functor (X : S) -> ...`,
//!   `include struct ... end` and first-class modules are not.
//! - **Labels and optional arguments** (`~f`, `?f`) are `~`/`?` plus a name,
//!   not one token, so `~f:(fun x -> x)` is an application of three atoms.
//! - **`match` arms are not or-patterns.** `| A | B -> e` reads as two arms,
//!   the first of which has an empty body; the `|` separator is the arm
//!   delimiter.
//! - **Extension nodes** (`[%expr ...]`) and attributes (`[@warning ...]`) are
//!   not modelled.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { OCAML {
    name = "ocaml";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        ARRAY_OPEN = "[|" open arrays,
        ARRAY_CLOSE = "|]" close arrays,

        IDENT = "identifier",
        UIDENT = "value_name",
        TYPE_VAR = "type_variable",
        POLY_VARIANT = "polymorphic_variant",
        NUMBER = "number",
        STRING = "string",
        CHAR = "char",
        BAD_LITERAL = "bad_literal" bad,

        KW_AND = "and",
        KW_AS = "as",
        KW_ASSERT = "assert",
        KW_BEGIN = "begin",
        KW_CLASS = "class",
        KW_CONSTRAINT = "constraint",
        KW_DO = "do",
        KW_DONE = "done",
        KW_DOWNTO = "downto",
        KW_ELSE = "else",
        KW_END = "end",
        KW_EXCEPTION = "exception",
        KW_EXTERNAL = "external",
        KW_FALSE = "false",
        KW_FOR = "for",
        KW_FUN = "fun",
        KW_FUNCTION = "function",
        KW_FUNCTOR = "functor",
        KW_IF = "if",
        KW_IN = "in",
        KW_INCLUDE = "include",
        KW_INHERIT = "inherit",
        KW_INITIALIZER = "initializer",
        KW_LAZY = "lazy",
        KW_LET = "let",
        KW_LAND = "land",
        KW_LOR = "lor",
        KW_LXOR = "lxor",
        KW_MATCH = "match",
        KW_METHOD = "method",
        KW_MOD = "mod",
        KW_MODULE = "module",
        KW_MUTABLE = "mutable",
        KW_NEW = "new",
        KW_NONREC = "nonrec",
        KW_NOT = "not",
        KW_OBJECT = "object",
        KW_OF = "of",
        KW_OPEN = "open",
        KW_OR = "or",
        KW_PRIVATE = "private",
        KW_REC = "rec",
        KW_SIG = "sig",
        KW_STRUCT = "struct",
        KW_THEN = "then",
        KW_TO = "to",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPE = "type",
        KW_VAL = "val",
        KW_VIRTUAL = "virtual",
        KW_WHEN = "when",
        KW_WHILE = "while",
        KW_WITH = "with",

        SEMI = ";",
        SEMISEMI = ";;",
        COMMA = ",",
        DOT = ".",
        COLON = ":",
        COLON_EQ = ":=",
        COLONCOLON = "::",
        ARROW = "->",
        LARROW = "<-",
        BAR = "|",
        OROR = "||",
        AMP = "&",
        ANDAND = "&&",
        EQ = "=",
        NE = "<>",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        EQEQ = "==",
        BANGEQ = "!=",
        PLUS = "+",
        PLUSDOT = "+.",
        MINUS = "-",
        MINUSDOT = "-.",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        AT = "@",
        CARET = "^",
        BANG = "!",
        QUESTION = "?",
        TILDE = "~",
        HASH = "#",
        UNDERSCORE = "_",
        BACKTICK = "`",
        STARSTAR = "**",
    }

    program: _structure_item*,

    _structure_item:
        &(_expression) expression_statement
        | let_definition
        | open_statement
        | include_statement
        | module_definition
        | module_type_definition
        | type_definition
        | exception_definition
        | external_definition
        | class_definition
        | ";;",

    // ---- structure ----

    open_statement: "open" module_path,
    include_statement: "include" (module_path | signature),
    module_path: UIDENT ("." UIDENT)*,

    module_definition: "module" ["rec"] UIDENT [":" _type] "=" module_expression,
    module_expression: structure | module_path,
    structure: "struct" _structure_item* "end",
    module_type_definition: "module" "type" UIDENT "=" signature,
    signature: "sig" _signature_item* "end",
    _signature_item: val_specification | type_definition | module_specification,
    val_specification: "val" IDENT ":" _type,
    module_specification: "module" UIDENT ":" _type,

    type_definition: "type" ["nonrec"] type_declarator ("and" type_declarator)*,
    type_declarator: _type_parameters? IDENT ["=" _type],
    _type_parameters: TYPE_VAR+,
    record_type: "{" [_field_declaration %? ";"] "}",
    _field_declaration: ["mutable"] IDENT ":" _type,

    exception_definition: "exception" UIDENT ["of" _type],
    external_definition: "external" IDENT ":" _type "=" STRING STRING*,

    class_definition: "class" ["virtual"] IDENT [binding] [":" _type] "=" class_expression,
    class_expression: "object" [self_pattern] _class_member* "end",
    self_pattern: "(" IDENT ")",
    _class_member:
        method_definition
        | initializer_member
        | value_member
        | inherit_member,
    method_definition: ["private"] "method" ["virtual"] IDENT [binding] [":" _type] "=" sequence,
    initializer_member: "initializer" sequence,
    value_member: "val" ["mutable"] ["virtual"] IDENT [":" _type] ["=" sequence],
    inherit_member: "inherit" _expression,

    let_definition: "let" ["rec"] _let_binding ["=" sequence],
    _let_binding: binding [":" _type],
    binding: _pattern_atom+,

    expression_statement: _expression [";;"],

    // ---- expressions ----

    _expression: precedence _operand {
        assign: right ":=" | "<-" => assignment_expression,
        word_or: left "or" | "||" => binary_expression,
        word_and: left "&" | "&&" => binary_expression,
        compare: left "=" | "<>" | "<" | ">" | "<=" | ">=" | "==" | "!=" => comparison_expression,
        concat: right "@" | "^" => binary_expression,
        cons: right "::" => cons_expression,
        add: left "+" | "+." | "-" | "-." => binary_expression,
        mul: left "*" | "/" | "%" | "mod" | "land" | "lor" | "lxor" => binary_expression,
        power: right "**" => binary_expression,
        unary: prefix "not" | "-" | "-." | "!" | "~" => unary_expression,
        apply: postfix _application_argument => application_expression,
        index: postfix _dot_index => index_expression,
        field: postfix "." IDENT => field_expression,
    },

    _dot_index: "." (parenthesized_expression | list_expression),
    _application_argument: _simple_atom _postfix_tail*,
    _postfix_tail: "." IDENT | _dot_index,

    _operand: _let_atom | _simple_atom,
    _let_atom:
        let_expression
        | if_expression
        | match_expression
        | try_expression
        | function_expression
        | fun_expression
        | while_expression
        | for_expression
        | begin_expression
        | lazy_expression
        | assert_expression,

    _simple_atom:
        _name
        | UIDENT
        | POLY_VARIANT
        | NUMBER
        | STRING
        | CHAR
        | BAD_LITERAL
        | "false"
        | "true"
        | parenthesized_expression
        | list_expression
        | array_expression
        | record_expression,
    _name: IDENT | UNDERSCORE,

    let_expression: "let" ["rec"] _let_binding "=" sequence "in" sequence,
    if_expression: "if" sequence "then" sequence ["else" sequence],
    match_expression: "match" sequence "with" [BAR] match_arm (BAR match_arm)*,
    match_arm: _pattern ["when" sequence] "->" sequence,
    try_expression: "try" sequence "with" [BAR] match_arm (BAR match_arm)*,
    function_expression: "function" [BAR] match_arm (BAR match_arm)*,
    fun_expression: "fun" _pattern_atom+ "->" sequence,
    while_expression: "while" sequence "do" sequence "done",
    for_expression: "for" IDENT "=" sequence ("to" | "downto") sequence "do" sequence "done",
    begin_expression: "begin" sequence "end",
    lazy_expression: "lazy" _simple_atom,
    assert_expression: "assert" _simple_atom,

    sequence: _expression (";" _expression)*,

    parenthesized_expression: "(" [(sequence %? ",")] ")",
    list_expression: "[" [(sequence %? ";")] "]",
    array_expression: ARRAY_OPEN [(sequence %? ";")] ARRAY_CLOSE,
    record_expression: "{" [(_record_field %? ";")] "}",
    _record_field: &(IDENT "=") field_assign | field_pun,
    field_assign: IDENT "=" sequence,
    field_pun: IDENT,

    // ---- patterns ----

    _pattern: precedence _pattern_atom {
        cons: right "::" => cons_pattern,
        apply: postfix _pattern_atom => constructor_pattern,
    },
    _pattern_atom:
        IDENT
        | UIDENT
        | UNDERSCORE
        | NUMBER
        | STRING
        | CHAR
        | "true"
        | "false"
        | POLY_VARIANT
        | parenthesized_pattern
        | pattern_list
        | pattern_record
        | pattern_array,
    parenthesized_pattern: "(" [_pattern [":" _type]] ")",
    pattern_list: "[" [(_pattern_item %? ";")] "]",
    pattern_array: ARRAY_OPEN [(_pattern_item %? ";")] ARRAY_CLOSE,
    pattern_record: "{" [_pattern_field %? ";"] "}",
    _pattern_item: _pattern,
    _pattern_field: IDENT ["=" _pattern],

    // ---- types ----

    _type: precedence _type_atom {
        variant: left "|" => variant_type,
        payload: left "of" => constructor_declaration,
        arrow: right "->" => function_type,
        product: right "*" => tuple_type,
        constructor: postfix _type_atom => type_constructor,
    },
    _type_atom:
        type_path
        | IDENT
        | TYPE_VAR
        | UNDERSCORE
        | POLY_VARIANT
        | parenthesized_type
        | bracketed_type
        | record_type,
    type_path: UIDENT ("." (UIDENT | IDENT))*,
    parenthesized_type: "(" [_type] ")",
    bracketed_type: "[" (_type %? "|") "]",
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | 0x0b => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | 0x0b));
            WHITESPACE
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'(' if c.peek_at(1) == b'*' => {
            // A comment runs to `*)`, nesting like OCaml's own.
            lex::block_comment(c, b"(*", b"*)", true);
            COMMENT
        }
        b'"' => if string(c) { STRING } else { BAD_LITERAL },
        b'\'' => character_or_variable(c),
        b'`' => {
            c.bump();
            c.eat_while(is_word_byte);
            POLY_VARIANT
        }
        b'0'..=b'9' => {
            lex::number(c);
            NUMBER
        }
        b'a'..=b'z' => word(c),
        b'_' if !is_word_byte(c.peek_at(1)) => {
            c.bump();
            UNDERSCORE
        }
        b'A'..=b'Z' => uword(c),
        byte if byte == b'_' || byte >= 0x80 => word(c),
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

/// A lower-case identifier, which may carry primes (`x'`, `fold''`).
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    eat_primes(c);
    keyword(&start[..length])
}

/// An upper-case name: a constructor or a module.
fn uword(c: &mut Cursor) -> Kind {
    c.eat_while(is_word_byte);
    eat_primes(c);
    UIDENT
}

/// `'` is allowed inside an identifier and does not join two words, because it
/// needs the word byte before it.
fn eat_primes(c: &mut Cursor) {
    while c.peek() == b'\'' && is_word_byte(c.peek_at(1)) {
        c.bump_n(2);
        c.eat_while(is_word_byte);
    }
    if c.peek() == b'\'' && !matches!(c.peek_at(1), b'\'') && !is_word_start(c.peek_at(1)) {
        // `x'` with nothing after: the prime still belongs to the name.
        c.bump();
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"and" => KW_AND,
        b"as" => KW_AS,
        b"assert" => KW_ASSERT,
        b"begin" => KW_BEGIN,
        b"class" => KW_CLASS,
        b"constraint" => KW_CONSTRAINT,
        b"do" => KW_DO,
        b"done" => KW_DONE,
        b"downto" => KW_DOWNTO,
        b"else" => KW_ELSE,
        b"end" => KW_END,
        b"exception" => KW_EXCEPTION,
        b"external" => KW_EXTERNAL,
        b"false" => KW_FALSE,
        b"for" => KW_FOR,
        b"fun" => KW_FUN,
        b"function" => KW_FUNCTION,
        b"functor" => KW_FUNCTOR,
        b"if" => KW_IF,
        b"in" => KW_IN,
        b"include" => KW_INCLUDE,
        b"inherit" => KW_INHERIT,
        b"initializer" => KW_INITIALIZER,
        b"lazy" => KW_LAZY,
        b"let" => KW_LET,
        b"land" => KW_LAND,
        b"lor" => KW_LOR,
        b"lxor" => KW_LXOR,
        b"match" => KW_MATCH,
        b"method" => KW_METHOD,
        b"mod" => KW_MOD,
        b"module" => KW_MODULE,
        b"mutable" => KW_MUTABLE,
        b"new" => KW_NEW,
        b"nonrec" => KW_NONREC,
        b"object" => KW_OBJECT,
        b"of" => KW_OF,
        b"open" => KW_OPEN,
        b"or" => KW_OR,
        b"private" => KW_PRIVATE,
        b"rec" => KW_REC,
        b"sig" => KW_SIG,
        b"struct" => KW_STRUCT,
        b"then" => KW_THEN,
        b"to" => KW_TO,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"type" => KW_TYPE,
        b"val" => KW_VAL,
        b"virtual" => KW_VIRTUAL,
        b"when" => KW_WHEN,
        b"while" => KW_WHILE,
        b"with" => KW_WITH,
        _ => IDENT,
    }
}

/// A character literal, or a type variable when the quote is followed by a
/// name instead of a character and a closing quote.
fn character_or_variable(c: &mut Cursor) -> Kind {
    if c.peek_at(1) == b'\\' {
        c.bump_n(2);
        while !c.at_end() {
            match c.peek() {
                b'\'' => {
                    c.bump();
                    return CHAR;
                }
                b'\n' | b'\r' => return BAD_LITERAL,
                _ => c.bump_char(),
            }
        }
        return BAD_LITERAL;
    }
    if c.peek_at(2) == b'\'' && c.peek_at(1) != b'\'' {
        c.bump_n(3);
        return CHAR;
    }
    c.bump();
    c.eat_while(is_word_byte);
    TYPE_VAR
}

/// A string from its opening quote. Newlines are legal inside an OCaml
/// string, so only the end of the input can break one; a backslash escapes the
/// next byte, including a line break.
fn string(c: &mut Cursor) -> bool {
    c.bump();
    while !c.at_end() {
        match c.peek() {
            b'\\' => c.bump_n(2),
            b'"' => {
                c.bump();
                return true;
            }
            _ => c.bump_char(),
        }
    }
    false
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b';', b';', _) => (SEMISEMI, 2),
        (b':', b':', _) => (COLONCOLON, 2),
        (b':', b'=', _) => (COLON_EQ, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b'<', b'-', _) => (LARROW, 2),
        (b'<', b'>', _) => (NE, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'=', b'=', _) => (EQEQ, 2),
        (b'!', b'=', _) => (BANGEQ, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'*', b'*', _) => (STARSTAR, 2),
        (b'+', b'.', _) => (PLUSDOT, 2),
        (b'-', b'.', _) => (MINUSDOT, 2),
        (b'[', b'|', _) => (ARRAY_OPEN, 2),
        (b'|', b']', _) => (ARRAY_CLOSE, 2),
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
        (b'|', ..) => (BAR, 1),
        (b'&', ..) => (AMP, 1),
        (b'=', ..) => (EQ, 1),
        (b'<', ..) => (LT, 1),
        (b'>', ..) => (GT, 1),
        (b'+', ..) => (PLUS, 1),
        (b'-', ..) => (MINUS, 1),
        (b'*', ..) => (STAR, 1),
        (b'/', ..) => (SLASH, 1),
        (b'%', ..) => (PERCENT, 1),
        (b'@', ..) => (AT, 1),
        (b'^', ..) => (CARET, 1),
        (b'!', ..) => (BANG, 1),
        (b'?', ..) => (QUESTION, 1),
        (b'~', ..) => (TILDE, 1),
        (b'#', ..) => (HASH, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    c.bump_n(width);
    kind
}
