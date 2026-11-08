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
        | literal_type
        | TEMPLATE
        | "this"
        | "void"
        | "null"
        | "true"
        | "false"
        | _dynamic_import,
    function_type: [type_parameters] formal_parameters "=>" _return_type,
    constructor_type: ["abstract"] "new" [type_parameters] formal_parameters "=>" _type,
    parenthesized_type: "(" _type ")",
    tuple_type: "[" (_tuple_element %? ",") "]",
    _tuple_element: &(["..."] _name ["?"] ":") named_tuple_member | rest_type | _type ["?"],
    named_tuple_member: ["..."] _name ["?"] ":" _type,
    rest_type: "..." _type,
    type_query: "typeof" (_entity_name | _dynamic_import) [type_arguments],
    infer_type: "infer" _name,
    literal_type: STRING | NUMBER | "-" NUMBER,
    _type_path: precedence _type_name {
        postfix "." _name => nested_type_identifier,
        postfix type_arguments => generic_type,
    },
    _entity_name: _name ("." _name)*,

    object_type: "{" (_type_member [";" | "," | LINE_END])* "}",
    _type_member:
        &(["-" | "+"] _modifier* index_signature_key) index_signature
        | &(_member_key_modded ["?"] (type_parameters | formal_parameters)) method_signature
        | &("new" (type_parameters | formal_parameters)) construct_signature
        | call_signature
        | property_signature,
    index_signature: ["-" | "+"] _modifier* index_signature_key ["-" | "+"] ["?"] [type_annotation],
    index_signature_key: "[" _name (":" _type | "in" _type ["as" _type]) "]",
    method_signature: _member_key_modded ["?"] _signature,
    call_signature: [type_parameters] formal_parameters [type_annotation_return],
    construct_signature: "new" [type_parameters] formal_parameters [type_annotation],
    property_signature: _member_key_modded ["?"] [type_annotation],

    // ---- names ----

    /// A name that can be bound: identifiers and contextual keywords.
    _name:
        IDENT | "async" | "get" | "set" | "static" | "of" | "from" | "as" | "type" | "interface"
        | "namespace" | "module" | "declare" | "abstract" | "readonly" | "public" | "private"
        | "protected" | "implements" | "keyof" | "infer" | "is" | "asserts" | "unique"
        | "satisfies" | "override" | "accessor" | "global",
    /// A type's name: not the words that start a type operator.
    _type_name:
        IDENT | "async" | "get" | "set" | "static" | "of" | "from" | "as" | "type" | "interface"
        | "namespace" | "module" | "declare" | "public" | "private" | "protected" | "implements"
        | "satisfies" | "override" | "accessor" | "global",
    /// After `.` and as a key, every word is a name.
    _property_name:
        _name | "break" | "case" | "catch" | "class" | "const" | "continue" | "debugger" | "default"
        | "delete" | "do" | "else" | "enum" | "export" | "extends" | "false" | "finally" | "for"
        | "function" | "if" | "import" | "in" | "instanceof" | "new" | "null" | "return" | "super"
        | "switch" | "this" | "throw" | "true" | "try" | "typeof" | "var" | "void" | "while"
        | "with" | "let" | "yield" | "await",
} }

/// JavaScript: [`TYPESCRIPT`]'s grammar, with JSX.
pub static JAVASCRIPT: Language = Language::new("javascript", PROGRAM, KINDS, lex_jsx, parse_program);
/// TSX: [`TYPESCRIPT`]'s grammar, with JSX.
pub static TSX: Language = Language::new("tsx", PROGRAM, KINDS, lex_jsx, parse_program);

use super::syntax::Language;

/// One token at the cursor, for TypeScript: `<` is always an operator.
pub fn lex_typescript(c: &mut Cursor) -> Kind {
    lex(c, false)
}

/// One token at the cursor, for JavaScript and TSX: `<` where an operand
/// starts opens a JSX element.
pub fn lex_jsx(c: &mut Cursor) -> Kind {
    lex(c, true)
}

