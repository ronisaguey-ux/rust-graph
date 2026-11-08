//! Go through `grammar!`: Slice 4. Node names follow Tree-sitter's Go where
//! that costs nothing; the tree is not the same tree.
//!
//! Choices that make Go LL(3):
//!
//! - Semicolons are inserted by the lexer exactly as the Go specification
//!   says: a line break after an identifier, a literal, `break`, `continue`,
//!   `fallthrough`, `return`, `++`, `--`, `)`, `]` or `}` is a `;` token (its
//!   text the whitespace), so statements and declarations are lists over `;`.
//! - Header expressions (`if`, `for`, `switch`) come from `_expression_nb`,
//!   where a `{` opens the body. A composite literal is taken there only after
//!   a literal type (`range []int{1, 2} {`), never after a name, as in Go.
//! - What only a longer look tells apart is decided by predicates: a simple
//!   statement before `;` in a header, a `range` clause, a label, a named
//!   parameter or field (`a []int` against the type `[]int`), and a type
//!   parameter list (`type A[T any] ...` against `type A [N]int`).
//! - Simple statements are one table over an expression list, so `a, b := f()`
//!   reads the list once and becomes a `short_var_declaration` at the `:=`.

use super::lexer::Cursor;
use super::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { GO {
    name = "go";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        INT = "int_literal",
        FLOAT = "float_literal",
        IMAGINARY = "imaginary_literal",
        RUNE = "rune_literal",
        STRING = "interpreted_string_literal",
        RAW_STRING = "raw_string_literal",
        /// A string, rune or raw string with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CHAN = "chan",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DEFAULT = "default",
        KW_DEFER = "defer",
        KW_ELSE = "else",
        KW_FALLTHROUGH = "fallthrough",
        KW_FOR = "for",
        KW_FUNC = "func",
        KW_GO = "go",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_INTERFACE = "interface",
        KW_MAP = "map",
        KW_PACKAGE = "package",
        KW_RANGE = "range",
        KW_RETURN = "return",
        KW_SELECT = "select",
        KW_STRUCT = "struct",
        KW_SWITCH = "switch",
        KW_TYPE = "type",
        KW_VAR = "var",

        /// Written, or a line break where Go inserts one.
        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        ELLIPSIS = "...",
        COLON = ":",
        DEFINE = ":=",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        BANG = "!",
        TILDE = "~",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        AND_NOT = "&^",
        SHL = "<<",
        SHR = ">>",
        ANDAND = "&&",
        OROR = "||",
        ARROW = "<-",
        INCREMENT = "++",
        DECREMENT = "--",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        AND_NOT_EQ = "&^=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
    }

    source_file: (_top_level %? ";"),
    _top_level:
        package_clause
        | import_declaration
        | &("func" _name) function_declaration
        | method_declaration
        | _declaration,

    package_clause: "package" _name,
    import_declaration: "import" (import_spec | import_spec_list),
    import_spec_list: "(" (import_spec %? ";") ")",
    import_spec: [_name | "."] _string,
    _string: STRING | RAW_STRING,

    // ---- declarations ----

    _declaration: const_declaration | var_declaration | type_declaration,
    const_declaration: "const" (const_spec | const_spec_list),
    const_spec_list: "(" (const_spec %? ";") ")",
    const_spec: _name ("," _name)* [_type] ["=" _expression_list],
    var_declaration: "var" (var_spec | var_spec_list),
    var_spec_list: "(" (var_spec %? ";") ")",
    var_spec: _name ("," _name)* (_type ["=" _expression_list] | "=" _expression_list),
    type_declaration: "type" (type_spec | type_spec_list),
    type_spec_list: "(" (type_spec %? ";") ")",
    type_spec: _name _type_spec_rest,
    _type_spec_rest: &type_parameter_list type_parameter_list ["="] _type | ["="] _type,

    function_declaration: "func" _name [type_parameter_list] parameter_list [_result] [block],
    method_declaration: "func" parameter_list _name parameter_list [_result] [block],
    type_parameter_list: "[" (type_parameter_declaration %? ",") "]",
    type_parameter_declaration: _name ("," _name)* _constraint,
    _constraint: _constraint_term ("|" _constraint_term)*,
    _constraint_term: "~" _type | _type,
    parameter_list: "(" (_parameter %? ",") ")",
    _parameter:
        &(_name ["..."] _type) parameter_declaration
        | variadic_parameter_declaration
        | _type,
    parameter_declaration: _name ["..."] _type,
    variadic_parameter_declaration: "..." _type,
    _result: parameter_list | _type_np,

    // ---- types ----

    _type: _type_np | parenthesized_type,
    _type_np:
        _type_path
        | pointer_type
        | array_type
        | map_type
        | channel_type
        | receive_channel_type
        | function_type
        | struct_type
        | interface_type,
    parenthesized_type: "(" _type ")",
    _type_path: precedence _name {
        postfix "." _name => qualified_type,
        postfix type_arguments => generic_type,
    },
    type_arguments: "[" (_type %? ",") "]",
    pointer_type: "*" _type,
    array_type: array_length _type,
    array_length: "[" [_expression | "..."] "]",
    map_type: "map" map_key _type,
    map_key: "[" _type "]",
    channel_type: "chan" ["<-"] _type,
    /// `<-chan T`. Not an expression's operand: there `<-` receives, and the
    /// operand is `chan T`.
    receive_channel_type: "<-" "chan" _type,
    function_type: "func" parameter_list [_result],
    struct_type: "struct" field_declaration_list,
    field_declaration_list: "{" (field_declaration %? ";") "}",
    field_declaration: _field_head [_string],
    _field_head: &(_name ("," _name)* _type) _name ("," _name)* _type | _type,
    interface_type: "interface" interface_body,
    interface_body: "{" (_interface_element %? ";") "}",
    _interface_element: &(_name parameter_list) method_elem | type_elem,
    method_elem: _name parameter_list [_result],
    type_elem: _constraint,

    // ---- statements ----

    block: "{" (_statement %? ";") "}",
    _statement:
        _declaration
        | &(_name ":") labeled_statement
        | _simple
        | go_statement
        | defer_statement
        | return_statement
        | break_statement
        | continue_statement
        | goto_statement
        | fallthrough_statement
        | block
        | if_statement
        | for_statement
        | switch_statement
        | select_statement,
    labeled_statement: _name ":" [_statement],
    go_statement: "go" _expression,
    defer_statement: "defer" _expression,
    return_statement: "return" [_expression_list],
    break_statement: "break" [_name],
    continue_statement: "continue" [_name],
    goto_statement: "goto" _name,
    fallthrough_statement: "fallthrough",

    _simple: precedence _expression_list {
        postfix ":=" _expression_list => short_var_declaration,
        postfix ("=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "&^=" | "<<=" | ">>=") _expression_list => assignment_statement,
        postfix "++" => inc_statement,
        postfix "--" => dec_statement,
        postfix "<-" _expression => send_statement,
    },
    _simple_nb: precedence _expression_list_nb {
        postfix ":=" _expression_list_nb => short_var_declaration,
        postfix ("=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "&^=" | "<<=" | ">>=") _expression_list_nb => assignment_statement,
        postfix "++" => inc_statement,
        postfix "--" => dec_statement,
        postfix "<-" _expression_nb => send_statement,
    },
    _expression_list: _expression ("," _expression)*,
    _expression_list_nb: _expression_nb ("," _expression_nb)*,

    if_statement: "if" _if_header block ["else" (if_statement | block)],
    _if_header: &(_simple_nb ";") _simple_nb ";" _expression_nb | _expression_nb,
    for_statement: "for" [_for_header] block,
    _for_header:
        &([_expression_list_nb ("=" | ":=")] "range") range_clause
        | &([_simple_nb] ";") for_clause
        | _expression_nb,
    range_clause: [_expression_list_nb ("=" | ":=")] "range" _expression_nb,
    for_clause: [_simple_nb] ";" [_expression_nb] ";" [_simple_nb],
    switch_statement: "switch" [_switch_header] switch_body,
    _switch_header: &([_simple_nb] ";") [_simple_nb] ";" [_simple_nb] | _simple_nb,
    switch_body: "{" (expression_case | default_case)* "}",
    expression_case: "case" _expression_list ":" (_statement %? ";"),
    default_case: "default" ":" (_statement %? ";"),
    select_statement: "select" select_body,
    select_body: "{" (communication_case | default_case)* "}",
    communication_case: "case" _simple ":" (_statement %? ";"),

    // ---- expressions ----

    _expression: precedence _primary {
        or: left "||" => binary_expression,
        and: left "&&" => binary_expression,
        cmp: left "==" | "!=" | "<" | "<=" | ">" | ">=" => binary_expression,
        add: left "+" | "-" | "|" | "^" => binary_expression,
        mul: left "*" | "/" | "%" | "<<" | ">>" | "&" | "&^" => binary_expression,
        prefix "+" | "-" | "!" | "^" | "*" | "&" | "<-" => unary_expression,
        postfix arguments => call_expression,
        postfix index => index_expression,
        postfix "." _name => selector_expression,
        postfix "." asserted_type => type_assertion_expression,
        postfix literal_value => composite_literal,
    },
    /// `_expression` in a header, where a `{` opens the body.
    _expression_nb: precedence _primary_nb {
        or_nb: left "||" => binary_expression,
        and_nb: left "&&" => binary_expression,
        cmp_nb: left "==" | "!=" | "<" | "<=" | ">" | ">=" => binary_expression,
        add_nb: left "+" | "-" | "|" | "^" => binary_expression,
        mul_nb: left "*" | "/" | "%" | "<<" | ">>" | "&" | "&^" => binary_expression,
        prefix "+" | "-" | "!" | "^" | "*" | "&" | "<-" => unary_expression,
        postfix arguments => call_expression,
        postfix index => index_expression,
        postfix "." _name => selector_expression,
        postfix "." asserted_type => type_assertion_expression,
    },
    /// A header's operands: a composite literal only after a literal type, as
    /// in `range []int{1, 2} {`; after a name, `{` opens the body.
    _primary_nb:
        _name
        | INT
        | FLOAT
        | IMAGINARY
        | RUNE
        | _string
        | func_literal
        | parenthesized_expression
        | _typed_literal
        | channel_type
        | interface_type,
    _typed_literal: precedence _literal_type {
        once literal_value => composite_literal,
    },
    _literal_type: array_type | map_type | struct_type,
    _primary:
        _name
        | INT
        | FLOAT
        | IMAGINARY
        | RUNE
        | _string
        | func_literal
        | parenthesized_expression
        | array_type
        | map_type
        | channel_type
        | struct_type
        | interface_type,
    func_literal: "func" parameter_list [_result] [block],
    parenthesized_expression: "(" _expression ")",
    arguments: "(" (_argument %? ",") ")",
    _argument: _expression ["..."],
    index: "[" (_expression %? ",") (":" [_expression])* "]",
    asserted_type: "(" (_type | "type") ")",
    literal_value: "{" (_element %? ",") "}",
    _element: precedence _element_value {
        postfix ":" _element_value => keyed_element,
    },
    _element_value: _expression | literal_value,

    _name: IDENT,
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' => {
            let mut line_break = false;
            c.eat_while(|byte| {
                line_break |= byte == b'\n';
                matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
            });
            if line_break && ends_line(c.previous()) { SEMI } else { WHITESPACE }
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            // A general comment with a line break acts like a line break.
            let start = c.pos();
            c.bump_n(2);
            while !c.at_end() && !c.starts_with(b"*/") {
                c.bump();
            }
            c.bump_n(2);
            if c.since(start).contains(&b'\n') && ends_line(c.previous()) { SEMI } else { COMMENT }
        }
        b'"' => {
            c.bump();
            if c.quoted(b'"', true) { STRING } else { BAD_LITERAL }
        }
        b'\'' => {
            c.bump();
            if c.quoted(b'\'', true) { RUNE } else { BAD_LITERAL }
        }
        b'`' => {
            c.bump();
            c.eat_while(|byte| byte != b'`');
            if c.eat(b'`') { RAW_STRING } else { BAD_LITERAL }
        }
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() => number(c),
        byte if is_word_start(byte) => {
            let start = c.pos();
            c.eat_while(is_word_byte);
            keyword(c.since(start))
        }
        _ => operator(c),
    }
}

