//! C through `grammar!`: translation unit, declarations, declarators,
//! statements and one precedence table. The preprocessor is trivia (see
//! `mod.rs`); braces are plain tokens so a predicate can find the `{` that
//! makes a definition.
//!
//! Not modelled: typedef names (a bare typedef type reads as an expression
//! statement), `_Generic`, compound literals, K&R parameter declarations, and
//! attributes. Everything listed in the module docs as "modelled" has a
//! fixture or a test.

use crate::batch_c::{directive, is_word_byte, is_word_start, literal_prefix, number, quoted, trivia};
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, UNKNOWN};

rust_graph_deep::grammar! { C {
    name = "c";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{",
        RBRACE = "}",

        IDENT = "identifier",
        NUMBER = "number",
        STR = "string_literal",
        CHAR_LITERAL = "char_literal",
        BAD_LITERAL = "bad_literal" bad,
        /// One preprocessor line, with its continuations: trivia.
        DIRECTIVE = "preprocessor" trivia,

        KW_AUTO = "auto",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CHAR = "char",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DEFAULT = "default",
        KW_DO = "do",
        KW_DOUBLE = "double",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXTERN = "extern",
        KW_FLOAT = "float",
        KW_FOR = "for",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_INLINE = "inline",
        KW_INT = "int",
        KW_LONG = "long",
        KW_REGISTER = "register",
        KW_RESTRICT = "restrict",
        KW_RETURN = "return",
        KW_SHORT = "short",
        KW_SIGNED = "signed",
        KW_SIZEOF = "sizeof",
        KW_STATIC = "static",
        KW_STRUCT = "struct",
        KW_SWITCH = "switch",
        KW_TYPEDEF = "typedef",
        KW_UNION = "union",
        KW_UNSIGNED = "unsigned",
        KW_VOID = "void",
        KW_VOLATILE = "volatile",
        KW_WHILE = "while",
        KW_BOOL = "_Bool",
        KW_COMPLEX = "_Complex",
        KW_IMAGINARY = "_Imaginary",
        KW_ALIGNAS = "_Alignas",
        KW_ALIGNOF = "_Alignof",
        KW_ATOMIC = "_Atomic",
        KW_NORETURN = "_Noreturn",
        KW_STATIC_ASSERT = "_Static_assert",
        KW_THREAD_LOCAL = "_Thread_local",

        COMMA = ",",
        SEMI = ";",
        DOT = ".",
        ELLIPSIS = "...",
        QUESTION = "?",
        COLON = ":",
        ARROW = "->",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        /// A `>` directly before another `>`: the first of the pair, so a
        /// template closer and a shift can both use it.
        GT_JOINED = ">joined",
        SHL = "<<",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        ANDAND = "&&",
        PIPE = "|",
        OROR = "||",
        CARET = "^",
        TILDE = "~",
        BANG = "!",
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
        SHL_EQ = "<<=",
    }

    translation_unit: _top_item*,

    _top_item:
        &(declaration_specifiers _declarator "{") function_definition
        | declaration_statement
        | static_assert_statement,

    function_definition: declaration_specifiers _declarator compound_statement,

    declaration_statement: declaration_specifiers (_init_declarator % ",") ";",
    _init_declarator: _declarator ["=" _initializer],
    _initializer: designated_field | designated_index | _expression@assign | initializer_list,
    initializer_list: "{" (_initializer %? ",") "}",
    /// C99 `.name = value` inside an initializer list.
    designated_field: "." IDENT "=" _initializer,
    /// C99 `[index] = value` inside an initializer list.
    designated_index: array_suffix "=" _initializer,

    static_assert_statement: "_Static_assert" static_assert_arguments ";",
    static_assert_arguments: "(" _expression ["," string] ")",

    declaration_specifiers: _declaration_specifier+,
    _declaration_specifier:
        "typedef" | "extern" | "static" | "auto" | "register" | "inline" | "_Noreturn"
        | "_Thread_local" | "const" | "volatile" | "restrict" | "_Atomic"
        | "_Alignas" alignment_arguments
        | type_specifier,
    alignment_arguments: "(" _expression ")",

    type_specifier:
        "void" | "char" | "short" | "int" | "long" | "float" | "double"
        | "signed" | "unsigned" | "_Bool" | "_Complex" | "_Imaginary"
        | struct_or_union_specifier
        | enum_specifier,

    struct_or_union_specifier: ("struct" | "union") [IDENT] [struct_body],
    struct_body: "{" _struct_declaration* "}",
    _struct_declaration: declaration_specifiers (_struct_declarator % ",") ";",
    _struct_declarator: _declarator [":" _expression@assign],

    enum_specifier: "enum" [IDENT] [enum_body],
    enum_body: "{" (_enumerator %? ",") "}",
    _enumerator: IDENT ["=" _expression@assign],

    _declarator: pointer* direct_declarator,
    pointer: "*" _pointer_qualifier*,
    _pointer_qualifier: "const" | "volatile" | "restrict",
    direct_declarator: _direct_base _declarator_suffix*,
    _direct_base: IDENT | parenthesized_declarator,
    parenthesized_declarator: "(" _declarator ")",
    _declarator_suffix: array_suffix | parameter_list,
    array_suffix: "[" [_expression] "]",
    parameter_list: "(" (_parameter % ",") ")",
    _parameter: parameter_declaration | "...",
    parameter_declaration: declaration_specifiers [_declarator],

    _abstract_declarator: pointer+ abstract_suffix*,
    abstract_suffix: array_suffix | parameter_list,
    type_name: declaration_specifiers [_abstract_declarator],

    compound_statement: "{" _statement* "}",

    _statement:
        compound_statement
        | if_statement
        | switch_statement
        | while_statement
        | do_statement
        | for_statement
        | return_statement
        | break_statement
        | continue_statement
        | goto_statement
        | labeled_statement
        | static_assert_statement
        | declaration_statement
        | expression_statement,

    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    switch_statement: "switch" parenthesized_expression switch_body,
    switch_body: "{" _switch_item* "}",
    _switch_item: case_statement | default_statement | _statement,
    case_statement: "case" _expression@assign ":" _statement*,
    default_statement: "default" ":" _statement*,
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: "for" for_header _statement,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression] ")",
    _for_init: _for_declaration | _expression,
    _for_declaration: declaration_specifiers (_init_declarator % ","),
    parenthesized_expression: "(" _expression ")",
    return_statement: "return" [_expression] ";",
    break_statement: "break" ";",
    continue_statement: "continue" ";",
    goto_statement: "goto" (IDENT | "*" _expression) ";",
    labeled_statement: IDENT ":" _statement,
    expression_statement: _expression ";",

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" => assignment_expression,
        cond: postfix "?" _expression@assign ":" _expression@assign => conditional_expression,
        lor: left "||" => binary_expression,
        land: left "&&" => binary_expression,
        bor: left "|" => binary_expression,
        bxor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        eq: left "==" | "!=" => binary_expression,
        rel: left "<" | ">" | "<=" | ">=" => binary_expression,
        shift: left "<<" | GT_JOINED ">" => binary_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        unary: prefix "!" | "~" | "-" | "+" | "*" | "&" => unary_expression,
        postfix "++" | "--" => update_expression,
        postfix arguments => call_expression,
        postfix subscript => subscript_expression,
        postfix "." _member => field_expression,
        postfix "->" _member => field_expression,
    },

    _primary:
        &(cast_type) cast_expression
        | sizeof_expression
        | alignof_expression
        | parenthesized_expression
        | string
        | _name
        | NUMBER
        | CHAR_LITERAL,

    _name: IDENT,
    _member: IDENT,
    string: STR+,
    cast_expression: cast_type _expression@unary,
    cast_type: "(" type_name ")",
    sizeof_expression: "sizeof" sizeof_operand,
    sizeof_operand: &(cast_type) cast_type | _expression@unary,
    alignof_expression: "_Alignof" cast_type,
    arguments: "(" (_argument %? ",") ")",
    _argument: _expression@assign,
