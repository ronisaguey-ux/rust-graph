//! Go through `grammar!`, with the automatic semicolon insertion Go's
//! specification defines done in the step lexer.
//!
//! Node names follow Tree-sitter's Go where that costs nothing; the tree is
//! not the same tree.
//!
//! **Modelled.** Package and import declarations (single and parenthesised
//! lists, aliases, dot imports); `const`/`var`/`type` declarations in groups,
//! with grouped names, optional types and initialisers; generic type
//! parameters and constraints (`~T | U`); function and method declarations
//! (pointer receivers, variadic parameters, multiple results); every type
//! form (pointers, arrays, slices, maps, channels in both directions, function
//! types, structs with tags, interfaces with embedded types and methods);
//! blocks, `if` with an init statement, three-clause and `range` `for`,
//! `switch` (expression and type switches), `select`, `go`, `defer`, `goto`,
//! labels, `fallthrough`, `return`, `break`/`continue` with labels; channel
//! sends and receives, `++`/`--`, short declarations and augmented
//! assignments; the whole binary operator precedence table, unary operators,
//! calls, indexes and slices, selectors, type assertions, function literals,
//! composite literals (keyed and positional, nested) and conversions.
//!
//! **Not modelled.** A bare `T{...}` composite literal is not accepted where
//! a control-flow header is being parsed (Go's own grammar has the same
//! restriction: `if x {` must not read `x{...}`), and the grammar has no
//! general "type names from this package" knowledge. Struct field tags are
//! kept as string tokens. Comments are trivia and are not attached to nodes.
//! `//go:build` lines are ordinary comments. Semantic things — package
//! resolution, iota evaluation, whether a `range` variable is a copy — are
//! out of scope by construction.
//!
//! **DSL gaps.** (1) A lexer cannot tell "this `{` follows a name with no
//! space" because the step function sees bytes only from the cursor forward;
//! the header restriction above is the grammar-level workaround. (2) Token
//! text cannot be changed after lexing, so an inserted semicolon is a `;`
//! token whose span is whitespace, not a zero-width token. (3) There is no
//! way to mark a rule's alternatives with different node names, so method and
//! function declarations are separate rules that share their body shape.
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

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
    use super::*;
    use crate::batch_a::fixtures;

    const VALID: &[(&str, &str)] = &[
        ("cache.go", "package_clause import_declaration type_declaration type_declaration function_declaration method_declaration method_declaration method_declaration"),
        ("cli.go", "package_clause import_declaration var_declaration function_declaration"),
        ("generic.go", "package_clause import_declaration type_declaration function_declaration function_declaration function_declaration"),
        ("handlers.go", "package_clause import_declaration type_declaration method_declaration method_declaration function_declaration"),
        ("models.go", "package_clause import_declaration type_declaration type_declaration type_declaration type_declaration type_declaration method_declaration"),
        ("pipeline.go", "package_clause import_declaration const_declaration var_declaration type_declaration type_declaration function_declaration function_declaration"),
        ("server.go", "package_clause import_declaration type_declaration method_declaration method_declaration"),
        ("worker.go", "package_clause import_declaration type_declaration function_declaration"),
    ];

    const MALFORMED: &[(&str, &str)] = &[
        ("broken_signature.go", "package_clause function_declaration"),
        ("unclosed_brace.go", "package_clause type_declaration"),
        ("unterminated_string.go", "package_clause var_declaration function_declaration"),
    ];

    #[test]
    fn go_fixtures_parse_cleanly() {
        fixtures::valid("go", &GO, VALID);
    }

    #[test]
    fn go_malformed_fixtures_keep_the_root_and_report_inside_it() {
        fixtures::malformed("go", &GO, MALFORMED);
    }

    #[test]
    fn go_lexer_inserts_semicolons() {
        let kinds = fixtures::tokens(&GO, "package p\nfunc f() {\n\tx := a +\n\t\tb\n\treturn\n}\n");
        assert!(kinds.contains(&";"), "a line end after an operand is a semicolon");
        assert!(fixtures::clean(&GO, "package p\nvar x = 1 +\n\t2\n"));
        assert!(!fixtures::clean(&GO, "package p\nvar x = \"open\n"));
    }

}