/// Whether a line break after a token of this kind inserts a semicolon.
fn ends_line(kind: Kind) -> bool {
    matches!(
        kind,
        IDENT | INT | FLOAT | IMAGINARY | RUNE | STRING | RAW_STRING | BAD_LITERAL | KW_BREAK | KW_CONTINUE | KW_FALLTHROUGH | KW_RETURN | INCREMENT | DECREMENT | RPAREN | RBRACKET | RBRACE
    )
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"chan" => KW_CHAN,
        b"const" => KW_CONST,
        b"continue" => KW_CONTINUE,
        b"default" => KW_DEFAULT,
        b"defer" => KW_DEFER,
        b"else" => KW_ELSE,
        b"fallthrough" => KW_FALLTHROUGH,
        b"for" => KW_FOR,
        b"func" => KW_FUNC,
        b"go" => KW_GO,
        b"goto" => KW_GOTO,
        b"if" => KW_IF,
        b"import" => KW_IMPORT,
        b"interface" => KW_INTERFACE,
        b"map" => KW_MAP,
        b"package" => KW_PACKAGE,
        b"range" => KW_RANGE,
        b"return" => KW_RETURN,
        b"select" => KW_SELECT,
        b"struct" => KW_STRUCT,
        b"switch" => KW_SWITCH,
        b"type" => KW_TYPE,
        b"var" => KW_VAR,
        _ => IDENT,
    }
}