fn lex(c: &mut Cursor, jsx: bool) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c' => {
            let mut line_break = false;
            c.eat_while(|byte| {
                line_break |= byte == b'\n';
                matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
            });
            if line_break && ends_statement(c) { LINE_END } else { WHITESPACE }
        }
        // No-break space, byte order mark, line and paragraph separators.
        0xc2 if c.peek_at(1) == 0xa0 => {
            c.bump_n(2);
            WHITESPACE
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        0xe2 if c.peek_at(1) == 0x80 && matches!(c.peek_at(2), 0xa8 | 0xa9) => {
            c.bump_n(3);
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            block_comment(c);
            COMMENT
        }
        b'#' if c.pos() == 0 && c.peek_at(1) == b'!' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'#' if is_word_start(c.peek_at(1)) => {
            c.bump();
            c.eat_while(is_word_byte);
            PRIVATE
        }
        b'"' | b'\'' => {
            let quote = c.peek();
            c.bump();
            if c.quoted(quote, true) { STRING } else { BAD_LITERAL }
        }
        b'`' => {
            if template(c) { TEMPLATE } else { BAD_LITERAL }
        }
        b'/' if !ends_operand(c.previous()) => {
            if regex(c) { REGEX } else { BAD_LITERAL }
        }
        b'<' if jsx && !ends_operand(c.previous()) && c.previous() != KW_FUNCTION && (is_word_start(c.peek_at(1)) || c.peek_at(1) == b'>') && !generic_arrow(c.rest()) => {
            if jsx_element(c) { JSX } else { BAD_LITERAL }
        }
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() => number(c),
        byte if is_word_start(byte) => word(c),
        b'\\' if c.peek_at(1) == b'u' => word(c),
        _ => operator(c),
    }
}

/// After a line break: whether it ends the statement, as automatic semicolon
/// insertion has it for the restricted productions (`return` and a value on
/// the next line are two statements), and as TypeScript has it for a type
/// followed by a line that starts with `[` (an interface's next member) or
/// `extends` (a member named `extends`, not a conditional type).
fn ends_statement(c: &Cursor) -> bool {
    let previous = c.previous();
    if matches!(previous, KW_BREAK | KW_CONTINUE | KW_RETURN | KW_YIELD) {
        return true;
    }
    ends_operand(previous)
        && (c.peek() == b'[' || c.starts_with(b"++") || c.starts_with(b"--") || (c.starts_with(b"extends") && !is_word_byte(c.peek_at(7))))
}

/// Whether a token of this kind can end an operand, so that a `/` after it
/// divides and a `<` after it compares.
fn ends_operand(kind: Kind) -> bool {
    matches!(kind, IDENT | PRIVATE | NUMBER | STRING | TEMPLATE | REGEX | JSX | BAD_LITERAL | RPAREN | RBRACKET | RBRACE | KW_THIS | KW_SUPER | KW_TRUE | KW_FALSE | KW_NULL | INCREMENT | DECREMENT)
        || (KW_ASYNC..=KW_GLOBAL).contains(&kind)
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

/// `/* ... */`. An unclosed one runs to the end of the file.
fn block_comment(c: &mut Cursor) {
    c.bump_n(2);
    while !c.at_end() && !c.starts_with(b"*/") {
        c.bump();
    }
    c.bump_n(2);
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let mut length = 0;
    loop {
        let taken = c.eat_while(is_word_byte);
        length += taken;
        // `abc`: an escape in a name.
        if c.peek() == b'\\' && c.peek_at(1) == b'u' {
            c.bump_n(2);
            length += 2;
        } else {
            break;
        }
    }
    keyword(&start[..length])
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"catch" => KW_CATCH,
        b"class" => KW_CLASS,
        b"const" => KW_CONST,
        b"continue" => KW_CONTINUE,
        b"debugger" => KW_DEBUGGER,
        b"default" => KW_DEFAULT,
        b"delete" => KW_DELETE,
        b"do" => KW_DO,
        b"else" => KW_ELSE,
        b"enum" => KW_ENUM,
        b"export" => KW_EXPORT,
        b"extends" => KW_EXTENDS,
        b"false" => KW_FALSE,
        b"finally" => KW_FINALLY,
        b"for" => KW_FOR,
        b"function" => KW_FUNCTION,
        b"if" => KW_IF,
        b"import" => KW_IMPORT,
        b"in" => KW_IN,
        b"instanceof" => KW_INSTANCEOF,
        b"new" => KW_NEW,
        b"null" => KW_NULL,
        b"return" => KW_RETURN,
        b"super" => KW_SUPER,
        b"switch" => KW_SWITCH,
        b"this" => KW_THIS,
        b"throw" => KW_THROW,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"typeof" => KW_TYPEOF,
        b"var" => KW_VAR,
        b"void" => KW_VOID,
        b"while" => KW_WHILE,
        b"with" => KW_WITH,
        b"let" => KW_LET,
        b"yield" => KW_YIELD,
        b"await" => KW_AWAIT,
        b"async" => KW_ASYNC,
        b"get" => KW_GET,
        b"set" => KW_SET,
        b"static" => KW_STATIC,
        b"of" => KW_OF,
        b"from" => KW_FROM,
        b"as" => KW_AS,
        b"type" => KW_TYPE,
        b"interface" => KW_INTERFACE,
        b"namespace" => KW_NAMESPACE,
        b"module" => KW_MODULE,
        b"declare" => KW_DECLARE,
        b"abstract" => KW_ABSTRACT,
        b"readonly" => KW_READONLY,
        b"public" => KW_PUBLIC,
        b"private" => KW_PRIVATE,
        b"protected" => KW_PROTECTED,
        b"implements" => KW_IMPLEMENTS,
        b"keyof" => KW_KEYOF,
        b"infer" => KW_INFER,
        b"is" => KW_IS,
        b"asserts" => KW_ASSERTS,
        b"unique" => KW_UNIQUE,
        b"satisfies" => KW_SATISFIES,
        b"override" => KW_OVERRIDE,
        b"accessor" => KW_ACCESSOR,
        b"global" => KW_GLOBAL,
        _ => IDENT,
    }
}

