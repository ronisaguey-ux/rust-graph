//! D: modules and imports, aggregate types (`struct`, `class`, `interface`,
//! `union`), `enum`, `template`, `alias`, functions and the statements and
//! expressions a security scanner looks at (calls, assignments, control forms).
//!
//! The shape is Tree-sitter's D where that costs nothing; the tree is not the
//! same tree.
//!
//! # What is modelled
//!
//! - **Top level.** `module a.b;`, `import x : y, z;` (with an alias),
//!   aggregates with bases and members, `enum` (an aggregate, a manifest
//!   constant), `template`, `alias = ...`, functions with attributes and a body
//!   (`in`/`out`/`body` are not), and variable declarations.
//! - **Statements.** Blocks, `if`/`else`, `while`, `do`/`while`, `for`,
//!   `foreach`/`foreach_reverse`, `switch`/`case`/`default`/`final switch`,
//!   `try`/`catch`/`finally`, `throw`, `scope(exit)`, `return`, `break`,
//!   `continue`, `goto`, `with`, `synchronized`, `assert`.
//! - **Expressions.** One precedence table down to calls, indexing, member
//!   access and template-free postfixes; `~` is concatenation, `is`/`!is` and
//!   `in`/`!in` are comparisons, `>>>` is a shift, and `? :` is a row.
//! - **The lexer** knows `//`, `/* */`, nested `/+ +/`, ordinary strings with
//!   escapes, `r"..."`, `` `...` ``, `q{...}` and the `x"..."` hex form, plus
//!   `0x`/`0b`/`0o` numbers with `_` and type suffixes.
//!
//! # What is missing, named here rather than accepted silently
//!
//! - **A user-defined type needs a builtin qualifier to declare by.** `Foo x;`
//!   is read as two expressions, because deciding it against `foo(1);` needs a
//!   symbol table; `immutable Foo x;` and every builtin type work.
//! - **`!` template instantiation** (`std.algorithm.map!(a => a)`) is not a
//!   postfix: `!` is only the not-operator and `!is`/`!in`.
//! - **Lambda and delegate literals** (`(a) => a`, `delegate int (int x)`) are
//!   not modelled; a delegate *type* is.
//! - **Attributes** (`@safe`, `@nogc`, `pragma(...)`, `version (...)`) are
//!   `@`/keyword plus a call at best, not a declaration prefix; only the
//!   builtin storage classes and `pure`/`nothrow`/`override` are.
//! - **`in`/`out` contract blocks** and `unittest` are not statements here.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { D {
    name = "d";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,

        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        CHAR_LITERAL = "char_literal",
        BAD_LITERAL = "bad_literal" bad,

        KW_ABSTRACT = "abstract",
        KW_ALIAS = "alias",
        KW_ALIGN = "align",
        KW_ASM = "asm",
        KW_ASSERT = "assert",
        KW_AUTO = "auto",
        KW_BODY = "body",
        KW_BOOL = "bool",
        KW_BREAK = "break",
        KW_BYTE = "byte",
        KW_CASE = "case",
        KW_CAST = "cast",
        KW_CATCH = "catch",
        KW_CDOUBLE = "cdouble",
        KW_CFLOAT = "cfloat",
        KW_CHAR = "char",
        KW_CLASS = "class",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_CREAL = "creal",
        KW_DCHAR = "dchar",
        KW_DEBUG = "debug",
        KW_DEFAULT = "default",
        KW_DELEGATE = "delegate",
        KW_DELETE = "delete",
        KW_DO = "do",
        KW_DOUBLE = "double",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXPORT = "export",
        KW_EXTERN = "extern",
        KW_FALSE = "false",
        KW_FINAL = "final",
        KW_FINALLY = "finally",
        KW_FLOAT = "float",
        KW_FOR = "for",
        KW_FOREACH = "foreach",
        KW_FOREACH_REVERSE = "foreach_reverse",
        KW_FUNCTION = "function",
        KW_GOTO = "goto",
        KW_IDOUBLE = "idouble",
        KW_IF = "if",
        KW_IFLOAT = "ifloat",
        KW_IMMUTABLE = "immutable",
        KW_IMPORT = "import",
        KW_IN = "in",
        KW_INOUT = "inout",
        KW_INT = "int",
        KW_INTERFACE = "interface",
        KW_INVARIANT = "invariant",
        KW_IREAL = "ireal",
        KW_IS = "is",
        KW_LAZY = "lazy",
        KW_LONG = "long",
        KW_MIXIN = "mixin",
        KW_MODULE = "module",
        KW_NEW = "new",
        KW_NOTHROW = "nothrow",
        KW_NULL = "null",
        KW_OUT = "out",
        KW_OVERRIDE = "override",
        KW_PACKAGE = "package",
        KW_PRAGMA = "pragma",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_PUBLIC = "public",
        KW_PURE = "pure",
        KW_REAL = "real",
        KW_REF = "ref",
        KW_RETURN = "return",
        KW_SCOPE = "scope",
        KW_SHARED = "shared",
        KW_SHORT = "short",
        KW_STATIC = "static",
        KW_STRUCT = "struct",
        KW_SUPER = "super",
        KW_SWITCH = "switch",
        KW_SYNCHRONIZED = "synchronized",
        KW_TEMPLATE = "template",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPEID = "typeid",
        KW_TYPEOF = "typeof",
        KW_UBYTE = "ubyte",
        KW_UCENT = "ucent",
        KW_UINT = "uint",
        KW_ULONG = "ulong",
        KW_UNION = "union",
        KW_UNITTEST = "unittest",
        KW_USHORT = "ushort",
        KW_VERSION = "version",
        KW_VOID = "void",
        KW_WCHAR = "wchar",
        KW_WHILE = "while",
        KW_WITH = "with",

        COMMA = ",",
        SEMI = ";",
        DOT = ".",
        DOTDOT = "..",
        ELLIPSIS = "...",
        COLON = ":",
        QUESTION = "?",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        ANDAND = "&&",
        OROR = "||",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
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
        TILDE_EQ = "~=",
        ANDAND_EQ = "&&=",
        OROR_EQ = "||=",
        SHL = "<<",
        SHR = ">>",
        USHR = ">>>",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        USHR_EQ = ">>>=",
        AT = "@",
    }

    program: _top_item*,

    _top_item:
        module_declaration
        | import_declaration
        | &(_type declarator_name parameters) function_definition
        | aggregate_definition
        | enum_definition
        | template_definition
        | alias_declaration
        | _statement,

    module_declaration: "module" module_path ";",
    module_path: IDENT ("." IDENT)*,
    import_declaration: ["static"] "import" import_clause (";" | imported_symbols ";"),
    import_clause: [IDENT "="] module_path,
    imported_symbols: ":" (_imported %? ","),
    _imported: imported_symbol_binding | IDENT,
    imported_symbol_binding: IDENT "=" IDENT,

    // ---- definitions ----

    aggregate_definition: aggregate_kind [IDENT] [template_parameters] [aggregate_bases] aggregate_body,
    aggregate_kind: "struct" | "class" | "interface" | "union",
    template_parameters: "(" (_template_parameter %? ",") ")",
    _template_parameter: ["alias"] ["ref" | "out" | "lazy"] IDENT ["=" _expression@assign | ":" _type],
    aggregate_bases: ":" (_type % ","),
    aggregate_body: "{" _aggregate_member* "}",
    _aggregate_member:
        member_attribute
        | invariant_block
        | unit_test_block
        | constructor_definition
        | destructor_definition
        | &(_type declarator_name parameters) function_definition
        | aggregate_definition
        | enum_definition
        | template_definition
        | alias_declaration
        | &(declaration_statement) declaration_statement,
    member_attribute: _visibility ":",
    _visibility: "public" | "private" | "protected" | "package" | "export" | "static" | "final" | "abstract" | "override" | "immutable" | "const" | "shared" | "synchronized",
    invariant_block: "invariant" member_body,
    unit_test_block: "unittest" member_body,
    constructor_definition: "this" parameters [member_body],
    destructor_definition: "~" "this" empty_parameters member_body,
    empty_parameters: "(" ")",
    member_body: "{" _statement* "}",

    enum_definition: "enum" [IDENT] [":" _type] enum_tail,
    enum_tail: enum_body | "=" _expression@assign ";",
    enum_body: "{" (_enum_member %? ",") "}",
    _enum_member: IDENT ["=" _expression@assign],

    template_definition: ["mixin"] "template" IDENT template_parameters aggregate_body,

    alias_declaration: "alias" alias_target (";" | "," ),
    alias_target: IDENT "=" _type | _type IDENT,

    function_definition: _type function_name parameters function_attribute* [member_body],
    function_attribute: "pure" | "nothrow" | "const" | "immutable" | "shared" | "static" | "final" | "override" | "abstract" | "public" | "private" | "protected" | "export" | "extern" | "scope" | "ref" | "@" IDENT,
    function_name: pointer* IDENT,
    pointer: "*" ["const" | "immutable" | "shared" | "inout"],
    parameters: "(" (parameter %? ",") ")",
    parameter: "..." | parameter_declaration,
    parameter_declaration: _type ["..." | "=" _expression@assign] [IDENT],

    declaration_statement: _type (_init_declarator % ",") ";",
    _init_declarator: pointer* declarator_name ["=" _initializer],
    declarator_name: IDENT declarator_suffix*,
    declarator_suffix: array_suffix | function_type_suffix,
    function_type_suffix: ("delegate" | "function") parameter_type_list,
    array_suffix: "[" [_expression] "]",
    _initializer: _expression@assign | initializer_list,
    initializer_list: "{" (_initializer %? ",") "}",

    // ---- types ----

    _type: type_qualifier* type_base declarator_suffix*,
    type_qualifier: "const" | "immutable" | "shared" | "inout" | "ref" | "lazy" | "scope" | "static" | "final" | "override" | "abstract" | "public" | "private" | "protected" | "package" | "export",
    type_base:
        builtin_type
        | type_path
        | typeof_expression
        | delegate_type,
    builtin_type:
        "void" | "bool" | "byte" | "ubyte" | "short" | "ushort" | "int" | "uint"
        | "long" | "ulong" | "ucent" | "char" | "wchar" | "dchar"
        | "float" | "double" | "real" | "ifloat" | "idouble" | "ireal"
        | "cfloat" | "cdouble" | "creal" | "auto",
    type_path: IDENT ("." IDENT)*,
    typeof_expression: "typeof" typeof_arguments,
    typeof_arguments: "(" _expression ")",
    delegate_type: "delegate" parameter_type_list,
    parameter_type_list: "(" (_type %? ",") ")",

    // ---- statements ----

    _statement:
        block
        | if_statement
        | while_statement
        | do_statement
        | for_statement
        | foreach_statement
        | switch_statement
        | try_statement
        | throw_statement
        | scope_statement
        | synchronized_statement
        | with_statement
        | assert_statement
        | return_statement
        | break_statement
        | continue_statement
        | goto_statement
        | labeled_statement
        | &(declaration_statement) declaration_statement
        | expression_statement,

    block: "{" _statement* "}",
    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: "for" for_header _statement,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression] ")",
    _for_init: &(_for_declaration) _for_declaration | _expression,
    _for_declaration: _type (_init_declarator % ","),
    foreach_statement: ("foreach" | "foreach_reverse") foreach_header _statement,
    foreach_header: "(" foreach_binding ";" _expression ")",
    foreach_binding: &(_type IDENT) foreach_typed | foreach_untyped,
    foreach_typed: _type IDENT ["," _type IDENT],
    foreach_untyped: IDENT ["," IDENT],
    switch_statement: ["final"] "switch" parenthesized_expression switch_body,
    switch_body: "{" _switch_item* "}",
    _switch_item: case_statement | default_statement | _statement,
    case_statement: "case" (_expression % ",") ":" _statement*,
    default_statement: "default" ":" _statement*,
    try_statement: "try" block catch_clause* [finally_clause],
    catch_clause: "catch" [catch_parameter] block,
    catch_parameter: "(" parameter ")",
    finally_clause: "finally" block,
    throw_statement: "throw" _expression ";",
    scope_statement: "scope" scope_kind _statement,
    scope_kind: "(" IDENT ")",
    synchronized_statement: "synchronized" [parenthesized_expression] _statement,
    with_statement: "with" parenthesized_expression _statement,
    assert_statement: "assert" assert_arguments ";",
    assert_arguments: "(" _expression ["," _expression] ")",
    return_statement: "return" [_expression] ";",
    break_statement: "break" [IDENT] ";",
    continue_statement: "continue" [IDENT] ";",
    goto_statement: "goto" (IDENT | "case" ";" | "default" ";"),
    labeled_statement: IDENT ":" _statement,
    parenthesized_expression: "(" _expression ")",
    expression_statement: _expression ";",

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "~=" | "&&=" | "||=" | "<<=" | ">>=" | ">>>=" => assignment_expression,
        cond: postfix "?" _expression@assign ":" _expression@assign => conditional_expression,
        lor: left "||" => binary_expression,
        land: left "&&" => binary_expression,
        bor: left "|" => binary_expression,
        bxor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        eq: left "==" | "!=" | "is" | "in" | "!" "is" | "!" "in" => binary_expression,
        rel: left "<" | ">" | "<=" | ">=" => binary_expression,
        concat: left "~" => binary_expression,
        shift: left "<<" | ">>" | ">>>" => binary_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        unary: prefix "!" | "-" | "+" | "~" | "*" | "&" | "++" | "--" | "cast" cast_arguments | "new" _type => unary_expression,
        postfix "++" | "--" => postfix_expression,
        call: postfix arguments => call_expression,
        index: postfix subscript => index_expression,
        field: postfix "." IDENT => field_expression,
    },

    _primary:
        _literal
        | _name
        | parenthesized_expression
        | array_literal
        | function_literal,
    _literal: NUMBER | STRING | CHAR_LITERAL | BAD_LITERAL | "true" | "false" | "null",
    _name: IDENT | "this" | "super",
    cast_arguments: "(" _type ")",
    arguments: "(" (_argument %? ",") ")",
    _argument: _expression@assign,
    subscript: "[" (_expression %? ",") "]",