/// An integer, float or imaginary literal, with `_` separators and the
/// `0x`, `0o` and `0b` prefixes; hexadecimal floats take a `p` exponent.
fn number(c: &mut Cursor) -> Kind {
    let mut kind = INT;
    let hex = c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X');
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        c.bump_n(2);
    }
    let digit = |byte: u8| byte.is_ascii_hexdigit() || byte == b'_';
    let decimal = |byte: u8| byte.is_ascii_digit() || byte == b'_';
    if hex {
        c.eat_while(digit);
    } else {
        c.eat_while(decimal);
    }
    if c.peek() == b'.' {
        kind = FLOAT;
        c.bump();
        if hex {
            c.eat_while(digit);
        } else {
            c.eat_while(decimal);
        }
    }
    let exponent = if hex { matches!(c.peek(), b'p' | b'P') } else { matches!(c.peek(), b'e' | b'E') };
    if exponent && (c.peek_at(1).is_ascii_digit() || (matches!(c.peek_at(1), b'+' | b'-') && c.peek_at(2).is_ascii_digit())) {
        kind = FLOAT;
        c.bump_n(2);
        c.eat_while(decimal);
    }
    if c.peek() == b'i' {
        c.bump();
        kind = IMAGINARY;
    }
    kind
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'(', _, _) => (LPAREN, 1),
        (b')', _, _) => (RPAREN, 1),
        (b'[', _, _) => (LBRACKET, 1),
        (b']', _, _) => (RBRACKET, 1),
        (b'{', _, _) => (LBRACE, 1),
        (b'}', _, _) => (RBRACE, 1),
        (b';', _, _) => (SEMI, 1),
        (b',', _, _) => (COMMA, 1),
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'.', _, _) => (DOT, 1),
        (b':', b'=', _) => (DEFINE, 2),
        (b':', _, _) => (COLON, 1),
        (b'=', b'=', _) => (EQ, 2),
        (b'=', _, _) => (ASSIGN, 1),
        (b'!', b'=', _) => (NE, 2),
        (b'!', _, _) => (BANG, 1),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'<', b'<', _) => (SHL, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'<', b'-', _) => (ARROW, 2),
        (b'<', _, _) => (LT, 1),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'>', b'>', _) => (SHR, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'>', _, _) => (GT, 1),
        (b'~', _, _) => (TILDE, 1),
        (b'+', b'+', _) => (INCREMENT, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'+', _, _) => (PLUS, 1),
        (b'-', b'-', _) => (DECREMENT, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'-', _, _) => (MINUS, 1),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'*', _, _) => (STAR, 1),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'/', _, _) => (SLASH, 1),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'%', _, _) => (PERCENT, 1),
        (b'&', b'^', b'=') => (AND_NOT_EQ, 3),
        (b'&', b'^', _) => (AND_NOT, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'&', _, _) => (AMP, 1),
        (b'|', b'|', _) => (OROR, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'|', _, _) => (PIPE, 1),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'^', _, _) => (CARET, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    c.bump_n(width);
    kind
}

