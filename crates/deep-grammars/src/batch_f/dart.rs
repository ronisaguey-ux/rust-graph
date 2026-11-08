//! Dart: libraries, classes, top-level functions and the statement forms of a
//! C-family language.
//!
//! Dart is brace-and-semicolon syntax, so most of it is the familiar statement
//! and precedence tables. The lexer knows `//`, `///` and `/* */` comments,
//! single/double/triple-quoted strings and numeric literals with `_`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **String interpolation stays opaque.** `"${a + b}"` is one `string` token;
//!   the field is not an expression node.
//! - **Type arguments are not parsed.** `List<int>` lexes `<` and `>` as
//!   operators; a generic type annotation is not a node, and a generic function
//!   declaration is only accepted where a predicate can see the `<` (see
//!   `_class_member`). A declaration written with a generic return type falls
//!   through to an expression statement and produces an error, which is honest.
//! - **Class members are one kind** (`member_declaration`) for a field, a
//!   method or a constructor. Splitting them needs the token after the name,
//!   which is one past what a three-token predicate can see; the head is
//!   `type? name` and the rest decides.
//! - **Annotations (`@override`)** are tokens in a `metadata` rule that may
//!   precede a declaration; their arguments are parsed, their meaning is not.
//! - **`switch` cases** are parsed as a flat list of `case`/`default` clauses
//!   with statements; pattern syntax beyond a constant or a name is not modelled.
//! - **A map literal `{...}` and an arrow closure `(a) => b` are not
//!   expressions here**: both start with a delimiter a predicate cannot name,
//!   and `{` is already the start of a block. A fixture that used either would
//!   fail, so it is named rather than half-parsed.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { DART {
    name = "dart";
    lex = lex_dart;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        COMMA = ",",
        SEMI = ";",
        COLON = ":",
        DOT = ".",
        QUESTION_DOT = "?.",
        QUESTION = "?",
        ARROW = "=>",
        ASSIGN = "=",
        PLUS_ASSIGN = "+=",
        MINUS_ASSIGN = "-=",
        STAR_ASSIGN = "*=",
        SLASH_ASSIGN = "/=",
        PERCENT_ASSIGN = "%=",
        EQ = "==",
        NE = "!=",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
        AND = "&&",
        OR = "||",
        BANG = "!",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        TILDE_SLASH = "~/",
        TILDE = "~",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        SHL = "<<",
        SHR = ">>",
        INCR = "++",
        DECR = "--",
        AT = "@",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_IMPORT = "import",
        KW_EXPORT = "export",
        KW_LIBRARY = "library",
        KW_PART = "part",
        KW_CLASS = "class",
        KW_ABSTRACT = "abstract",
        KW_INTERFACE = "interface",
        KW_MIXIN = "mixin",
        KW_ENUM = "enum",
        KW_EXTENSION = "extension",
        KW_EXTENDS = "extends",
        KW_IMPLEMENTS = "implements",
        KW_WITH = "with",
        KW_ON = "on",
        KW_OF = "of",
        KW_TYPEDEF = "typedef",
        KW_STATIC = "static",
        KW_FINAL = "final",
        KW_CONST = "const",
        KW_VAR = "var",
        KW_LATE = "late",
        KW_VOID = "void",
        KW_DYNAMIC = "dynamic",
        KW_INT = "int",
        KW_DOUBLE = "double",
        KW_NUM = "num",
        KW_BOOL = "bool",
        KW_NEW = "new",
        KW_THIS = "this",
        KW_SUPER = "super",
        KW_RETURN = "return",
        KW_IF = "if",
        KW_ELSE = "else",
        KW_FOR = "for",
        KW_IN = "in",
        KW_WHILE = "while",
        KW_DO = "do",
        KW_SWITCH = "switch",
        KW_CASE = "case",
        KW_DEFAULT = "default",
        KW_BREAK = "break",
        KW_CONTINUE = "continue",
        KW_TRY = "try",
        KW_CATCH = "catch",
        KW_FINALLY = "finally",
        KW_THROW = "throw",
        KW_RETHROW = "rethrow",
        KW_ASYNC = "async",
        KW_AWAIT = "await",
        KW_YIELD = "yield",
        KW_GET = "get",
        KW_SET = "set",
        KW_FACTORY = "factory",
        KW_EXTERNAL = "external",
        KW_COVARIANT = "covariant",
        KW_REQUIRED = "required",
        KW_OPERATOR = "operator",
        KW_AS = "as",
        KW_IS = "is",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_NULL = "null",
        KW_SHOW = "show",
        KW_HIDE = "hide",
        KW_ASYNC_STAR = "async*",
        KW_SYNC_STAR = "sync*",
    }

    program: _declaration*,

    _declaration:
        import_directive
        | export_directive
        | library_directive
        | part_directive
        | class_declaration
        | mixin_declaration
        | enum_declaration
        | extension_declaration
        | typedef_declaration
        | &(IDENT IDENT) function_declaration
        | &("void" IDENT) function_declaration
        | &("int" IDENT) function_declaration
        | &("bool" IDENT) function_declaration
        | &("double" IDENT) function_declaration
        | metadata
        | _statement,

    import_directive: "import" STRING [import_tail] ";",
    import_tail: ["as" IDENT] [("show" | "hide") IDENT ("," IDENT)*],
    export_directive: "export" STRING ("," STRING)* ["show" IDENT ("," IDENT)*] ";",
    library_directive: "library" IDENT ("." IDENT)* ";",
    part_directive: "part" (STRING | "of" IDENT ("." IDENT)*) ";",

    metadata: "@" _name ["." _name] [argument_list],

    class_declaration: ["abstract"] "class" IDENT [type_parameters] [class_heritage] class_body,
    mixin_declaration: "mixin" IDENT [type_parameters] [mixin_heritage] class_body,
    enum_declaration: "enum" IDENT [class_heritage] enum_body,
    extension_declaration: "extension" IDENT [type_parameters] ["on" _type] class_body,
    typedef_declaration: "typedef" IDENT [type_parameters] "=" _type ";",

    class_heritage: _heritage_clause*,
    _heritage_clause: "extends" _type | "implements" _type ("," _type)* | "with" _type ("," _type)*,
    mixin_heritage: _heritage_clause*,

    type_parameters: "<" type_parameter ("," type_parameter)* ">",
    type_parameter: IDENT ["extends" _type],
    _type: _name | "void" | "dynamic" | "int" | "double" | "num" | "bool",

    class_body: "{" _class_member* "}",
    enum_body: "{" [_enum_item ("," _enum_item)*] [","] [enum_members] "}",
    _enum_item: IDENT [argument_list],
    enum_members: ";" _class_member*,

    _class_member: metadata | member_declaration,

    /// A field, a method or a constructor: `type? name` then either a parameter
    /// list (a callable) or a declarator tail (a field). The head is shared
    /// because telling `int x = 1;` from `int f(` needs the token after the
    /// name, which is one past what a predicate may look at.
    member_declaration: member_modifier* member_head _member_rest,
    member_head: _head_type [IDENT],
    _head_type: IDENT | "void" | "dynamic" | "int" | "double" | "num" | "bool",
    member_modifier: "static" | "final" | "const" | "late" | "abstract" | "factory" | "external",
    _member_rest:
        parameter_list [method_effect] [initializer_list] (block | ";" | "=>" _expression ";")
        | _field_rest ";",
    _field_rest: ["=" _expression] ("," IDENT ["=" _expression])*,
    initializer_list: ":" _expression ("," _expression)*,
    method_effect: "async" | "async*" | "sync*",
    parameter_list: "(" [_parameter %? ","] ")",
    _parameter: ["required"] _parameter_kind,
    _parameter_kind:
        &("this" ".") this_parameter
        | &("super" ".") this_parameter
        | &(IDENT IDENT) typed_parameter
        | &("int" IDENT) typed_parameter
        | &("double" IDENT) typed_parameter
        | &("num" IDENT) typed_parameter
        | &("bool" IDENT) typed_parameter
        | &("dynamic" IDENT) typed_parameter
        | &("void" IDENT) typed_parameter
        | plain_parameter,
    this_parameter: ("this" | "super") "." _name,
    typed_parameter: parameter_modifier* _type _name ["=" _expression],
    plain_parameter: _name ["=" _expression],
    parameter_modifier: "final" | "const" | "covariant" | "required",

    variable_declaration: ("var" | "final" | "const" | "late") _declarators ";",
    _declarators: declarator ("," declarator)*,
    declarator: _name ["=" _expression],
    variable_head: ("var" | "final" | "const" | "late") _declarators,

    function_declaration:
        ["external"]
        _type
        _name
        [type_parameters]
        parameter_list
        [method_effect]
        (block | "=>" _expression ";"),

    _statement:
        block
        | if_statement
        | for_statement
        | while_statement
        | do_statement
        | switch_statement
        | try_statement
        | return_statement
        | break_statement
        | continue_statement
        | throw_statement
        | variable_declaration
        | &(IDENT IDENT "=") local_declaration
        | &(IDENT IDENT ";") local_declaration
        | expression_statement
        | ";",

    block: "{" _statement* "}",
    if_statement: "if" paren_expression _statement ["else" _statement],
    for_statement: "for" for_header _statement,
    for_header: "(" [for_init] ";" [_expression] ";" [_expression_list] ")",
    for_init: variable_head | local_declaration | _expression,
    local_declaration: _type IDENT ["=" _expression] ("," IDENT ["=" _expression])* ";",
    while_statement: "while" paren_expression _statement,
    do_statement: "do" _statement "while" paren_expression ";",
    switch_statement: "switch" paren_expression switch_body,
    switch_body: "{" switch_clause* "}",
    switch_clause: ("case" _expression | "default") ":" _statement*,
    try_statement: "try" block catch_clause* [finally_clause],
    catch_clause: "on" _type [catch_parameters] block,
    catch_parameters: "catch" exception_parameter,
    exception_parameter: "(" IDENT ")",
    finally_clause: "finally" block,
    return_statement: "return" [_expression] ";",
    break_statement: "break" ";",
    continue_statement: "continue" ";",
    throw_statement: "throw" _expression ";",
    expression_statement: _expression ";",

    _expression_list: _expression ("," _expression)*,

    _expression: precedence _primary {
        right "=" | "+=" | "-=" | "*=" | "/=" | "%=" => assignment,
        cond: right "?" _expression ":" => conditional,
        left "||" => logical_or,
        left "&&" => logical_and,
        left "==" | "!=" => equality,
        left "<" | ">" | "<=" | ">=" | "is" ["!"] | "as" => relational,
        left "|" => bitwise_or,
        left "^" => bitwise_xor,
        left "&" => bitwise_and,
        left "<<" | ">>" => shift,
        left "+" | "-" => additive,
        left "*" | "/" | "%" | "~/" => multiplicative,
        prefix "!" | "-" | "~" => unary,
        prefix "await" | "new" => await_expression,
        postfix "++" | "--" => update,
        prefix "++" | "--" => update,
        postfix argument_list => call,
        postfix "." _name => member_access,
        postfix "?." _name => null_aware_member,
        postfix index_expression => index,
        postfix "!" => null_assertion,
    },

    argument_list: "(" [_argument %? ","] ")",
    index_expression: "[" _expression "]",
    _argument: &(IDENT ":") named_argument | _expression,
    named_argument: _name ":" _expression,
    _name: IDENT,

    _primary:
        IDENT
        | NUMBER
        | string_literal
        | "true"
        | "false"
        | "null"
        | "this"
        | "super"
        | parenthesized_expression
        | list_literal,

    string_literal: (STRING | BAD_STRING)+,
    parenthesized_expression: "(" (_expression %? ",") ")",
    paren_expression: "(" _expression ")",
    list_literal: "[" [_expression %? ","] "]",
} }

