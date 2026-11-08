//! Groovy: scripts and classes. A script is a run of statements at file level;
//! a class has a head with `extends`/`implements` and a braced body of members.
//!
//! Groovy lets a statement end at a newline or at `;`, so the lexer treats both
//! as trivia and a statement ends where the next token cannot continue the one
//! being parsed — the same choice the AWK grammar makes, and the reason
//! `x = 1` then `println x` on the next line are two statements.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A GString is one token.** `"Hello ${name}"` never exposes `name` as an
//!   expression; interpolation is opaque.
//! - **A map literal `[:]` / `[a: 1]` is not modelled.** It starts with the same
//!   bracket a list literal does and telling them apart needs the token after
//!   the first key, which the DSL cannot look at. `[1, 2]` is a list.
//! - **`def` heads one rule for both a method and a variable.** `def f(a) {}`
//!   and `def x = 1` differ only in the token after the name, so the shared
//!   `definition` node has a `parameter_list` (a callable) or an initializer (a
//!   variable) inside it instead of two kinds.
//! - **Command syntax** (`println "hi"`, no parentheses) is not modelled; a
//!   call is `name(args)`.
//! - **Annotations and traits** are lexed as `@` and a name and are accepted as
//!   a statement of their own, not attached to the declaration that follows.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { GROOVY {
    name = "groovy";
    lex = lex_groovy;

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
        ARROW = "->",
        QUESTION = "?",
        ASSIGN = "=",
        PLUS_ASSIGN = "+=",
        MINUS_ASSIGN = "-=",
        STAR_ASSIGN = "*=",
        SLASH_ASSIGN = "/=",
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
        STARSTAR = "**",
        INCR = "++",
        DECR = "--",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        TILDE = "~",
        SHL = "<<",
        SHR = ">>",
        SPREAD = "..",
        SPREAD_DOT = "..<",
        AT = "@",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,

        KW_PACKAGE = "package",
        KW_IMPORT = "import",
        KW_AS = "as",
        KW_DEF = "def",
        KW_CLASS = "class",
        KW_INTERFACE = "interface",
        KW_ENUM = "enum",
        KW_EXTENDS = "extends",
        KW_IMPLEMENTS = "implements",
        KW_STATIC = "static",
        KW_FINAL = "final",
        KW_PRIVATE = "private",
        KW_PUBLIC = "public",
        KW_PROTECTED = "protected",
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
        KW_NEW = "new",
        KW_THIS = "this",
        KW_SUPER = "super",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_NULL = "null",
        KW_VOID = "void",
        KW_INT = "int",
        KW_LONG = "long",
        KW_DOUBLE = "double",
        KW_BOOLEAN = "boolean",
        KW_STRING = "String",
    }

    program: _statement*,

    _statement:
        package_declaration
        | import_declaration
        | class_declaration
        | interface_declaration
        | enum_declaration
        | annotation
        | &("def" IDENT) definition
        | if_statement
        | for_statement
        | while_statement
        | switch_statement
        | try_statement
        | return_statement
        | throw_statement
        | break_statement
        | continue_statement
        | expression_statement,

    package_declaration: "package" dotted_name,
    import_declaration: "import" ["static"] dotted_name ["." "*"] ["as" _name],
    dotted_name: _name ("." _name)*,
    annotation: "@" _name ["." _name] [argument_list],

    class_declaration: "class" _name [type_parameters] [class_heritage] class_body,
    interface_declaration: "interface" _name [type_parameters] [class_heritage] class_body,
    enum_declaration: "enum" _name ["implements" _type ("," _type)*] enum_body,
    class_heritage: _heritage_clause*,
    _heritage_clause: "extends" _type | "implements" _type ("," _type)*,
    class_body: "{" _member* "}",
    enum_body: "{" [enum_item ("," enum_item)*] "}",
    enum_item: _name [argument_list],

    _member: &("def" IDENT) definition | field_or_method,
    /// `def f(...)`, `def x = 1`, `String name = 'x'` and `int size()` all have
    /// the shape `[def] type? name` and are told apart by their tail.
    definition: ["def"] modifier* _head_type [_head_type] [_definition_tail],
    /// A field, a method or a constructor: `type? name` then a parameter list,
    /// an initializer, or neither. The head is shared because a constructor
    /// (`Foo(`) and a typed field (`String name =`) differ only after the name.
    field_or_method: modifier* _head_type [_head_type] [_definition_tail],
    modifier: "static" | "final" | "private" | "public" | "protected",
    _head_type: IDENT | "void" | "int" | "long" | "double" | "boolean" | "String",
    _definition_tail: parameter_list block | initializer,
    initializer: "=" _expression,

    parameter_list: "(" [_parameter %? ","] ")",
    _parameter: ["final"] parameter_body,
    parameter_body:
        &(IDENT IDENT) typed_parameter
        | &("String" IDENT) typed_parameter
        | &("int" IDENT) typed_parameter
        | &("long" IDENT) typed_parameter
        | &("double" IDENT) typed_parameter
        | &("boolean" IDENT) typed_parameter
        | &("void" IDENT) typed_parameter
        | plain_parameter,
    typed_parameter: _type _name ["=" _expression],
    plain_parameter: _name ["=" _expression],

    block: "{" _statement* "}",
    if_statement: "if" paren_expression block [else_clause],
    else_clause: "else" (block | if_statement),
    for_statement: "for" for_header block,
    for_header: "(" for_body ")",
    for_body: &(IDENT "in") for_in_body | &("def" IDENT "in") for_in_body | classic_for_body,
    classic_for_body: [for_init] ";" [_expression] ";" [_expression_list],
    for_in_body: ["def"] _name "in" _expression,
    for_init: ("def" | "final") _name ["=" _expression] | _expression,
    while_statement: "while" paren_expression block,
    switch_statement: "switch" paren_expression switch_body,
    switch_body: "{" switch_clause* "}",
    switch_clause: ("case" _expression | "default") ":" _statement*,
    try_statement: "try" block catch_clause* [finally_clause],
    catch_clause: "catch" catch_parameter block,
    catch_parameter: "(" _type _name ")",
    finally_clause: "finally" block,
    return_statement: "return" [_expression],
    throw_statement: "throw" _expression,
    break_statement: "break",
    continue_statement: "continue",
    expression_statement: _expression,
    paren_expression: "(" _expression ")",

    type_parameters: "<" _name ("," _name)* ">",
    _type: _head_type ["<" _type ("," _type)* ">"] array_suffix*,
    array_suffix: "[" "]",
    _name: IDENT,
    _expression_list: _expression ("," _expression)*,

    _expression: precedence _primary {
        right "=" | "+=" | "-=" | "*=" | "/=" => assignment,
        cond: right "?" _expression ":" => conditional,
        left "||" => logical_or,
        left "&&" => logical_and,
        left "|" => bitwise_or,
        left "^" => bitwise_xor,
        left "&" => bitwise_and,
        left "==" | "!=" => equality,
        left "<" | ">" | "<=" | ">=" | "in" => comparison,
        left "<<" | ">>" => shift,
        left "+" | "-" => additive,
        left "*" | "/" | "%" => multiplicative,