#[cfg(test)]
mod tests {
    use super::super::budget::Budget;
    use super::super::parser::Workspace;
    use super::super::tests::{check, mutate, on_big_stack, Rng};
    use super::super::tree::HAS_ERROR;
    use super::*;

    fn parse(src: &str) -> Workspace {
        let mut workspace = Workspace::default();
        let outcome = workspace.parse(src.as_bytes(), &GO, &Budget::default()).unwrap();
        assert!(outcome.complete(), "{src:?}: {}", outcome.describe());
        check(&workspace, src.as_bytes(), &GO);
        workspace
    }

    fn named(src: &str) -> String {
        parse(src).tree.sexp(&GO, true)
    }

    fn clean(src: &str) -> bool {
        parse(src).tree.root().flags & HAS_ERROR == 0
    }

    /// The expression in `var _ = SRC`.
    fn expression(src: &str) -> String {
        let tree = named(&format!("package p\nvar _ = {src}\n"));
        let prefix = "(source_file (package_clause package identifier) (var_declaration var (var_spec identifier ";
        tree.strip_prefix(prefix).and_then(|rest| rest.strip_suffix(")))")).unwrap_or_else(|| panic!("{src}: {tree}")).to_string()
    }

    #[test]
    fn an_error_stays_inside_its_construct() {
        // A missing value is an error inside its statement; the next statement
        // and the next function survive whole.
        assert_eq!(
            named("package p\nfunc f() { x := ; y() }\nfunc g() {}\n"),
            "(source_file (package_clause package identifier) (function_declaration func identifier (parameter_list) (block (short_var_declaration identifier (ERROR MISSING)) (call_expression identifier (arguments)))) (function_declaration func identifier (parameter_list) (block)))"
        );
        // An unclosed parenthesis costs its own `if`, not the next function.
        let tree = named("package p\nfunc f() { if (x { y() } }\nfunc g() {}\n");
        assert!(tree.contains("(parenthesized_expression UNCLOSED"), "{tree}");
        assert!(tree.ends_with("(function_declaration func identifier (parameter_list) (block)))"), "{tree}");
        // A bad token inside an argument list stays in that list.
        assert_eq!(
            named("package p\nvar x = f(1, $, 2)\nvar y = 2\n"),
            "(source_file (package_clause package identifier) (var_declaration var (var_spec identifier (call_expression identifier (arguments int_literal (ERROR UNKNOWN) int_literal)))) (var_declaration var (var_spec identifier int_literal)))"
        );
    }