/// One token at the cursor.
pub fn lex_dart(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            if crate::batch_d::lex::block_comment(c, b"/*", b"*/", true) {
                COMMENT
            } else {
                COMMENT
            }
        }
        b'\'' | b'"' => string(c),
        b'r' | b'R' if matches!(c.peek_at(1), b'\'' | b'"') => {
            c.bump();
            string(c)
        }
        b'0'..=b'9' => { number(c); NUMBER }
        b'.' if c.peek_at(1).is_ascii_digit() => { number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => word(c),
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b':' => { c.bump(); COLON }
        b'.' => { c.bump(); DOT }
        b'?' if c.peek_at(1) == b'.' => { c.bump_n(2); QUESTION_DOT }
        b'?' => { c.bump(); QUESTION }
        b'=' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' => { c.bump(); ASSIGN }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' => { c.bump(); BANG }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'<' => { c.bump_n(2); SHL }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' if c.peek_at(1) == b'>' => { c.bump_n(2); SHR }
        b'>' => { c.bump(); GT }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'&' => { c.bump(); AMP }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); OR }
        b'|' => { c.bump(); PIPE }
        b'^' => { c.bump(); CARET }
        b'~' if c.peek_at(1) == b'/' => { c.bump_n(2); TILDE_SLASH }
        b'~' => { c.bump(); TILDE }
        b'+' if c.peek_at(1) == b'+' => { c.bump_n(2); INCR }
        b'+' if c.peek_at(1) == b'=' => { c.bump_n(2); PLUS_ASSIGN }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'-' => { c.bump_n(2); DECR }
        b'-' if c.peek_at(1) == b'=' => { c.bump_n(2); MINUS_ASSIGN }
        b'-' => { c.bump(); MINUS }
        b'*' if c.peek_at(1) == b'=' => { c.bump_n(2); STAR_ASSIGN }
        b'*' => { c.bump(); STAR }
        b'/' if c.peek_at(1) == b'=' => { c.bump_n(2); SLASH_ASSIGN }
        b'/' => { c.bump(); SLASH }
        b'%' if c.peek_at(1) == b'=' => { c.bump_n(2); PERCENT_ASSIGN }
        b'%' => { c.bump(); PERCENT }
        b'@' => { c.bump(); AT }
        _ => UNKNOWN,
    }
}

