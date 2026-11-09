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

use super::lexer::Cursor;
use super::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

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
        | _name,
    pair_pattern: _member_key ":" _binding_element,
    object_assignment_pattern: _name "=" _expression@assign,
    array_pattern: "[" (_array_pattern_item [","] | ",")* "]",
    _array_pattern_item: rest_pattern | _binding_element,
    _binding_element: precedence _pattern {
        postfix "=" _expression@assign => assignment_pattern,
    },
    rest_pattern: "..." _pattern,

    // ---- expressions ----

    _expressions: precedence _expression {
        left "," => sequence_expression,
    },
    _expressions_st: precedence _expression_st {
        postfix "," _expression@assign => sequence_expression,
    },

    _expression: precedence _primary {
        assign: right "=" => assignment_expression,
        right "+=" | "-=" | "*=" | "**=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" | GT_JOINED GT_JOINED ">=" | "&&=" | "||=" | "??=" => augmented_assignment_expression,
        postfix "=>" _arrow_body => arrow_function,
        postfix "?" _expression@assign ":" _expression@assign => ternary_expression,
        nullish: left "??" => binary_expression,
        or: left "||" => binary_expression,
        and: left "&&" => binary_expression,
        bor: left "|" => binary_expression,
        xor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        eq: left "==" | "!=" | "===" | "!==" => binary_expression,
        rel: left "<" | ">" | "<=" | ">=" | "instanceof" | "in" => binary_expression,
        postfix "as" ("const" | _type) => as_expression,
        postfix "satisfies" _type => satisfies_expression,
        shift: left "<<" | GT_JOINED ">" | GT_JOINED GT_JOINED ">" => binary_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        exp: right "**" => binary_expression,
        unary: prefix "!" | "~" | "-" | "+" | "typeof" | "void" | "delete" => unary_expression,
        prefix "await" => await_expression,
        prefix "++" | "--" => update_expression,
        postfix "++" | "--" => update_expression,
        member: postfix "!" => non_null_expression,
        postfix arguments => call_expression,
        postfix TEMPLATE => call_expression,
        postfix &(type_arguments (arguments | TEMPLATE)) type_arguments (arguments | TEMPLATE) => call_expression,
        postfix "." (_property_name | PRIVATE) => member_expression,
        postfix "?." (_property_name | PRIVATE) => member_expression,
        postfix "?." arguments => call_expression,
        postfix "?." subscript => subscript_expression,
        postfix subscript => subscript_expression,
    },

    /// `_expression` where a statement starts: no `{`, `function` or `class`
    /// first. Right operands are full expressions.
    _expression_st: precedence _primary_st {
        postfix "=" _expression@assign => assignment_expression,
        postfix ("+=" | "-=" | "*=" | "**=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" | GT_JOINED GT_JOINED ">=" | "&&=" | "||=" | "??=") _expression@assign => augmented_assignment_expression,
        postfix "=>" _arrow_body => arrow_function,
        postfix "?" _expression@assign ":" _expression@assign => ternary_expression,
        postfix "??" _expression@or => binary_expression,
        postfix "||" _expression@and => binary_expression,
        postfix "&&" _expression@bor => binary_expression,
        postfix "|" _expression@xor => binary_expression,
        postfix "^" _expression@band => binary_expression,
        postfix "&" _expression@eq => binary_expression,
        postfix ("==" | "!=" | "===" | "!==") _expression@rel => binary_expression,
        postfix ("<" | ">" | "<=" | ">=" | "instanceof" | "in") _expression@shift => binary_expression,
        postfix "as" ("const" | _type) => as_expression,
        postfix "satisfies" _type => satisfies_expression,
        postfix ("<<" | GT_JOINED ">" | GT_JOINED GT_JOINED ">") _expression@add => binary_expression,
        postfix ("+" | "-") _expression@mul => binary_expression,
        postfix ("*" | "/" | "%") _expression@exp => binary_expression,
        postfix "**" _expression@exp => binary_expression,
        prefix "!" | "~" | "-" | "+" | "typeof" | "void" | "delete" => unary_expression,
        prefix "await" => await_expression,
        prefix "++" | "--" => update_expression,
        postfix "++" | "--" => update_expression,
        postfix "!" => non_null_expression,
        postfix arguments => call_expression,
        postfix TEMPLATE => call_expression,
        postfix &(type_arguments (arguments | TEMPLATE)) type_arguments (arguments | TEMPLATE) => call_expression,
        postfix "." (_property_name | PRIVATE) => member_expression,
        postfix "?." (_property_name | PRIVATE) => member_expression,
        postfix "?." arguments => call_expression,
        postfix "?." subscript => subscript_expression,
        postfix subscript => subscript_expression,
    },

    _primary: &("async" "function") function_expression | &("function") function_expression | _primary_st | object | class,
    _primary_st:
        &_arrow_head _arrow_parameters
        | _name
        | PRIVATE
        | NUMBER
        | STRING
        | TEMPLATE
        | REGEX
        | JSX
        | BAD_LITERAL
        | "this"
        | "super"
        | "true"
        | "false"
        | "null"
        | parenthesized_expression
        | array
        | new_expression
        | meta_property
        | yield_expression
        | type_assertion
        | _dynamic_import,
    /// `<T>x`, in TypeScript; JavaScript and TSX read `<T>` as JSX.
    type_assertion: type_arguments _expression@unary,
    _dynamic_import: "import" arguments,
    /// An arrow function's head, up to `=>`: its parameters are an operand,
    /// and the `=>` row of the expression tables makes the `arrow_function`.
    /// A lone name before `=>` is an operand already.
    _arrow_head: "async" (_name | _signature) "=>" | _signature "=>",
    _arrow_parameters: "async" (_name | _signature) | _signature,
    /// A `{` after `=>` opens a block: the expression form starts with no `{`.
    _arrow_body: statement_block | &("async" "function") function_expression | &("function") function_expression | class | _expression_st,
    function_expression: ["async"] "function" ["*"] [_name] _signature statement_block,
    class: decorator* "class" [_name] [type_parameters] [LINE_END] [class_heritage] class_body,
    parenthesized_expression: "(" _expressions ")",
    array: "[" (_array_item [","] | ",")* "]",
    _array_item: spread_element | _expression@assign,
    spread_element: "..." _expression@assign,
    object: "{" (_object_member %? ",") "}",
    _object_member:
        spread_element
        | &_method_head method_definition
        | _pair_like,
    _pair_like: precedence _member_key {
        postfix ":" _expression@assign => pair,
        postfix "=" _expression@assign => assignment_pattern,
    },
    arguments: "(" (_argument %? ",") ")",
    _argument: spread_element | _expression@assign,
    subscript: "[" _expressions "]",
    new_expression: "new" _new_callee [type_arguments] [arguments],
    _new_callee: precedence _new_target {
        postfix "." (_property_name | PRIVATE) => member_expression,
        postfix subscript => subscript_expression,
    },
    _new_target: &("function") function_expression | _name | "this" | "super" | parenthesized_expression | new_expression | class | array | _dynamic_import,
    meta_property: "new" "." _name | "import" "." _name,
    yield_expression: "yield" ["*"] [_expression@assign],

    // ---- types ----

    type_annotation: ":" _type,
    type_annotation_return: ":" _return_type,
    _return_type: &("asserts" (_name | "this")) asserts | &((_name | "this") "is") type_predicate | _type,
    asserts: "asserts" (_name | "this") ["is" _type],
    type_predicate: (_name | "this") "is" _type,

    type_parameters: "<" (type_parameter %? ",") _gt,
    type_parameter: ["const"] ["in"] _name ["extends" _type] ["=" _type],
    type_arguments: "<" (_type %? ",") _gt,
    _gt: ">" | GT_JOINED,

    _type: precedence _primary_type {
        cond: postfix "extends" _type@union "?" _type ":" _type => conditional_type,
        union: left "|" => union_type,
        prefix "|" => union_type,
        inter: left "&" => intersection_type,
        prefix "&" => intersection_type,
        prefix "keyof" => index_type_query,
        prefix "readonly" => readonly_type,
        prefix "unique" => unique_type,
        postfix type_subscript => lookup_type,
        postfix "." _type_name => nested_type_identifier,
        postfix type_arguments => generic_type,
    },
    type_subscript: "[" [_type] "]",
    _primary_type:
        &([type_parameters] formal_parameters "=>") function_type
        | &(["abstract"] "new") constructor_type
        | _type_path
        | parenthesized_type
        | object_type
        | tuple_type
        | type_query
        | infer_type