    #[test]
    fn operators_nest_by_precedence() {
        let cases = [
            ("a + b * c", "(binary_expression identifier (binary_expression identifier identifier))"),
            ("a * b + c", "(binary_expression (binary_expression identifier identifier) identifier)"),
            ("a || b && c == d", "(binary_expression identifier (binary_expression identifier (binary_expression identifier identifier)))"),
            ("a << 1 | b", "(binary_expression (binary_expression identifier int_literal) identifier)"),
            ("a &^ b", "(binary_expression identifier identifier)"),
            ("-x.y", "(unary_expression (selector_expression identifier identifier))"),
            ("*p.f()", "(unary_expression (call_expression (selector_expression identifier identifier) (arguments)))"),
            ("<-ch", "(unary_expression identifier)"),
            ("x.(T).m", "(selector_expression (type_assertion_expression identifier (asserted_type identifier)) identifier)"),
            ("f[int](x)", "(call_expression (index_expression identifier (index identifier)) (arguments identifier))"),
            ("s[a:b:c]", "(index_expression identifier (index identifier identifier identifier))"),
            ("T{1, k: v}", "(composite_literal identifier (literal_value int_literal (keyed_element identifier identifier)))"),
            ("&T{}", "(unary_expression (composite_literal identifier (literal_value)))"),
            ("func(x int) int { return x }(1)", "(call_expression (func_literal func (parameter_list (parameter_declaration identifier identifier)) identifier (block (return_statement return identifier))) (arguments int_literal))"),
            ("[]int{1}[0]", "(index_expression (composite_literal (array_type (array_length) identifier) (literal_value int_literal)) (index int_literal))"),
            ("map[string]int{\"a\": 1}", "(composite_literal (map_type map (map_key identifier) identifier) (literal_value (keyed_element interpreted_string_literal int_literal)))"),
        ];
        for (src, want) in cases {
            assert_eq!(expression(src), want, "{src:?}");
        }
    }

