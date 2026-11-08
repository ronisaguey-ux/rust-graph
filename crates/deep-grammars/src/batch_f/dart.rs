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