/// The prefix `async*`/`sync*` cannot be a word because `*` is an operator, so
/// the two are recognised here rather than in `word`.
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$');
    let text = &start[..length];
    if text == b"async" && c.peek() == b'*' {
        c.bump();
        return KW_ASYNC_STAR;
    }
    if text == b"sync" && c.peek() == b'*' {
        c.bump();
        return KW_SYNC_STAR;
    }
    keyword(text)
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"import" => KW_IMPORT,
        b"export" => KW_EXPORT,
        b"library" => KW_LIBRARY,
        b"part" => KW_PART,
        b"class" => KW_CLASS,
        b"abstract" => KW_ABSTRACT,
        b"interface" => KW_INTERFACE,
        b"mixin" => KW_MIXIN,
        b"enum" => KW_ENUM,
        b"extension" => KW_EXTENSION,
        b"extends" => KW_EXTENDS,
        b"implements" => KW_IMPLEMENTS,
        b"with" => KW_WITH,
        b"on" => KW_ON,
        b"of" => KW_OF,
        b"typedef" => KW_TYPEDEF,
        b"static" => KW_STATIC,
        b"final" => KW_FINAL,
        b"const" => KW_CONST,
        b"var" => KW_VAR,
        b"late" => KW_LATE,
        b"void" => KW_VOID,
        b"dynamic" => KW_DYNAMIC,
        b"int" => KW_INT,
        b"double" => KW_DOUBLE,
        b"num" => KW_NUM,
        b"bool" => KW_BOOL,
        b"new" => KW_NEW,
        b"this" => KW_THIS,
        b"super" => KW_SUPER,
        b"return" => KW_RETURN,
        b"if" => KW_IF,
        b"else" => KW_ELSE,
        b"for" => KW_FOR,
        b"in" => KW_IN,
        b"while" => KW_WHILE,
        b"do" => KW_DO,
        b"switch" => KW_SWITCH,
        b"case" => KW_CASE,
        b"default" => KW_DEFAULT,
        b"break" => KW_BREAK,
        b"continue" => KW_CONTINUE,
        b"try" => KW_TRY,
        b"catch" => KW_CATCH,
        b"finally" => KW_FINALLY,
        b"throw" => KW_THROW,
        b"rethrow" => KW_RETHROW,
        b"async" => KW_ASYNC,
        b"await" => KW_AWAIT,
        b"yield" => KW_YIELD,
        b"get" => KW_GET,
        b"set" => KW_SET,
        b"factory" => KW_FACTORY,
        b"external" => KW_EXTERNAL,
        b"covariant" => KW_COVARIANT,
        b"required" => KW_REQUIRED,
        b"operator" => KW_OPERATOR,
        b"as" => KW_AS,
        b"is" => KW_IS,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        b"null" => KW_NULL,
        b"show" => KW_SHOW,
        b"hide" => KW_HIDE,
        _ => IDENT,
    }
}