    #[test]
    fn declarations_types_and_generics() {
        let tree = named("package p\ntype Pair[K comparable, V any] struct {\n\tKey K\n\tbuf [64]byte `json:\"b\"`\n\tsync.Mutex\n}\ntype A [N]int\n");
        assert!(tree.contains("(type_spec identifier (type_parameter_list (type_parameter_declaration identifier identifier) (type_parameter_declaration identifier identifier)) (struct_type struct (field_declaration_list (field_declaration identifier identifier) (field_declaration identifier (array_type (array_length int_literal) identifier) raw_string_literal) (field_declaration (qualified_type identifier identifier)))))"), "{tree}");
        assert!(tree.contains("(type_spec identifier (array_type (array_length identifier) identifier))"), "`type A [N]int` is an array: {tree}");
        // `a, b int`: the first name reads as a type, the second as a parameter.
        let tree = named("package p\nfunc (p *Pair[K, V]) Get(a, b int, rest ...string) (int, error)\n");
        assert!(tree.contains("(method_declaration func (parameter_list (parameter_declaration identifier (pointer_type (generic_type identifier (type_arguments identifier identifier))))) identifier (parameter_list identifier (parameter_declaration identifier identifier) (parameter_declaration identifier identifier)) (parameter_list identifier identifier))"), "{tree}");
        assert!(clean("package p\nimport (\n\t\"fmt\"\n\tstr \"strings\"\n\t. \"math\"\n)\nconst (\n\tA = iota\n\tB\n)\nvar x, y int = 1, 2\ntype (\n\tF func(int) (string, error)\n\tC <-chan map[string][]*T\n\tI interface {\n\t\t~int | ~string\n\t\tio.Reader\n\t\tM(x int) bool\n\t}\n\tAlias = int\n)\nfunc G[T any, S ~[]T](s S) T { var z T; return z }\n"));
    }