/// Decimal, hex, octal and binary numbers, with `_` separators, fractions,
/// exponents and the BigInt `n`.
fn number(c: &mut Cursor) -> Kind {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_hexdigit() || byte == b'_');
        c.eat(b'n');
        return NUMBER;
    }
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.peek() == b'.' {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'e' | b'E') && (c.peek_at(1).is_ascii_digit() || (matches!(c.peek_at(1), b'+' | b'-') && c.peek_at(2).is_ascii_digit())) {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    c.eat(b'n');
    NUMBER
}

/// A template literal from its backtick: false if it never closes.
fn template(c: &mut Cursor) -> bool {
    c.bump();
    while !c.at_end() {
        match c.peek() {
            b'\\' => c.bump_n(2),
            b'`' => {
                c.bump();
                return true;
            }
            b'$' if c.peek_at(1) == b'{' => {
                c.bump();
                if !code_in_braces(c) {
                    return false;
                }
            }
            _ => c.bump(),
        }
    }
    false
}

/// Code from a `{` to its `}`, as in `${...}` and JSX's `{...}`, with strings,
/// templates, comments and JSX inside skipped whole. False if it never closes.
fn code_in_braces(c: &mut Cursor) -> bool {
    c.bump();
    let mut depth = 1u32;
    // Whether the last significant byte can end an operand: then `/` divides
    // and `<` compares, else they start a regular expression or JSX.
    let mut operand = false;
    while !c.at_end() {
        let byte = c.peek();
        match byte {
            b'{' => {
                depth += 1;
                c.bump();
                operand = false;
            }
            b'}' => {
                depth -= 1;
                c.bump();
                if depth == 0 {
                    return true;
                }
                operand = true;
            }
            b'"' | b'\'' => {
                c.bump();
                c.quoted(byte, true);
                operand = true;
            }
            b'`' => {
                template(c);
                operand = true;
            }
            b'/' if c.peek_at(1) == b'/' => {
                c.eat_while(|byte| byte != b'\n');
            }
            b'/' if c.peek_at(1) == b'*' => block_comment(c),
            b'/' if !operand => {
                regex(c);
                operand = true;
            }
            b'<' if !operand && (is_word_start(c.peek_at(1)) || c.peek_at(1) == b'>') => {
                jsx_element(c);
                operand = true;
            }
            b' ' | b'\t' | b'\n' | b'\r' => c.bump(),
            _ if is_word_byte(byte) => {
                let start = c.rest();
                let length = c.eat_while(is_word_byte);
                // After a keyword such as `return`, an operand starts.
                operand = !matches!(&start[..length], b"return" | b"typeof" | b"case" | b"do" | b"else" | b"in" | b"of" | b"new" | b"delete" | b"void" | b"throw" | b"yield" | b"await");
            }
            b')' | b']' => {
                c.bump();
                operand = true;
            }
            _ => {
                c.bump();
                operand = false;
            }
        }
    }
    false
}

