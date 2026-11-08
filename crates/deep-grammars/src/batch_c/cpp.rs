//! C++ through `grammar!`: C's skeleton with namespaces, classes, templates,
//! references, `new`/`delete`, named casts and lambdas.
//!
//! The `>`-splitting of the shared lexer is what makes nested templates parse:
//! `_gt: ">" | GT_JOINED` closes both lists in `vector<vector<int>>`, and the
//! shift row spells `>>` as the same pair, so `a >> b` is a shift.
//!
//! Not modelled: operator overloads, conversion operators, out-of-line member
//! definitions (`Foo::bar() {}`), user-defined literals, attributes,
//! `consteval`/`concept`/`requires`/`co_await` (the C++20 surface). A cast to
//! a bare user type (`(Foo)x`) is not a cast here: with no symbol table it
//! stays a parenthesized expression, which is also a legal reading.

use crate::batch_c::{directive, is_word_byte, is_word_start, number, quoted, trivia};
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, UNKNOWN};

rust_graph_deep::grammar! { CPP {
    name = "cpp";
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
        DIRECTIVE = "preprocessor" trivia,

        KW_AUTO = "auto",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CHAR = "char",
        KW_CLASS = "class",
        KW_CONST = "const",
        KW_CONSTEXPR = "constexpr",
        KW_CONTINUE = "continue",
        KW_DECLTYPE = "decltype",
        KW_DEFAULT = "default",
        KW_DELETE = "delete",
        KW_DO = "do",
        KW_DOUBLE = "double",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXPLICIT = "explicit",
        KW_EXTERN = "extern",
        KW_FALSE = "false",
        KW_FINAL = "final",
        KW_FLOAT = "float",
        KW_FOR = "for",
        KW_FRIEND = "friend",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_INLINE = "inline",
        KW_INT = "int",
        KW_LONG = "long",
        KW_MUTABLE = "mutable",
        KW_NAMESPACE = "namespace",
        KW_NEW = "new",
        KW_NOEXCEPT = "noexcept",
        KW_NULLPTR = "nullptr",
        KW_OPERATOR = "operator",
        KW_OVERRIDE = "override",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_PUBLIC = "public",
        KW_REGISTER = "register",
        KW_RETURN = "return",
        KW_SHORT = "short",
        KW_SIGNED = "signed",
        KW_SIZEOF = "sizeof",
        KW_STATIC = "static",
        KW_STATIC_ASSERT = "static_assert",
        KW_STATIC_CAST = "static_cast",
        KW_DYNAMIC_CAST = "dynamic_cast",
        KW_REINTERPRET_CAST = "reinterpret_cast",
        KW_CONST_CAST = "const_cast",
        KW_STRUCT = "struct",
        KW_SWITCH = "switch",
        KW_TEMPLATE = "template",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPEDEF = "typedef",
        KW_TYPENAME = "typename",
        KW_UNION = "union",
        KW_UNSIGNED = "unsigned",
        KW_USING = "using",
        KW_VIRTUAL = "virtual",
        KW_VOID = "void",
        KW_VOLATILE = "volatile",
        KW_WHILE = "while",
        KW_BOOL = "bool",

        COMMA = ",",
        SEMI = ";",
        DOT = ".",
        ELLIPSIS = "...",
        QUESTION = "?",
        COLON = ":",
        SCOPE = "::",
        ARROW = "->",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
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
        &(declaration_specifiers _declarator _function_tail "{") function_definition
        | &(scoped_type_name _declarator _function_tail "{") scoped_user_function
        | &(plain_type_name _declarator _function_tail "{") plain_user_function
        | namespace_definition
        | template_declaration
        | using_declaration
        | declaration_statement,

    function_definition: declaration_specifiers _declarator _function_tail compound_statement,
    scoped_user_function: scoped_type_name _declarator _function_tail compound_statement,
    plain_user_function: plain_type_name _declarator _function_tail compound_statement,
    _function_tail: _cv_qualifier* ["noexcept"] ["override"] ["final"] ["->" _type_id],
    _cv_qualifier: "const" | "volatile",

    // ---- declarations ----

    declaration_statement:
        &(scoped_type_name) _scoped_user_declaration
        | _user_type_declaration
        | &(declaration_specifiers scoped_type_name) _scoped_qualified_declaration
        | _default_declaration,
    _scoped_user_declaration: scoped_type_name (_init_declarator % ",") ";",
    _user_type_declaration: plain_type_name (_init_declarator % ",") ";",
    _scoped_qualified_declaration: declaration_specifiers scoped_type_name (_init_declarator % ",") ";",
    _default_declaration: declaration_specifiers (_init_declarator % ",") ";",
    _init_declarator: _declarator ["=" _initializer],
    _initializer: designated_field | &(array_suffix "=") designated_index | _expression@assign | initializer_list,
    initializer_list: "{" (_initializer %? ",") "}",
    designated_field: "." IDENT "=" _initializer,
    designated_index: array_suffix "=" _initializer,

    static_assert_statement: "static_assert" static_assert_arguments ";",
    static_assert_arguments: "(" _expression ["," string] ")",

    declaration_specifiers: _declaration_specifier+,
    _declaration_specifier:
        "typedef" | "extern" | "static" | "auto" | "register" | "inline" | "mutable"
        | "constexpr" | "virtual" | "explicit" | "friend" | "const" | "volatile"
        | type_specifier,
    type_specifier:
        "void" | "char" | "short" | "int" | "long" | "float" | "double"
        | "signed" | "unsigned" | "bool"
        | "decltype" decltype_arguments
        | class_specifier
        | enum_specifier,
    decltype_arguments: "(" _expression ")",

    enum_specifier: "enum" ["class" | "struct"] [IDENT] [enum_body],
    enum_body: "{" (_enumerator %? ",") "}",
    _enumerator: IDENT ["=" _expression@assign],

    // ---- namespaces, using, templates ----

    namespace_definition: "namespace" [_namespace_name] namespace_body,
    _namespace_name: IDENT ("::" IDENT)*,
    namespace_body: "{" _top_item* "}",

    using_declaration:
        "using" "namespace" [_namespace_name] ";"
        | "using" IDENT "=" _type_id ";",

    template_declaration: "template" template_parameter_list _top_item,
    template_parameter_list: "<" (_template_parameter % ",") _gt,
    _template_parameter: type_parameter | non_type_parameter,
    type_parameter: ("typename" | "class") [IDENT] ["=" _type_id],
    non_type_parameter: _non_class_specifier+ [_declarator] ["=" _expression@assign],
    _non_class_specifier:
        "const" | "volatile" | "unsigned" | "signed" | "short" | "long" | "int" | "char"
        | "bool" | "auto" | enum_specifier | scoped_type_name,

    template_arguments: "<" (_type_id % ",") _gt,
    _gt: ">" | GT_JOINED,

    // ---- classes ----

    class_specifier: ("class" | "struct" | "union") [IDENT] [base_clause] [class_body],
    base_clause: ":" (_base_specifier % ","),
    _base_specifier: ["virtual"] [("public" | "private" | "protected")] _type_id,
    class_body: "{" _class_member* "}",
    _class_member:
        &(IDENT parameter_list ("{" | ":")) constructor_definition
        | &("~" IDENT parameter_list) destructor_definition
        | &(declaration_specifiers _declarator _function_tail "{") function_definition
        | &(scoped_type_name _declarator _function_tail "{") scoped_user_function
        | &(plain_type_name _declarator _function_tail "{") plain_user_function
        | access_specifier
        | template_declaration
        | declaration_statement
        | ";",
    access_specifier: ("public" | "private" | "protected") ":",
    constructor_definition: IDENT parameter_list [member_initializer_list] compound_statement,
    destructor_definition: "~" IDENT parameter_list [member_initializer_list] compound_statement,
    member_initializer_list: ":" (_member_initializer % ","),
    _member_initializer: IDENT member_initializer_args,
    member_initializer_args: arguments | initializer_list,

    // ---- types and declarators ----

    _declarator: pointer* direct_declarator,
    pointer: ("*" | "&" | "&&") _pointer_qualifier*,
    _pointer_qualifier: "const" | "volatile",
    direct_declarator: _direct_base _declarator_suffix*,
    _direct_base: IDENT | parenthesized_declarator,
    parenthesized_declarator: "(" _declarator ")",
    _declarator_suffix: array_suffix | parameter_list,
    array_suffix: "[" [_expression] "]",
    parameter_list: "(" (_parameter % ",") ")",
    _parameter:
        &(scoped_type_name) _scoped_user_parameter
        | _user_type_parameter
        | &(declaration_specifiers scoped_type_name) _scoped_parameter
        | &(declaration_specifiers plain_type_name _declarator) _qualified_user_parameter
        | _default_parameter
        | "...",
    _scoped_user_parameter: scoped_type_name _declarator,
    _user_type_parameter: plain_type_name _declarator,
    _scoped_parameter: declaration_specifiers scoped_type_name _declarator,
    _qualified_user_parameter: declaration_specifiers plain_type_name _declarator,
    _default_parameter: declaration_specifiers [_declarator],

    _abstract_declarator: pointer+ abstract_suffix*,
    abstract_suffix: array_suffix | parameter_list,

    _type_id: _type_prefixed | &(scoped_type_name) _scoped_type_id | _user_type_id,
    _type_prefixed: declaration_specifiers [user_type_name] [_abstract_declarator],
    user_type_name: _name_part ("::" _name_part)*,
    _user_type_id: plain_type_name [_abstract_declarator],
    _scoped_type_id: scoped_type_name [_abstract_declarator],
    scoped_type_name: _name_part ("::" _name_part)+,
    plain_type_name: _name_part,
    _name_part: IDENT [template_arguments],

    // ---- statements ----

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
        | try_statement
        | throw_statement
        | static_assert_statement
        | &(declaration_specifiers) declaration_statement
        | &(scoped_type_name _declarator) declaration_statement
        | expression_statement,

    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    switch_statement: "switch" parenthesized_expression switch_body,
    switch_body: "{" _switch_item* "}",
    _switch_item: case_statement | default_statement | _statement,
    case_statement: "case" _expression@assign ":" _statement*,
    default_statement: "default" ":" _statement*,
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: &("for" range_for_header) range_for_statement | classic_for_statement,
    classic_for_statement: "for" for_header _statement,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression] ")",
    range_for_statement: "for" range_for_header _statement,
    range_for_header: "(" _range_for_declaration ":" _expression ")",
    _range_for_declaration: declaration_specifiers _declarator | plain_type_name _declarator,
    _for_init: &(declaration_specifiers) _for_declaration | _expression,
    _for_declaration: declaration_specifiers (_init_declarator % ","),
    parenthesized_expression: "(" _expression ")",
    return_statement: "return" [_expression] ";",
    break_statement: "break" ";",
    continue_statement: "continue" ";",
    goto_statement: "goto" (IDENT | "*" _expression) ";",
    labeled_statement: IDENT ":" _statement,
    try_statement: "try" compound_statement catch_clause+,
    catch_clause: "catch" catch_parameter compound_statement,
    catch_parameter: "(" [_parameter] ")",
    throw_statement: "throw" [_expression] ";",
    expression_statement: _expression ";",