    #[test]
    fn statements_and_headers() {
        let tree = named("package p\nfunc f() {\n\tfor i := 0; i < n; i++ {\n\t}\n\tfor _, x := range []int{1, 2} {\n\t\tx++\n\t}\n\tif want := []string{\"x\"}; !eq(got, want) {\n\t}\n}\n");
        assert!(tree.contains("(for_clause (short_var_declaration identifier int_literal) (binary_expression identifier identifier) (inc_statement identifier))"), "{tree}");
        assert!(tree.contains("(range_clause identifier identifier range (composite_literal (array_type (array_length) identifier) (literal_value int_literal int_literal))) (block (inc_statement identifier))"), "{tree}");
        assert!(tree.contains("(if_statement if (short_var_declaration identifier (composite_literal (array_type (array_length) identifier) (literal_value interpreted_string_literal))) (unary_expression (call_expression identifier (arguments identifier identifier))) (block))"), "{tree}");
        // After a name, a header's `{` is the body.
        assert!(named("package p\nfunc f() { for x {\n} }\n").contains("(for_statement for identifier (block))"));
        assert!(clean("package p\nfunc f() {\n\tswitch t := v.(type) {\n\tcase int, []string:\n\t\tfallthrough\n\tdefault:\n\t}\n\tselect {\n\tcase v, ok := <-ch:\n\t\t_ = v\n\tcase ch <- 1:\n\tdefault:\n\t}\nL:\n\tfor {\n\t\tbreak L\n\t}\n\tgo func() { defer wg.Done() }()\n\tx, y = y, x\n\tm[k] += 1\n\tgoto L\n}\n"));
    }

    #[test]
    fn line_breaks_insert_semicolons() {
        // After an operand a line break ends the statement; after an operator it does not.
        assert_eq!(
            named("package p\nfunc f() {\n\tx := a +\n\t\tb\n\treturn\n}\n"),
            "(source_file (package_clause package identifier) (function_declaration func identifier (parameter_list) (block (short_var_declaration identifier (binary_expression identifier identifier)) (return_statement return))))"
        );
        // `x` then `++y` on the next line: two statements, and the second is an error.
        assert!(!clean("package p\nfunc f() {\n\tx\n\t++y\n}\n"));
        // A general comment with a line break counts as one.
        assert!(clean("package p\nvar a = 1 /* one\n*/ var b = 2\n"));
        let workspace = parse("package p\nvar s = `raw\nstring` + \"\\\"\" + 'x'\nvar n = 0x1F_ff + 1e3 + .5i + 0o17 + 0b1\n");
        let kinds: Vec<&str> = (0..workspace.tokens.len()).filter(|&at| !GO.is_trivia(workspace.tokens.kind[at])).map(|at| GO.name(workspace.tokens.kind[at])).collect();
        assert_eq!(
            kinds,
            [
                "package", "identifier", ";", "var", "identifier", "=", "raw_string_literal", "+", "interpreted_string_literal", "+", "rune_literal", ";", "var", "identifier", "=", "int_literal", "+",
                "float_literal", "+", "imaginary_literal", "+", "int_literal", "+", "int_literal", ";"
            ]
        );
    }

    /// Malformed inputs: each must report an error, and keep its root.
    #[test]
    fn malformed_inputs_report_errors() {
        let bad = [
            "package p\nfunc f( {}\n",
            "package p\nfunc f() { x := }\n",
            "package p\nfunc f() { if x == T{} { } }\n",
            "package p\nimport \"a\" \"b\"\n",
            "package p\nfunc f() {\n\tx := []int{1, 2\n\t}\n}\n",
            "package p\nfunc f() { a b }\n",
            "package p\nfunc f() { for i := 0; i < n {} }\n",
            "package p\nfunc f()\n{\n}\n",
            "package p\nfunc f() { if x {\n}\nelse {} }\n",
            "package p\ntype T struct { a, int }\n",
            "package p\nvar s = \"open\n",
            "package p\nfunc f() { switch { case: } }\n",
            "package p\nfunc f() { go }\n",
            "package p\nfunc f() { x.(1) }\n",
            "package p\nfunc f() { m := map[string]{} }\n",
            "package p\nfunc f[T]() {}\n",
        ];
        for src in bad {
            let workspace = parse(src);
            assert!(workspace.tree.root().flags & HAS_ERROR != 0, "{src:?} parsed clean: {}", workspace.tree.sexp(&GO, true));
            assert_eq!(workspace.tree.root().kind, SOURCE_FILE);
        }
    }

