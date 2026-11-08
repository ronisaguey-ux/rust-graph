//! JavaScript, TypeScript and TSX through one `grammar!`: Slice 4. TypeScript is
//! a superset of JavaScript, so the grammar is TypeScript's, and the three
//! languages differ only in the lexer: JavaScript and TSX read `<` where an
//! operand starts as a JSX element, TypeScript as a type assertion `<T>x`.
//! Node names follow Tree-sitter's TypeScript (a superset of its JavaScript)
//! where that costs nothing; the tree is not the same tree.
//!
//! Choices that make it LL(3):
//!
//! - Semicolons are optional everywhere a statement may end, and an expression
//!   takes every operator that can continue it, across lines. That is automatic
//!   semicolon insertion for real code, where `a\n(b)` is a call too. A
//!   restricted production (`return` and a value on the next line) reads as one
//!   statement here, not two.
//! - A template literal and a JSX element are one token each; the lexer scans
//!   their `${...}` and `{...}` parts with the braces balanced. A regular
//!   expression is a token where an operand may start: after anything that
//!   cannot end an operand ([`Cursor::previous`]).
//! - `>` right before `>` is `GT_JOINED`, so `Map<K, Set<V>>` closes two lists
//!   and `a >> b` is `GT_JOINED ">"`, as in the Rust grammar.
//! - What only a longer look tells apart is decided by predicates: an arrow
//!   function's head `(a, b) =>`, a generic call `f<T>(x)`, `for (x of y)`,
//!   statements that start with a contextual keyword (`type X = ...`), and a
//!   member's modifiers (`get x() {}` against a method named `get`).
//! - Statements use `_expression_st`, whose first operand is not `{`,
//!   `function` or `class`: those start a block or a declaration there.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **Context-sensitive tokens.** Whether `/` starts a regular expression or
//!   is division, and whether `<` starts JSX or is an operator, depends on the
//!   tokens before it. The DSL has no lexer-state hook (no mode stack, no
//!   "external scanner"), so the step lexer decides from
//!   [`Cursor::previous`] and from the language it was built for, and the
//!   three languages differ only in which function is passed as `lex`.
//! - **`>>` in a type.** Closing two type argument lists with `>>` would read
//!   as one shift token, and the DSL cannot split a token. `>` directly before
//!   `>` is therefore lexed as `GT_JOINED` followed by `>`, and both the type
//!   list and the shift operator accept that pair. This is a lexer
//!   workaround the DSL's token model forces, not a language rule.
//! - **A decision sees three tokens.** `(a, b) =>`, `f<T>(x)` and `for (x of
//!   y)` need more context than that, so they are entered through `&(...)`
//!   predicates; there is no arbitrary lookahead and no priority between
//!   alternatives beyond their order.
//! - **One kind per rule**, so a node that can be several things is several
//!   rules rather than a labelled alternative.
//! - **No grammar-level "empty statement" sugar**: an optional item is `[x]`
//!   or `x?`, and an empty alternative is a DSL error by design.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { TYPESCRIPT {
    name = "typescript";
    lex = lex_typescript;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        PRIVATE = "private_property_identifier",
        NUMBER = "number",
        STRING = "string",
        TEMPLATE = "template_string",
        REGEX = "regex",
        /// A whole JSX element or fragment, in JavaScript and TSX.
        JSX = "jsx_element",
        /// A line break that ends a statement: after `break`, `continue`, `return`
        /// or `yield`, and after an operand when the next line starts with `[`,
        /// `++` or `--`. Elsewhere a line break is whitespace.
        LINE_END = "line_end",
        /// A string, template, regular expression or JSX element with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CLASS = "class",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DEBUGGER = "debugger",
        KW_DEFAULT = "default",
        KW_DELETE = "delete",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXPORT = "export",
        KW_EXTENDS = "extends",
        KW_FALSE = "false",
        KW_FINALLY = "finally",
        KW_FOR = "for",
        KW_FUNCTION = "function",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_IN = "in",
        KW_INSTANCEOF = "instanceof",
        KW_NEW = "new",
        KW_NULL = "null",
        KW_RETURN = "return",
        KW_SUPER = "super",
        KW_SWITCH = "switch",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPEOF = "typeof",
        KW_VAR = "var",
        KW_VOID = "void",
        KW_WHILE = "while",
        KW_WITH = "with",
        KW_LET = "let",
        KW_YIELD = "yield",
        KW_AWAIT = "await",
        /// Contextual keywords: names everywhere but where they mean something.
        KW_ASYNC = "async",
        KW_GET = "get",
        KW_SET = "set",
        KW_STATIC = "static",
        KW_OF = "of",
        KW_FROM = "from",
        KW_AS = "as",
        KW_TYPE = "type",
        KW_INTERFACE = "interface",
        KW_NAMESPACE = "namespace",
        KW_MODULE = "module",
        KW_DECLARE = "declare",
        KW_ABSTRACT = "abstract",
        KW_READONLY = "readonly",
        KW_PUBLIC = "public",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_IMPLEMENTS = "implements",
        KW_KEYOF = "keyof",
        KW_INFER = "infer",
        KW_IS = "is",
        KW_ASSERTS = "asserts",
        KW_UNIQUE = "unique",
        KW_SATISFIES = "satisfies",
        KW_OVERRIDE = "override",
        KW_ACCESSOR = "accessor",
        KW_GLOBAL = "global",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        ELLIPSIS = "...",
        OPTIONAL_CHAIN = "?.",
        QUESTION = "?",
        NULLISH = "??",
        COLON = ":",
        ARROW = "=>",
        AT = "@",
        ASSIGN = "=",
        EQ = "==",
        STRICT_EQ = "===",
        NE = "!=",
        STRICT_NE = "!==",
        LT = "<",
        LE = "<=",
        SHL = "<<",
        GT = ">",
        GE = ">=",
        /// A `>` right before another `>`: the first half of `>>`, `>>>`, `>>=`,
        /// or a type argument list closing right before another.
        GT_JOINED = ">joined",
        BANG = "!",
        TILDE = "~",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        STARSTAR = "**",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        ANDAND = "&&",
        OROR = "||",
        INCREMENT = "++",
        DECREMENT = "--",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        STARSTAR_EQ = "**=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        SHL_EQ = "<<=",
        ANDAND_EQ = "&&=",
        OROR_EQ = "||=",
        NULLISH_EQ = "??=",
    }

    program: _statement*,

    // ---- statements ----

    _statement:
        &_declaration_start _declaration
        | export_statement
        | import_statement
        | statement_block
        | if_statement
        | for_statement
        | while_statement
        | do_statement
        | try_statement
        | switch_statement
        | return_statement
        | break_statement
        | continue_statement
        | throw_statement
        | debugger_statement
        | with_statement
        | empty_statement
        | LINE_END
        | labeled_statement
        | expression_statement,

    /// What starts a declaration and nothing else: tried as a predicate.
    _declaration_start:
        "function" | "class" | "var" | "const" | "enum"
        | decorator+ ["abstract"] "class"
        | "let" (_name | array_pattern | object_pattern)
        | "async" "function"
        | "abstract" "class"
        | "type" _name
        | "interface" _name
        | ("namespace" | "module") (_name | STRING)
        | "declare" ("function" | "class" | "var" | "let" | "const" | "enum" | "type" | "interface" | "namespace" | "module" | "global" | "abstract" | "async"),

    _declaration:
        function_declaration
        | &("const" "enum") enum_declaration
        | &(decorator* ["abstract"] "class") class_declaration
        | lexical_declaration
        | variable_declaration
        | &("enum") enum_declaration
        | type_alias_declaration
        | interface_declaration
        | internal_module
        | ambient_declaration,

    expression_statement: _expressions_st [_semi],
    /// A statement's end: `;`, or a line break where it matters.
    _semi: ";" | LINE_END,
    empty_statement: ";",
    statement_block: "{" _statement* "}",
    labeled_statement: _name ":" _statement,
    if_statement: "if" parenthesized_expression _statement [else_clause],
    else_clause: "else" _statement,
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression [_semi],
    with_statement: "with" parenthesized_expression _statement,
    debugger_statement: "debugger" [_semi],
    return_statement: "return" [_expressions] [_semi],
    throw_statement: "throw" _expressions [_semi],
    break_statement: "break" [_name] [_semi],
    continue_statement: "continue" [_name] [_semi],

    for_statement: "for" ["await"] for_header _statement,
    for_header: "(" (&(_for_left ("in" | "of")) _for_in_of | _for_classic) ")",
    _for_left: ("var" | "let" | "const") _pattern | _expression@shift,
    _for_in_of: _for_left ("in" | "of") _expressions,
    _for_classic: [_for_init] ";" [_expressions] ";" [_expressions],
    _for_init: ("var" | "let" | "const") (variable_declarator % ",") | _expressions,

    try_statement: "try" statement_block [catch_clause] [finally_clause],
    catch_clause: "catch" [catch_parameter] statement_block,
    catch_parameter: "(" _pattern [type_annotation] ")",
    finally_clause: "finally" statement_block,

    switch_statement: "switch" parenthesized_expression switch_body,
    switch_body: "{" (switch_case | switch_default)* "}",
    switch_case: "case" _expressions ":" _statement*,
    switch_default: "default" ":" _statement*,

    variable_declaration: "var" (variable_declarator % ",") [_semi],
    lexical_declaration: ("let" | "const") (variable_declarator % ",") [_semi],
    variable_declarator: _pattern ["!"] [type_annotation] ["=" _expression@assign],

    // ---- modules ----

    import_statement: "import" ["type"] (STRING | &(_name "=") import_alias | import_clause "from" STRING) [import_attribute] [_semi],
    import_clause: namespace_import | named_imports | _name ["," (namespace_import | named_imports)],
    namespace_import: "*" "as" _name,
    named_imports: "{" (import_specifier %? ",") "}",
    import_specifier: &("type" _module_export_name) "type" _module_export_name ["as" _name] | _module_export_name ["as" _name],
    import_alias: _name "=" (&(_name arguments) import_require_clause | _entity_name),
    import_require_clause: _name arguments,
    import_attribute: "with" object,
    _module_export_name: _property_name | STRING,

    export_statement: decorator* "export" (
        "*" ["as" _module_export_name] "from" STRING [import_attribute] [_semi]
        | export_clause ["from" STRING [import_attribute]] [_semi]
        | "default" (&("interface" | "abstract" | "enum") _declaration | &(["async"] "function" ["*"] _name) function_declaration | _expression [_semi])
        | &("type" export_clause) "type" export_clause ["from" STRING] [_semi]
        | "=" _expression [_semi]
        | "as" "namespace" _name [_semi]
        | "import" import_alias [_semi]
        | _declaration
    ),
    export_clause: "{" (export_specifier %? ",") "}",
    export_specifier: &("type" _module_export_name) "type" _module_export_name ["as" _module_export_name] | _module_export_name ["as" _module_export_name],

    // ---- declarations ----

    function_declaration: ["async"] "function" ["*"] _name _signature [statement_block | ";"],
    class_declaration: decorator* ["abstract"] "class" _name [type_parameters] [LINE_END] [class_heritage] class_body,
    class_heritage: "extends" _expression@member [type_arguments] [implements_clause] | implements_clause,
    implements_clause: "implements" (_type % ","),
    class_body: "{" _class_member* "}",
    _class_member:
        &_method_head method_definition
        | &("static" statement_block) class_static_block
        | _semi
        | &(decorator* _modifier* index_signature_key) index_signature
        | public_field_definition,
    class_static_block: "static" statement_block,
    /// What a method starts with, up to its parameters.
    _method_head: decorator* _member_key_modded ["?" | "!"] (type_parameters | formal_parameters),
    method_definition: decorator* _member_key_modded ["?" | "!"] _signature [statement_block] [_semi],
    public_field_definition: decorator* _member_key_modded ["?" | "!"] [type_annotation] ["=" _expression@assign] [_semi],
    /// A member's name after its modifiers. A word is a modifier only if a name
    /// follows: `static get x() {}` has two, `get() {}` is a method named `get`.
    _member_key_modded: &(_modifier (_member_key | "*")) _modifier _member_key_modded | _member_key,
    _modifier:
        "static" | "async" | "get" | "set" | "*" | "public" | "private" | "protected" | "readonly"
        | "abstract" | "override" | "declare" | "accessor",
    _member_key: _property_name | PRIVATE | STRING | NUMBER | computed_property_name,
    computed_property_name: "[" _expression "]",
    decorator: "@" (_decorator_path | parenthesized_expression),
    _decorator_path: precedence _name {
        postfix "." _property_name => member_expression,
        postfix arguments => call_expression,
    },

    _signature: [type_parameters] formal_parameters [type_annotation_return],
    formal_parameters: "(" (_formal_parameter %? ",") ")",
    _formal_parameter: decorator* (rest_parameter | required_parameter),
    rest_parameter: "..." _pattern [type_annotation],
    required_parameter: _param_modded ["?"] [type_annotation] ["=" _expression@assign],
    _param_modded: &(_param_modifier (_pattern | _param_modifier)) _param_modifier _param_modded | _pattern | "this",
    _param_modifier: "public" | "private" | "protected" | "readonly" | "override",

    enum_declaration: ["const"] "enum" _name enum_body,
    enum_body: "{" (enum_assignment %? ",") "}",
    enum_assignment: (_property_name | STRING | NUMBER | computed_property_name) ["=" _expression@assign],

    type_alias_declaration: "type" _name [type_parameters] "=" _type [_semi],
    interface_declaration: "interface" _name [type_parameters] [LINE_END] [extends_type_clause] object_type,
    extends_type_clause: "extends" (_type % ","),
    internal_module: ("namespace" | "module") (_entity_name | STRING) [statement_block] [_semi],
    ambient_declaration: "declare" ("global" statement_block | _declaration),

    // ---- patterns ----

    _pattern: _name | object_pattern | array_pattern,
    object_pattern: "{" (_object_pattern_member %? ",") "}",
    _object_pattern_member:
        rest_pattern
        | &(_member_key ":") pair_pattern
        | object_assignment_pattern
