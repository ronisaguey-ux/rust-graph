//! C# through `grammar!`, covering the surface a security scanner needs:
//! files and namespaces, classes/structs/interfaces/records/enums/delegates,
//! members, statements and the full expression precedence table.
//!
//! Node names follow Tree-sitter's C# where that costs nothing; the tree is
//! not the same tree.
//!
//! **Modelled.** File-scoped and block namespaces, `using` directives
//! (aliases, `static`, `global`), global attributes; classes, structs,
//! interfaces, records (positional and nominal), enums, delegates, nested and
//! local types; fields, constants, methods (generic, `async`, expression
//! bodied), constructors with `base`/`this` initialisers, destructors,
//! properties with `get`/`set`/`init` accessors, indexers, events, operators,
//! static constructors; attribute sections; generic type parameters with
//! `where` constraints; nullable, array, pointer and qualified types;
//! statements: blocks, local declarations (including `using` declarations),
//! `if`/`else`, `while`, `do`, classic `for`, `foreach`, `switch` sections
//! with `when` guards, `try`/`catch`/`finally`, `using`, `lock`, `return`,
//! `throw`, `break`, `continue`, `goto`, `yield return`/`yield break`, labels;
//! expressions: assignment and compound assignment, lambda expressions
//! (typed and untyped), conditional `?:`, `??`, `??=`, logical/bitwise/
//! comparison/`is`/`as`/shift/arithmetic operators, unary operators, `await`,
//! casts, object and array creation, collection and object initialisers,
//! anonymous objects, member access, conditional access `?.`, invocation,
//! element access, the null-forgiving `!`, postfix `++`/`--`, `typeof`,
//! `sizeof`, `default`, `nameof`, `new` and target-typed `new()`.
//!
//! **Not modelled.** Interpolated strings are single tokens: their `{...}`
//! holes are not parsed (the same choice as Python's f-strings and
//! TypeScript's template literals). Raw string literals are single tokens.
//! LINQ query syntax (`from x in xs where ... select`) is not modelled; the
//! method form (`xs.Where(...)`) is. Switch *expressions* are not modelled
//! (only switch statements); pattern combinators (`and`, `or`, `not`,
//! property patterns) are reduced to an `is` type/constant test. A bare
//! `default` literal and `default(T)` are not expressions here: `default` has
//! to end a switch section's statement list, and the DSL cannot both let it
//! start an expression statement and stop there. Preprocessor directives are
//! comments. XML documentation comments are ordinary comments. Local
//! functions are not modelled (a local declaration shape is tried first and
//! reports the error inside the block).
//!
//! **DSL gaps.** (1) Nested generic closers `>>` cannot be split by the
//! grammar, so the lexer emits `GT_JOINED` before a second `>` and both type
//! arguments and the shift operators accept that pair. (2) A member's shape
//! (method versus property versus field versus constructor) is decided by
//! syntactic predicates over attributes, modifiers, a type and a name,
//! because alternatives have no other priority mechanism and the DSL has no
//! arbitrary lookahead. (3) `>` in `where T : IComparable<T>` is an ordinary
//! token; there is no per-rule lexer mode, so the joined-token convention
//! has to hold everywhere.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { CSHARP {
    name = "csharp";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        INTEGER = "integer_literal",
        FLOAT = "floating_point_literal",
        CHAR = "character_literal",
        STRING = "string_literal",
        VERBATIM_STRING = "verbatim_string_literal",
        INTERPOLATED_STRING = "interpolated_string_literal",
        RAW_STRING = "raw_string_literal",
        /// A string or character literal with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_ABSTRACT = "abstract",
        KW_AS = "as",
        KW_BASE = "base",
        KW_BOOL = "bool",
        KW_BREAK = "break",
        KW_BYTE = "byte",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CHAR = "char",
        KW_CHECKED = "checked",
        KW_CLASS = "class",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DECIMAL = "decimal",
        KW_DEFAULT = "default",
        KW_DELEGATE = "delegate",
        KW_DO = "do",
        KW_DOUBLE = "double",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EVENT = "event",
        KW_EXPLICIT = "explicit",
        KW_EXTERN = "extern",
        KW_FALSE = "false",
        KW_FINALLY = "finally",
        KW_FIXED = "fixed",
        KW_FLOAT = "float",
        KW_FOR = "for",
        KW_FOREACH = "foreach",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_IMPLICIT = "implicit",
        KW_IN = "in",
        KW_INT = "int",
        KW_INTERFACE = "interface",
        KW_INTERNAL = "internal",
        KW_IS = "is",
        KW_LOCK = "lock",
        KW_LONG = "long",
        KW_NAMESPACE = "namespace",
        KW_NEW = "new",
        KW_NULL = "null",
        KW_OBJECT = "object",
        KW_OPERATOR = "operator",
        KW_OUT = "out",
        KW_OVERRIDE = "override",
        KW_PARAMS = "params",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_PUBLIC = "public",
        KW_READONLY = "readonly",
        KW_REF = "ref",
        KW_RETURN = "return",
        KW_SBYTE = "sbyte",
        KW_SEALED = "sealed",
        KW_SHORT = "short",
        KW_SIZEOF = "sizeof",
        KW_STACKALLOC = "stackalloc",
        KW_STATIC = "static",
        KW_STRING = "string",
        KW_STRUCT = "struct",
        KW_SWITCH = "switch",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPEOF = "typeof",
        KW_UINT = "uint",
        KW_ULONG = "ulong",
        KW_UNCHECKED = "unchecked",
        KW_UNSAFE = "unsafe",
        KW_USHORT = "ushort",
        KW_USING = "using",
        KW_VIRTUAL = "virtual",
        KW_VOID = "void",
        KW_VOLATILE = "volatile",
        KW_WHILE = "while",
        /// Contextual keywords: names everywhere but where they mean something.
        KW_ASYNC = "async",
        KW_AWAIT = "await",
        KW_RECORD = "record",
        KW_PARTIAL = "partial",
        KW_WHERE = "where",
        KW_GET = "get",
        KW_SET = "set",
        KW_INIT = "init",
        KW_ADD = "add",
        KW_REMOVE = "remove",
        KW_VALUE = "value",
        KW_WHEN = "when",
        KW_REQUIRED = "required",
        KW_GLOBAL = "global",
        KW_NAMEOF = "nameof",
        KW_DYNAMIC = "dynamic",
        KW_SCOPED = "scoped",
        KW_YIELD = "yield",
        KW_ALIAS = "alias",
        KW_FILE = "file",
        KW_WITH = "with",
        KW_VAR = "var",
        KW_NOTNULL = "notnull",
        KW_UNMANAGED = "unmanaged",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        ELLIPSIS = "...",
        AT = "@",
        COLONCOLON = "::",
        COLON = ":",
        QUESTION = "?",
        NULLISH = "??",
        NULLISH_EQ = "??=",
        CONDITIONAL_ACCESS = "?.",
        ARROW = "=>",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        /// A `>` right before another `>`: the first half of `>>`, `>>>`, or a
        /// type argument list closing right before another.
        GT_JOINED = ">joined",
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
        SHL = "<<",
        ANDAND = "&&",
        OROR = "||",
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
        ARROW2 = "->",
    }

    compilation_unit: _top*,
    _top:
        using_directive
        | &([attributes] [modifiers] "namespace") namespace_declaration
        | _type_declaration
        | ";",

    using_directive: ["global"] "using" (using_static | _alias_directive | &(using_group) using_group | using_plain),
    using_static: "static" _type ";",
    using_plain: _type ";",
    using_group: using_items ";",
    using_items: "(" _using_item ("," _using_item)* ")",
    _using_item: ["static"] _type,
    _alias_directive: _name "=" _type ";",

    namespace_declaration: [attributes] [modifiers] "namespace" _qualified_name namespace_tail,
    namespace_tail: ";" _top* | namespace_block,
    namespace_block: "{" _top* "}",
    _qualified_name: _type_name ("." _type_name)*,

    attributes: attribute_section+,
    attribute_section: "[" (attribute %? ",") "]",
    attribute: _qualified_name [attribute_arguments],
    attribute_arguments: "(" (_attribute_argument %? ",") ")",
    _attribute_argument: &(_name ":") named_attribute_argument | _expression,
    named_attribute_argument: _name ":" _expression,

    modifiers: _modifier+,
    _modifier:
        "public" | "private" | "protected" | "internal" | "static" | "abstract" | "sealed"
        | "partial" | "readonly" | "unsafe" | "virtual" | "override" | "new" | "extern"
        | "async" | "const" | "volatile" | "fixed" | "required" | KW_FILE,
    _local_modifiers: (attribute_section | "const" | "ref" | "readonly" | "scoped" | "static" | "unsafe" | "extern")*,

    // ---- type declarations ----

    _type_declaration:
        &([attributes] [modifiers] "class") class_declaration
        | &([attributes] [modifiers] "struct") struct_declaration
        | &([attributes] [modifiers] "interface") interface_declaration
        | &([attributes] [modifiers] "enum") enum_declaration
        | &([attributes] [modifiers] "record") record_declaration
        | &([attributes] [modifiers] "delegate") delegate_declaration,

    class_declaration: [attributes] [modifiers] "class" _name [type_parameters] [base_list] [type_parameter_constraints] class_body [";"],
    struct_declaration: [attributes] [modifiers] "struct" _name [type_parameters] [base_list] [type_parameter_constraints] class_body [";"],
    interface_declaration: [attributes] [modifiers] "interface" _name [type_parameters] [base_list] [type_parameter_constraints] class_body [";"],
    record_declaration: [attributes] [modifiers] "record" ["class" | "struct"] _name [type_parameters] [record_parameters] [base_list] [type_parameter_constraints] (class_body | ";"),
    record_parameters: "(" (parameter %? ",") ")",
    enum_declaration: [attributes] [modifiers] "enum" _name [":" _type] enum_body [";"],
    enum_body: "{" (enum_member %? ",") "}",
    enum_member: [attributes] _name ["=" _expression],
    delegate_declaration: [attributes] [modifiers] "delegate" _type _name [type_parameters] formal_parameters [type_parameter_constraints] ";",

    base_list: ":" _type ("," _type)*,
    type_parameter_constraints: "where" type_parameter_constraint ("," type_parameter_constraint)*,
    type_parameter_constraint: _name ":" _constraint_item ("," _constraint_item)*,
    _constraint_item: "class" | "struct" | KW_NOTNULL | KW_UNMANAGED | new_constraint | _type,
    new_constraint: "new" empty_arguments,
    empty_arguments: "(" ")",
    type_parameters: "<" (type_parameter %? ",") _gt,
    type_parameter: [attributes] ["in" | "out"] _name,

    class_body: "{" _class_member* "}",
    _class_member:
        &([attributes] [modifiers] "~" _name) destructor_declaration
        | &([attributes] [modifiers] _name formal_parameters) constructor_declaration
        | &([attributes] [modifiers] "static" _name formal_parameters) constructor_declaration
        | &([attributes] [modifiers] "class") class_declaration
        | &([attributes] [modifiers] "struct") struct_declaration
        | &([attributes] [modifiers] "interface") interface_declaration
        | &([attributes] [modifiers] "enum") enum_declaration
        | &([attributes] [modifiers] "record" _name) record_declaration
        | &([attributes] [modifiers] _type "operator") operator_declaration
        | &([attributes] [modifiers] _type "this") indexer_declaration
        | &([attributes] [modifiers] "event") event_declaration
        | &([attributes] [modifiers] _type _name [type_parameters] formal_parameters) method_declaration
        | &([attributes] [modifiers] _type _name "=>") expression_bodied_member
        | &([attributes] [modifiers] _type _name property_body) property_declaration
        | field_declaration
        | ";",

    method_declaration: [attributes] [modifiers] _type _name [type_parameters] formal_parameters [type_parameter_constraints] (block | "=>" _expression ";" | ";"),
    expression_bodied_member: [attributes] [modifiers] _type _name "=>" _expression ";",
    constructor_declaration: [attributes] [modifiers] _name formal_parameters [constructor_initializer] (block | "=>" _expression ";"),
    constructor_initializer: ":" ("base" | "this") arguments,
    destructor_declaration: [attributes] [modifiers] "~" _name empty_arguments block,
    property_declaration: [attributes] [modifiers] _type _name property_body [property_tail],
    property_tail: "=" _expression@assign ";" | ";",
    property_body: "{" accessor_declaration* "}",
    accessor_declaration: [attributes] [modifiers] ("get" | "set" | "init" | "add" | "remove") (block | ";"),
    indexer_declaration: [attributes] [modifiers] _type "this" indexer_parameters property_body [property_tail],
    indexer_parameters: "[" (parameter %? ",") "]",
    event_declaration: [attributes] [modifiers] "event" _type _name event_tail,
    event_tail: event_accessors | ";" | "=" _expression ";",
    event_accessors: "{" accessor_declaration* "}",
    operator_declaration: [attributes] [modifiers] ["implicit" | "explicit"] _type "operator" _operator_token formal_parameters (block | "=>" _expression ";"),
    _operator_token: PLUS | MINUS | STAR | SLASH | PERCENT | AMP | PIPE | CARET | BANG | TILDE | EQ | NE | LT | GT | LE | GE | INCREMENT | DECREMENT | "true" | "false",
    field_declaration: [attributes] [modifiers] _type _variable_declarator ("," _variable_declarator)* ";",
    _variable_declarator: _name ["=" _expression@assign],

    // ---- types ----

    _type: precedence _type_head {
        prefix "*" => pointer_type,
        postfix "?" => nullable_type,
        postfix type_arguments => generic_type,
        postfix "." _type_name => qualified_type,
        postfix "::" _type_name => alias_qualified_type,
        postfix dimensions => array_type,
    },
    _type_head: _type_name | _predefined_type | tuple_type,
    _predefined_type:
        "bool" | "byte" | "sbyte" | "short" | "ushort" | "int" | "uint" | "long" | "ulong"
        | "char" | "float" | "double" | "decimal" | "object" | "string" | "void",
    tuple_type: "(" _tuple_element "," _tuple_element ("," _tuple_element)* ")",
    _tuple_element: _type _name,
    dimensions: "[" ("," | _expression)* "]",
    type_arguments: "<" (_type %? ",") _gt,
    _gt: ">" | GT_JOINED,
    formal_parameters: "(" (parameter %? ",") ")",
    parameter: [attributes] ["this" | "ref" | "out" | "in" | "params" | "scoped"] _type ["?"] _name ["=" _expression@assign],
    arguments: "(" (_argument %? ",") ")",
    _argument: &(("ref" | "out" | "in") _type _name) out_declaration | ("ref" | "out" | "in") _expression | _expression,
    out_declaration: ("ref" | "out" | "in") _type _name,

    // ---- statements ----

    block: "{" _statement* "}",
    _statement:
        &(_local_modifiers _type _name ["="]) local_variable_declaration
        | &("using" _local_modifiers _type _name "=") using_declaration
        | &([attributes] [modifiers] "class" _name) class_declaration
        | &([attributes] [modifiers] "struct" _name) struct_declaration
        | &([attributes] [modifiers] "interface" _name) interface_declaration
        | &([attributes] [modifiers] "record" _name) record_declaration
        | &(_name ":") labeled_statement
        | block
        | ";"
        | if_statement
        | while_statement
        | do_statement
        | for_statement
        | foreach_statement
        | switch_statement
        | try_statement
        | using_statement
        | lock_statement
        | return_statement
        | throw_statement
        | break_statement
        | continue_statement
        | goto_statement
        | yield_statement
        | checked_statement
        | unsafe_statement
        | fixed_statement
        | expression_statement,

    local_variable_declaration: _local_modifiers _type _variable_declarator ("," _variable_declarator)* ";",
    using_declaration: "using" _local_modifiers _type _name "=" _expression ";",
    labeled_statement: _name ":" _statement,
    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: "for" for_header _statement,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression ("," _expression)*] ")",
    _for_init:
        &(_local_modifiers _type _name "=") _local_modifiers _type _variable_declarator ("," _variable_declarator)*
        | _expression ("," _expression)*,
    foreach_statement: "foreach" foreach_header _statement,
    foreach_header: "(" _local_modifiers _type _name "in" _expression ")",
    switch_statement: "switch" parenthesized_expression switch_body,
    switch_body: "{" switch_section* "}",
    switch_section: (switch_label ":" | "default" ":") _statement*,
    switch_label: "case" _expression ["when" _expression],
    try_statement: "try" block catch_clause* [finally_clause],
    catch_clause: "catch" [catch_declaration] ["when" parenthesized_expression] block,
    catch_declaration: "(" [_local_modifiers] _type _name ")",
    finally_clause: "finally" block,
    using_statement: "using" using_header _statement,
    using_header: "(" [_local_modifiers] _type _name "=" _expression ")",
    lock_statement: "lock" parenthesized_expression _statement,
    checked_statement: ("checked" | "unchecked") block,
    unsafe_statement: "unsafe" block,
    fixed_statement: "fixed" fixed_header _statement,
    fixed_header: "(" [_local_modifiers] _type _name "=" _expression ")",
    return_statement: "return" [_expression] ";",
    throw_statement: "throw" [_expression] ";",
    break_statement: "break" ";",
    continue_statement: "continue" ";",
    goto_statement: "goto" ("case" _expression | "default" | _name) ";",
    yield_statement: "yield" ("return" _expression | "break") ";",
    expression_statement: _expression ";",
    parenthesized_expression: "(" _expression ")",

    // ---- expressions ----

    _expression: precedence _primary {
        postfix "=>" _lambda_body => lambda_expression,
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" | GT_JOINED GT_JOINED ">=" | "??=" => assignment_expression,
        cond: postfix "?" _expression ":" _expression@cond => conditional_expression,
        nullish: left "??" => binary_expression,
        left "||" => binary_expression,
        left "&&" => binary_expression,
        bor: left "|" => binary_expression,
        xor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        eq: left "==" | "!=" => binary_expression,
        rel: left "<" | ">" | "<=" | ">=" | "is" | "as" => binary_expression,
        shift: left "<<" | GT_JOINED ">" | GT_JOINED GT_JOINED ">" => binary_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        unary: prefix "+" | "-" | "!" | "~" | "++" | "--" | "await" | "&" | "*" => unary_expression,
        postfix "++" | "--" => postfix_expression,
        postfix arguments => invocation_expression,
        postfix "?." _name => conditional_member_access,
        postfix "?." arguments => conditional_invocation,
        postfix "." _name => member_access,
        postfix "." type_arguments _name => member_access,
        postfix subscript => element_access,
        postfix "!" => null_forgiving,
        with_expr: postfix "with" object_initializer => with_expression,
        postfix &(type_arguments arguments) type_arguments arguments => generic_invocation,
    },

    _primary:
        &(lambda_parameters "=>") lambda_parameters
        | &("@" _name) _name
        | &(cast_type _cast_operand) cast_expression
        | parenthesized_expression
        | _literal
        | _name
        | "this"
        | "base"
        | &("new" array_specifier) implicit_array_creation
        | &("new" arguments) implicit_creation
        | &("new" anonymous_body) anonymous_object
        | object_creation
        | typeof_expression
        | sizeof_expression
        | nameof_expression
        | checked_expression
        | _predefined_type
        | stackalloc_expression,
    _literal:
        INTEGER | FLOAT | CHAR | STRING | VERBATIM_STRING | INTERPOLATED_STRING | RAW_STRING
        | "true" | "false" | "null",
    lambda_parameters: "(" (lambda_parameter %? ",") ")",
    lambda_parameter: [attributes] [_local_modifiers] _type _name | _name,
    cast_type: "(" _type ")",
    cast_expression: cast_type _expression@unary,
    _cast_operand: _literal | _name | "this" | "base" | "new" | "typeof" | "sizeof" | "!" | "~",
    _lambda_body: block | _expression,
    object_creation: "new" _type [arguments] [object_initializer],
    implicit_creation: "new" arguments [object_initializer],
    implicit_array_creation: "new" array_specifier+ [array_initializer],
    array_specifier: array_size_specifier | array_rank_specifier,
    array_size_specifier: "[" [_expression] "]",
    array_rank_specifier: "[" ","+ "]",
    anonymous_object: "new" anonymous_body,
    anonymous_body: "{" (_anonymous_member %? ",") "}",
    _anonymous_member: _name "=" _expression,
    object_initializer: "{" (_initializer_item %? ",") "}",
    _initializer_item: &(_name "=") _name "=" _expression | _expression,
    array_initializer: "{" (_initializer_item %? ",") "}",
    subscript: "[" _expression ("," _expression)* "]",
    typeof_expression: "typeof" parenthesized_type,
    sizeof_expression: "sizeof" parenthesized_type,
    nameof_expression: "nameof" nameof_arguments,
    nameof_arguments: "(" _expression ")",
    parenthesized_type: "(" _type ")",
    checked_expression: ("checked" | "unchecked") checked_arguments,
    checked_arguments: "(" _expression ")",
    stackalloc_expression: "stackalloc" _type [array_initializer],

    _name:
        IDENT | "async" | "record" | "partial" | "where" | "get" | "set" | "init" | "add"
        | "remove" | "value" | "when" | "required" | "global" | "dynamic" | "scoped"
        | "yield" | "alias",
    _type_name: _name | KW_VAR,
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'));
            WHITESPACE
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
            c.bump_n(2);
            while !c.at_end() && !c.starts_with(b"*/") {
                c.bump();
            }
            c.bump_n(2);
            COMMENT
        }
        b'#' => {
            // A preprocessor directive, and (at the start) a shebang.
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'@' if c.peek_at(1) == b'"' => {
            c.bump_n(2);
            if verbatim_string(c) { VERBATIM_STRING } else { BAD_LITERAL }
        }
        b'@' if c.peek_at(1) == b'$' && c.peek_at(2) == b'"' => {
            c.bump_n(3);
            if interpolated_string(c, true) { INTERPOLATED_STRING } else { BAD_LITERAL }
        }
        b'@' if is_word_start(c.peek_at(1)) => {
            // A verbatim identifier: `@class` is the name `class`.
            c.bump();
            c.eat_while(is_word_byte);
            IDENT
        }
        b'$' if c.peek_at(1) == b'"' => {
            c.bump_n(2);
            if interpolated_string(c, false) { INTERPOLATED_STRING } else { BAD_LITERAL }
        }
        b'$' if c.peek_at(1) == b'@' && c.peek_at(2) == b'"' => {
            c.bump_n(3);
            if interpolated_string(c, true) { INTERPOLATED_STRING } else { BAD_LITERAL }
        }
        b'"' if c.starts_with(b"\"\"\"") => {
            if raw_string(c) { RAW_STRING } else { BAD_LITERAL }
        }
        b'"' => {
            c.bump();
            if c.quoted(b'"', true) { STRING } else { BAD_LITERAL }
        }
        b'\'' => {
            c.bump();
            if c.quoted(b'\'', true) { CHAR } else { BAD_LITERAL }
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

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// The rest of a verbatim string after its `@"`: `""` is one quote, a line
/// break is allowed. True if it closed.
fn verbatim_string(c: &mut Cursor) -> bool {
    loop {
        if c.at_end() {
            return false;
        }
        if c.peek() == b'"' {
            if c.peek_at(1) == b'"' {
                c.bump_n(2);
                continue;
            }
            c.bump();
            return true;
        }
        c.bump();
    }
}

/// The rest of an interpolated string after its opening `$"` (or `$@"`). Holes
/// are scanned with balanced braces and quoted strings inside them skipped,
/// and stay opaque. `{{` and `}}` are literal braces. True if it closed.
fn interpolated_string(c: &mut Cursor, verbatim: bool) -> bool {
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'"' if verbatim && c.peek_at(1) == b'"' => c.bump_n(2),
            b'"' => {
                c.bump();
                return true;
            }
            b'\n' | b'\r' if !verbatim => return false,
            b'\\' if !verbatim => c.bump_n(2),
            b'{' if c.peek_at(1) == b'{' => c.bump_n(2),
            b'}' if c.peek_at(1) == b'}' => c.bump_n(2),
            b'{' => {
                c.bump();
                if !interpolation_hole(c) {
                    return false;
                }
            }
            _ => c.bump(),
        }
    }
}