/// A regular expression from its `/`: false if the line ends first.
fn regex(c: &mut Cursor) -> bool {
    c.bump();
    let mut class = false;
    loop {
        match c.peek() {
            0 if c.at_end() => return false,
            b'\n' | b'\r' => return false,
            b'\\' => c.bump_n(2),
            b'[' => {
                class = true;
                c.bump();
            }
            b']' => {
                class = false;
                c.bump();
            }
            b'/' if !class => {
                c.bump();
                c.eat_while(is_word_byte);
                return true;
            }
            _ => c.bump(),
        }
    }
}

/// `<T,>(` or `<T extends U>(`: a generic arrow function in TSX, not a tag.
fn generic_arrow(rest: &[u8]) -> bool {
    let mut at = 1;
    while at < rest.len() && is_word_byte(rest[at]) {
        at += 1;
    }
    while at < rest.len() && rest[at] == b' ' {
        at += 1;
    }
    rest.get(at) == Some(&b',') || rest[at.min(rest.len())..].starts_with(b"extends ")
}

/// A JSX element or fragment from its `<`, to the end of its closing tag:
/// tags nest, attributes and children may hold `{...}` code. False if it
/// never closes.
fn jsx_element(c: &mut Cursor) -> bool {
    let mut depth = 0u32;
    loop {
        // At a `<`: a tag.
        c.bump();
        let closing = c.eat(b'/');
        // Type arguments after a tag's name, `<Select<T | false> ...>`, nest.
        let mut angles = 0u32;
        // The tag: name, attributes, up to `>` or `/>`.
        loop {
            match c.peek() {
                0 if c.at_end() => return false,
                b'<' => {
                    angles += 1;
                    c.bump();
                }
                b'>' if angles > 0 => {
                    angles -= 1;
                    c.bump();
                }
                b'"' | b'\'' => {
                    let quote = c.peek();
                    c.bump();
                    c.quoted(quote, false);
                }
                b'{' => {
                    if !code_in_braces(c) {
                        return false;
                    }
                }
                b'/' if c.peek_at(1) == b'>' => {
                    c.bump_n(2);
                    if depth == 0 {
                        return true;
                    }
                    break;
                }
                b'>' => {
                    c.bump();
                    if closing {
                        depth -= 1;
                        if depth == 0 {
                            return true;
                        }
                    } else {
                        depth += 1;
                    }
                    break;
                }
                _ => c.bump(),
            }
        }
        // Children: text and `{...}` up to the next tag.
        loop {
            match c.peek() {
                0 if c.at_end() => return false,
                b'{' => {
                    if !code_in_braces(c) {
                        return false;
                    }
                }
                b'<' => break,
                _ => c.bump(),
            }
        }
    }
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
        (b'?', b'.', _) if !d.is_ascii_digit() => (OPTIONAL_CHAIN, 2),
        (b'?', b'?', b'=') => (NULLISH_EQ, 3),
        (b'?', b'?', _) => (NULLISH, 2),
        (b'?', _, _) => (QUESTION, 1),
        (b':', _, _) => (COLON, 1),
        (b'@', _, _) => (AT, 1),
        (b'=', b'>', _) => (ARROW, 2),
        (b'=', b'=', b'=') => (STRICT_EQ, 3),
        (b'=', b'=', _) => (EQ, 2),
        (b'=', _, _) => (ASSIGN, 1),
        (b'!', b'=', b'=') => (STRICT_NE, 3),
        (b'!', b'=', _) => (NE, 2),
        (b'!', _, _) => (BANG, 1),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'<', b'<', _) => (SHL, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'<', _, _) => (LT, 1),
        (b'>', b'>', _) => (GT_JOINED, 1),
        (b'>', b'=', _) => (GE, 2),
        (b'>', _, _) => (GT, 1),
        (b'~', _, _) => (TILDE, 1),
        (b'+', b'+', _) => (INCREMENT, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'+', _, _) => (PLUS, 1),
        (b'-', b'-', _) => (DECREMENT, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'-', _, _) => (MINUS, 1),
        (b'*', b'*', b'=') => (STARSTAR_EQ, 3),
        (b'*', b'*', _) => (STARSTAR, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'*', _, _) => (STAR, 1),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'/', _, _) => (SLASH, 1),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'%', _, _) => (PERCENT, 1),
        (b'&', b'&', b'=') => (ANDAND_EQ, 3),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'&', _, _) => (AMP, 1),
        (b'|', b'|', b'=') => (OROR_EQ, 3),
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
    use super::super::syntax::Language;
    use super::super::tests::{check, mutate, on_big_stack, Rng};
    use super::super::tree::HAS_ERROR;
    use super::*;

    fn parse(src: &str, language: &Language) -> Workspace {
        let mut workspace = Workspace::default();
        let outcome = workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
        assert!(outcome.complete(), "{src:?}: {}", outcome.describe());
        check(&workspace, src.as_bytes(), language);
        workspace
    }

    fn named(src: &str, language: &Language) -> String {
        parse(src, language).tree.sexp(language, true)
    }

    fn clean(src: &str, language: &Language) -> bool {
        parse(src, language).tree.root().flags & HAS_ERROR == 0
    }

    /// The expression in the statement `SRC;`.
    fn expression(src: &str, language: &Language) -> String {
        let tree = named(&format!("{src};"), language);
        tree.strip_prefix("(program (expression_statement ").and_then(|rest| rest.strip_suffix("))")).unwrap_or_else(|| panic!("{src}: {tree}")).to_string()
    }

    /// The significant tokens' kinds.
    fn kinds(src: &str, language: &Language) -> Vec<&'static str> {
        let workspace = parse(src, language);
        (0..workspace.tokens.len())
            .filter(|&at| !language.is_trivia(workspace.tokens.kind[at]))
            .map(|at| language.name(workspace.tokens.kind[at]))
            .collect()
    }

    #[test]
    fn an_error_stays_inside_its_construct() {
        // A bad declaration is an error inside the function's block, and the
        // class after it survives whole.
        assert_eq!(
            named("function f() { let = 1; }\nclass C {}\n", &JAVASCRIPT),
            "(program (function_declaration function identifier (formal_parameters) (statement_block (ERROR let) (ERROR) (expression_statement number))) (class_declaration class identifier (class_body)))"
        );
        // An unclosed parenthesis costs its own `if`, not the next method or statement.
        let tree = named("class A { m() { if (x { y(); } } n() {} }\nf();\n", &JAVASCRIPT);
        assert!(tree.contains("(parenthesized_expression UNCLOSED identifier (ERROR"), "{tree}");
        assert!(tree.ends_with("(method_definition identifier (formal_parameters) (statement_block)))) (expression_statement (call_expression identifier (arguments))))"), "{tree}");
        // A bad token inside an argument list stays in that list.
        assert_eq!(
            named("f(1, #, 2); g();\n", &JAVASCRIPT),
            "(program (expression_statement (call_expression identifier (arguments number (ERROR UNKNOWN) number))) (expression_statement (call_expression identifier (arguments))))"
        );
        // A missing value is one pair's error; the other pairs and the next statement stand.
        assert_eq!(
            named("let o = { a: 1, b: , c: 3 };\nh();\n", &JAVASCRIPT),
            "(program (lexical_declaration let (variable_declarator identifier (object (pair identifier number) (pair identifier (ERROR MISSING)) (pair identifier number)))) (expression_statement (call_expression identifier (arguments))))"
        );
        assert_eq!(
            named("interface I { a: ; b: string }\ntype T = number;\n", &TYPESCRIPT),
            "(program (interface_declaration interface identifier (object_type (property_signature identifier (type_annotation (ERROR MISSING))) (property_signature identifier (type_annotation identifier)))) (type_alias_declaration type identifier identifier))"
        );
    }

    #[test]
    fn operators_nest_by_precedence() {
        let cases = [
            ("a + b * c", "(binary_expression identifier (binary_expression identifier identifier))"),
            ("a * b + c", "(binary_expression (binary_expression identifier identifier) identifier)"),
            ("a - b - c", "(binary_expression (binary_expression identifier identifier) identifier)"),
            ("a ** b ** c", "(binary_expression identifier (binary_expression identifier identifier))"),
            ("a ?? b || c", "(binary_expression identifier (binary_expression identifier identifier))"),
            ("a || b && c", "(binary_expression identifier (binary_expression identifier identifier))"),
            ("a >> b > c", "(binary_expression (binary_expression identifier identifier) identifier)"),
            ("x = y = z", "(assignment_expression identifier (assignment_expression identifier identifier))"),
            ("a ? b : c ? d : e", "(ternary_expression identifier identifier (ternary_expression identifier identifier identifier))"),
            ("(a, b) => a + b", "(arrow_function (formal_parameters (required_parameter identifier) (required_parameter identifier)) (binary_expression identifier identifier))"),
            ("async x => await x", "(arrow_function async identifier (await_expression await identifier))"),
            ("f?.(x)?.[y]?.z", "(member_expression (subscript_expression (call_expression identifier (arguments identifier)) (subscript identifier)) identifier)"),
            ("new Foo(1).bar", "(member_expression (new_expression new identifier (arguments number)) identifier)"),
            ("x = /re/g.test(s) / 2", "(assignment_expression identifier (binary_expression (call_expression (member_expression regex identifier) (arguments identifier)) number))"),
            ("!a instanceof B", "(binary_expression (unary_expression identifier) instanceof identifier)"),
            ("a, b = c", "(sequence_expression identifier (assignment_expression identifier identifier))"),
        ];
        for (src, want) in cases {
            assert_eq!(expression(src, &JAVASCRIPT), want, "{src:?}");
        }
    }

    #[test]
    fn typescript_tells_generics_from_comparisons() {
        let cases = [
            ("f<T>(x)", "(call_expression identifier (type_arguments identifier) (arguments identifier))"),
            ("a < b && c > d", "(binary_expression (binary_expression identifier identifier) (binary_expression identifier identifier))"),
            ("<T,>(x: T) => x", "(arrow_function (type_parameters (type_parameter identifier)) (formal_parameters (required_parameter identifier (type_annotation identifier))) identifier)"),
            ("x as unknown as T", "(as_expression (as_expression identifier as identifier) as identifier)"),
            ("x!.y", "(member_expression (non_null_expression identifier) identifier)"),
        ];
        for (src, want) in cases {
            assert_eq!(expression(src, &TYPESCRIPT), want, "{src:?}");
        }
        assert!(clean("let m: Map<string, Set<number>> = new Map();\ntype F = <T>(x: T) => T extends string ? 1 : 2;\n", &TYPESCRIPT));
        assert!(clean("type K = keyof typeof o;\ntype L = T[number]['a'];\ntype N = import('x').Y<Z>;\nlet v = <T>x;\n", &TYPESCRIPT));
    }

    #[test]
    fn declarations_classes_and_modules() {
        let ts = &TYPESCRIPT;
        assert!(clean("export default class A<T> extends B<T> implements I, J {\n  private readonly x: number = 1;\n  static #y?: string;\n  constructor(public a: T, protected b = 2) { super(); }\n  get z() { return this.x }\n  set z(v) {}\n  async *gen() { yield* g(); }\n  static { init(); }\n  [key: string]: unknown;\n  abstract m(): void;\n  declare d: D;\n}\n", ts));
        assert!(clean("import x, { a as b, type C } from './m';\nimport * as ns from 'n';\nimport type { T } from 't';\nimport fs = require('fs');\nexport { a, b as c } from 'd';\nexport * from 'e';\nexport = x;\nexport type { T };\n", ts));
        assert!(clean("declare module 'm' { export function f(): void; }\nnamespace N.M { export const x = 1; }\ndeclare global { interface Window { a: 1 } }\nconst enum E { A = 1, B }\nenum F { C }\nabstract class G {}\n", ts));
        assert!(clean("function f(this: Window, ...rest: number[]): asserts x is string {}\nfunction g<T extends object = {}>(x?: T): x is T;\nlet { a, b: [c, ...d] = [], ...e } = o;\n", ts));
        assert!(clean("label: for (const [k, v] of m) { if (k) continue label; else break }\nfor (let i = 0, j; i < n; i++) {}\nfor (x in o);\nfor await (const x of it) {}\ndo x++; while (x < 3)\nswitch (a) { case 1: case 2: f(); break; default: }\ntry { t() } catch { } finally { }\ntry {} catch ({ message }) {}\n", ts));
        // A contextual keyword is still a name where a declaration cannot start.
        assert!(clean("let type = 1, async = 2, get = 3, of = [];\ntype(of);\nasync(get);\nlet o = { get, set: 1, async() {}, get a() { return 1 }, static: 2 };\n", ts));
    }

    #[test]
    fn line_ends_end_statements_where_asi_does() {
        assert_eq!(
            named("a\n++b\nlet x = 1\n[1, 2].map(f)\nreturn\n", &JAVASCRIPT),
            "(program (expression_statement identifier line_end) (expression_statement (update_expression identifier)) (lexical_declaration let (variable_declarator identifier number) line_end) (expression_statement (call_expression (member_expression (array number number) identifier) (arguments identifier))) (return_statement return line_end))"
        );
        // A line break inside an expression goes on with it.
        assert_eq!(
            named("x = a\n  + b\n  .c()\n", &JAVASCRIPT),
            "(program (expression_statement (assignment_expression identifier (binary_expression identifier (call_expression (member_expression identifier identifier) (arguments))))))"
        );
        // `break` then a name on the next line is not a label.
        assert!(named("while (1) { break\nfoo() }\n", &JAVASCRIPT).contains("(break_statement break line_end) (expression_statement (call_expression identifier (arguments)))"));
    }

    #[test]
    fn lexing_regexes_templates_and_jsx() {
        assert_eq!(kinds("a / b / c", &JAVASCRIPT), ["identifier", "/", "identifier", "/", "identifier"]);
        assert_eq!(kinds("x = /a[/]b/g; y = !/c/.test(s)", &JAVASCRIPT), ["identifier", "=", "regex", ";", "identifier", "=", "!", "regex", ".", "identifier", "(", "identifier", ")"]);
        assert_eq!(kinds("`a ${ {b: `c${d}`}.b } e` + 1", &JAVASCRIPT), ["template_string", "+", "number"]);
        assert_eq!(kinds("<div a=\"}\" b={c > d}>{'}'}<br/>{x && <i>{`${y}`}</i>}</div>;", &JAVASCRIPT), ["jsx_element", ";"]);
        assert_eq!(kinds("<T>x", &TYPESCRIPT), ["<", "identifier", ">", "identifier"]);
        assert_eq!(kinds("<Select<T | false> v={1} />", &TSX), ["jsx_element"]);
        assert_eq!(kinds("1_000n 0x1F .5e-3 1.", &JAVASCRIPT), ["number", "number", "number", "number"]);
        assert_eq!(kinds("#p 'it\\'s' \"q\"", &JAVASCRIPT), ["private_property_identifier", "string", "string"]);
    }

    /// Malformed inputs: each must report an error, and keep its root.
    #[test]
    fn malformed_inputs_report_errors() {
        let bad: [(&str, &Language); 18] = [
            ("function f( {}\n", &JAVASCRIPT),
            ("let x = ;\n", &JAVASCRIPT),
            ("if (a) else b;\n", &JAVASCRIPT),
            ("f(a b);\n", &JAVASCRIPT),
            ("let o = { a: 1 b: 2 };\n", &JAVASCRIPT),
            ("class { }\n", &JAVASCRIPT),
            ("x.;\n", &JAVASCRIPT),
            ("a + ;\n", &JAVASCRIPT),
            ("for (let i = 0; i < n) {}\n", &JAVASCRIPT),
            ("let s = 'unterminated;\nf();\n", &JAVASCRIPT),
            ("let t = `open ${ x;\n", &JAVASCRIPT),
            ("import { a from 'b';\n", &JAVASCRIPT),
            ("switch (x) { foo: 1 }\n", &JAVASCRIPT),
            ("let e = <div>;\n", &JAVASCRIPT),
            ("let x: = 1;\n", &TYPESCRIPT),
            ("interface I extends { }\n", &TYPESCRIPT),
            ("type T = A | ;\n", &TYPESCRIPT),
            ("function f<T(x: T) {}\n", &TSX),
        ];
        for (src, language) in bad {
            let workspace = parse(src, language);
            assert!(workspace.tree.root().flags & HAS_ERROR != 0, "{src:?} parsed clean: {}", workspace.tree.sexp(language, true));
            assert_eq!(workspace.tree.root().kind, PROGRAM);
        }
    }

    #[test]
    fn deep_input_stays_within_the_budget() {
        on_big_stack(|| {
            let deep = [
                format!("x = {}1;", "(".repeat(100_000)),
                format!("x = {}y;", "-".repeat(100_000)),
                format!("x = {}y;", "!".repeat(100_000)),
                format!("x = {}b;", "a + ".repeat(100_000)),
                format!("x = {}y;", "a => ".repeat(50_000)),
                format!("x = {}0;", "a ? b : ".repeat(50_000)),
                format!("x = {}1{};", "[".repeat(50_000), "]".repeat(50_000)),
                format!("let x: {}T{};", "Array<".repeat(50_000), ">".repeat(50_000)),
                format!("{}", "if (a) ".repeat(50_000) + "b;"),
                format!("x = `{}`;", "${`".repeat(20_000)),
                "{".repeat(100_000),
            ];
            for src in deep {
                for language in [&JAVASCRIPT, &TYPESCRIPT] {
                    let mut workspace = Workspace::default();
                    let outcome = workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
                    assert!(outcome.exhausted.is_none(), "{}: {}", &src[..20], outcome.describe());
                    check(&workspace, src.as_bytes(), language);
                    assert_eq!(workspace.tree.root().kind, PROGRAM);
                }
            }
        });
    }

    const SEEDS: &[&str] = &[
        "import { readFile } from 'fs/promises';\n\nexport class Cache {\n  #store = new Map();\n  constructor(limit = 100) { this.limit = limit }\n  async get(key) {\n    if (!this.#store.has(key)) {\n      const text = await readFile(key, 'utf8');\n      this.#store.set(key, JSON.parse(text));\n    }\n    return this.#store.get(key) ?? null\n  }\n}\n",
        "const sum = (xs) => xs.reduce((a, b) => a + b, 0)\nlet total = 0\nfor (const [i, x] of items.entries()) {\n  total += x.price * (i % 2 ? 1 : 0.5)\n  if (/^sale/i.test(x.name)) continue\n}\nconsole.log(`total: ${sum([total, 1])}`)\n",
        "export interface Node<T = unknown> {\n  kind: 'leaf' | 'branch';\n  children?: readonly Node<T>[];\n  value: T extends string ? number : T;\n}\n\nexport function walk<T>(node: Node<T>, visit: (n: Node<T>) => void): void {\n  visit(node);\n  for (const child of node.children ?? []) walk(child, visit);\n}\n",
        "export const App = ({ items }: Props) => {\n  const [open, setOpen] = useState<boolean>(false);\n  return (\n    <ul className=\"list\">\n      {items.map((x) => <li key={x.id} onClick={() => setOpen(!open)}>{x.label}</li>)}\n    </ul>\n  );\n};\n",
    ];

    const JS_BYTES: &[u8] = b"()[]{}<>:;,.=+-*/%!&|^~?@#$'\"`\\_0123456789 \n\t xyfunctionletconstclassreturnifelseimportexporttypeinterfaceasyncawait=>${}\xc3\xa9";

    #[test]
    fn fuzzed_javascript_terminates_and_keeps_every_invariant() {
        on_big_stack(|| {
            let mut rng = Rng(0x5910_0004_006a_73ff);
            let mut workspace = Workspace::default();
            let budget = Budget::default();
            let mut erroneous = 0;
            const ROUNDS: usize = 3000;
            for round in 0..ROUNDS {
                let language: &Language = [&JAVASCRIPT, &TYPESCRIPT, &TSX][round % 3];
                let mut src = SEEDS[round % SEEDS.len()].as_bytes().to_vec();
                for _ in 0..1 + rng.below(6) {
                    let length = src.len();
                    mutate(&mut rng, &mut src, JS_BYTES, b"", 0, length);
                }
                let outcome = workspace.parse(&src, language, &budget).unwrap();
                assert!(outcome.exhausted.is_none(), "budget ran out on {:?}", String::from_utf8_lossy(&src));
                check(&workspace, &src, language);
                assert_eq!(workspace.tree.root().kind, PROGRAM);
                if workspace.tree.root().flags & HAS_ERROR != 0 {
                    erroneous += 1;
                }
            }
            assert!(erroneous > ROUNDS / 3, "{erroneous} of {ROUNDS}");
        });
        assert!(clean(SEEDS[0], &JAVASCRIPT) && clean(SEEDS[1], &JAVASCRIPT));
        assert!(clean(SEEDS[2], &TYPESCRIPT));
        assert!(clean(SEEDS[3], &TSX));
    }
}