    #[test]
    fn deep_input_stays_within_the_budget() {
        on_big_stack(|| {
            let deep = [
                format!("package p\nvar x = {}1", "(".repeat(100_000)),
                format!("package p\nvar x = {}y", "-".repeat(100_000)),
                format!("package p\nvar x = {}y", "<-".repeat(100_000)),
                format!("package p\nvar x = {}b", "a + ".repeat(100_000)),
                format!("package p\nvar x {}int", "[]".repeat(100_000)),
                format!("package p\nvar x = {}1{}", "T{".repeat(50_000), "}".repeat(50_000)),
                format!("package p\nfunc f() {{ {} }}", "if a { ".repeat(30_000)),
                format!("package p\nvar x = {}", "func() { ".repeat(30_000)),
                "{".repeat(100_000),
            ];
            for src in deep {
                let mut workspace = Workspace::default();
                let outcome = workspace.parse(src.as_bytes(), &GO, &Budget::default()).unwrap();
                assert!(outcome.exhausted.is_none(), "{}: {}", &src[..30], outcome.describe());
                check(&workspace, src.as_bytes(), &GO);
                assert_eq!(workspace.tree.root().kind, SOURCE_FILE);
            }
        });
    }

    const SEEDS: &[&str] = &[
        "package cache\n\nimport (\n\t\"sync\"\n\t\"time\"\n)\n\ntype Cache[K comparable, V any] struct {\n\tmu    sync.Mutex\n\titems map[K]entry[V]\n}\n\nfunc (c *Cache[K, V]) Get(key K) (V, bool) {\n\tc.mu.Lock()\n\tdefer c.mu.Unlock()\n\tif e, ok := c.items[key]; ok && time.Now().Before(e.until) {\n\t\treturn e.value, true\n\t}\n\tvar zero V\n\treturn zero, false\n}\n",
        "package main\n\nfunc main() {\n\tch := make(chan int, 3)\n\tgo func() {\n\t\tfor i := range []int{1, 2, 3} {\n\t\t\tch <- i * 2\n\t\t}\n\t\tclose(ch)\n\t}()\n\tfor v := range ch {\n\t\tswitch {\n\t\tcase v > 2:\n\t\t\tprintln(v)\n\t\tdefault:\n\t\t}\n\t}\n}\n",
        "package walk\n\ntype Node interface {\n\tChildren() []Node\n}\n\nfunc Walk(n Node, visit func(Node) bool) {\nouter:\n\tfor _, c := range n.Children() {\n\t\tswitch t := c.(type) {\n\t\tcase nil:\n\t\t\tcontinue outer\n\t\tdefault:\n\t\t\tif !visit(t) {\n\t\t\t\tbreak outer\n\t\t\t}\n\t\t}\n\t\tWalk(c, visit)\n\t}\n}\n",
    ];

    const GO_BYTES: &[u8] = b"()[]{}<>:;,.=+-*/%!&|^~?'\"`\\_0123456789 \n\t xyfuncvartypestructinterfacemapchanifelseforrangeswitchcasereturngo:=<-\xc3\xa9";

    #[test]
    fn fuzzed_go_terminates_and_keeps_every_invariant() {
        on_big_stack(|| {
            let mut rng = Rng(0x5910_0004_0067_6fff);
            let mut workspace = Workspace::default();
            let budget = Budget::default();
            let mut erroneous = 0;
            const ROUNDS: usize = 3000;
            for round in 0..ROUNDS {
                let mut src = SEEDS[round % SEEDS.len()].as_bytes().to_vec();
                for _ in 0..1 + rng.below(6) {
                    let length = src.len();
                    mutate(&mut rng, &mut src, GO_BYTES, b"", 0, length);
                }
                let outcome = workspace.parse(&src, &GO, &budget).unwrap();
                assert!(outcome.exhausted.is_none(), "budget ran out on {:?}", String::from_utf8_lossy(&src));
                check(&workspace, &src, &GO);
                assert_eq!(workspace.tree.root().kind, SOURCE_FILE);
                if workspace.tree.root().flags & HAS_ERROR != 0 {
                    erroneous += 1;
                }
            }
            assert!(erroneous > ROUNDS / 2, "{erroneous} of {ROUNDS}");
        });
        for seed in SEEDS {
            assert!(clean(seed), "{seed}");
        }
    }
}