/// A string from its opening quote: one line, or triple-quoted to the closing
/// three quotes. `r` prefixes are handled by the caller.
fn string(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    let triple = c.peek_at(1) == quote && c.peek_at(2) == quote;
    if triple {
        c.bump_n(3);
        loop {
            if c.at_end() {
                return BAD_STRING;
            }
            if c.peek() == b'\\' {
                c.bump_n(2);
            } else if c.peek() == quote && c.peek_at(1) == quote && c.peek_at(2) == quote {
                c.bump_n(3);
                return STRING;
            } else {
                c.bump_char();
            }
        }
    }
    c.bump();
    if c.quoted(quote, true) {
        STRING
    } else {
        BAD_STRING
    }
}

fn number(c: &mut Cursor) {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return;
    }
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.peek() == b'.' && c.peek_at(1).is_ascii_digit() {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'e' | b'E') {
        let sign = usize::from(matches!(c.peek_at(1), b'+' | b'-'));
        if c.peek_at(1 + sign).is_ascii_digit() {
            c.bump_n(1 + sign);
            c.eat_while(|byte| byte.is_ascii_digit());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_f::harness;

    const VALID: &[(&str, &[&str])] = &[
        (
            "main.dart",
            &[
                "import_directive",
                "import_directive",
                "part_directive",
                "class_declaration",
                "function_declaration",
            ],
        ),
        (
            "models.dart",
            &["library_directive", "enum_declaration", "class_declaration", "class_declaration"],
        ),
        (
            "pipeline.dart",
            &[
                "import_directive",
                "variable_declaration",
                "variable_declaration",
                "variable_declaration",
                "function_declaration",
                "function_declaration",
                "function_declaration",
            ],
        ),
    ];

    #[test]
    fn valid_fixtures() {
        harness::check_valid("dart", &DART, VALID);
    }

    #[test]
    fn malformed_keeps_the_root() {
        harness::check_malformed(
            "malformed",
            "dart_broken.dart",
            &DART,
            &["import_directive", "class_declaration"],
        );
    }

    #[test]
    #[ignore]
    fn dump_top_level_kinds() {
        harness::dump("dart", &DART);
        harness::dump("malformed", &DART);
    }

    #[test]
    fn lexes_strings_and_comments() {
        use crate::deep::lexer::{lex, Tokens};
        let src = br#"// c
var s = "a ${b} c";
var t = '''x
y''';
"#;
        let mut tokens = Tokens::default();
        lex(src, &DART, &mut tokens);
        let kinds: Vec<&str> = (0..tokens.len()).map(|i| DART.name(tokens.kind[i])).collect();
        assert!(kinds.contains(&"string"), "{kinds:?}");
        assert_eq!(kinds.iter().filter(|kind| **kind == "string").count(), 2, "{kinds:?}");
        assert!(kinds.contains(&"comment"), "{kinds:?}");
    }
}
