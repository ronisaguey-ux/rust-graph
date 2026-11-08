//! Java through `grammar!`, including the Java 21 surface a scanner sees:
//! records, sealed types, pattern matching, text blocks and modules.
//!
//! Node names follow Tree-sitter's Java where that costs nothing; the tree is
//! not the same tree.
//!
//! **Modelled.** Package and import declarations (static, wildcard), module
//! declarations with every directive; classes, interfaces, enums (with
//! constant bodies), records, annotation types, nested and local types,
//! compact constructors, static and instance initialisers; modifiers,
//! annotations with element-value pairs and arrays, generic type parameters
//! with bounds, wildcards, arrays, annotations on types; methods (varargs,
//! throws, default values), constructors, fields with multiple declarators and
//! array initialisers; blocks, `if`, `while`, `do`, classic and enhanced
//! `for`, `switch` statements and switch expressions with `->` rules, guards,
//! type and record patterns, labels, `yield`, `try` with resources and
//! multi-catch, `synchronized`, `assert`, `throw`, `return`, `break`,
//! `continue`; the whole expression precedence table, assignment operators,
//! lambdas, method references, casts, `instanceof` patterns, object creation,
//! array creation, class literals, qualified `this`/`super`, generic method
//! calls.
//!
//! **Not modelled.** Generic type arguments in a method invocation are only
//! recognised in the `receiver.<T>name(...)` form; a bare `f<T>(x)` parses as
//! two comparisons in expression position (Java's own grammar needs a
//! lookahead the DSL does not have). Annotation processors, module versions,
//! `_` unnamed patterns and preview-only constructs are out of scope. Text
//! blocks are one token; their content is not interpreted.
//!
//! **DSL gaps.** (1) Context-sensitive tokens (`>` before `>` in nested
//! generics) have no lexer-state hook, so the lexer emits `GT_JOINED` and the
//! grammar accepts that pair wherever `>` may close a type list — the same
//! workaround `javascript.rs` uses. (2) A cast is disambiguated with a
//! predicate on the operand shape, because the DSL has no arbitrary
//! lookahead and no "try this parse" operator beyond `&(...)`. (3) A local
//! declaration and an expression statement start alike, so the statement
//! choice is a predicate over modifiers, type and name; there is no priority
//! mechanism beyond alternative order.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { JAVA {
    name = "java";
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
        TEXT_BLOCK = "text_block",
        /// A string, character literal or text block with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_ABSTRACT = "abstract",
        KW_ASSERT = "assert",
        KW_BOOLEAN = "boolean",
        KW_BREAK = "break",
        KW_BYTE = "byte",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CHAR = "char",
        KW_CLASS = "class",
        KW_CONTINUE = "continue",
        KW_DEFAULT = "default",
        KW_DO = "do",
        KW_DOUBLE = "double",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXTENDS = "extends",
        KW_FINAL = "final",
        KW_FINALLY = "finally",
        KW_FLOAT = "float",
        KW_FOR = "for",
        KW_IF = "if",
        KW_IMPLEMENTS = "implements",
        KW_IMPORT = "import",
        KW_INSTANCEOF = "instanceof",
        KW_INT = "int",
        KW_INTERFACE = "interface",
        KW_LONG = "long",
        KW_NATIVE = "native",
        KW_NEW = "new",
        KW_PACKAGE = "package",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_PUBLIC = "public",
        KW_RETURN = "return",
        KW_SHORT = "short",
        KW_STATIC = "static",
        KW_STRICTFP = "strictfp",
        KW_SUPER = "super",
        KW_SWITCH = "switch",
        KW_SYNCHRONIZED = "synchronized",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_THROWS = "throws",
        KW_TRANSIENT = "transient",
        KW_TRY = "try",
        KW_VOID = "void",
        KW_VOLATILE = "volatile",
        KW_WHILE = "while",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_NULL = "null",
        KW_NON_SEALED = "non-sealed",
        AT_INTERFACE = "@interface",
        /// Contextual keywords: names everywhere but where they mean something.
        KW_RECORD = "record",
        KW_YIELD = "yield",
        KW_SEALED = "sealed",
        KW_PERMITS = "permits",
        KW_WHEN = "when",
        KW_MODULE = "module",
        KW_OPEN = "open",
        KW_REQUIRES = "requires",
        KW_EXPORTS = "exports",
        KW_OPENS = "opens",
        KW_TO = "to",
        KW_USES = "uses",
        KW_PROVIDES = "provides",
        KW_WITH = "with",
        KW_TRANSITIVE = "transitive",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        ELLIPSIS = "...",
        AT = "@",
        COLONCOLON = "::",
        COLON = ":",
        QUESTION = "?",
        ARROW = "->",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
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
    }

    program: _top*,
    _top:
        &([modifiers] "package") package_declaration
        | import_declaration
        | &([modifiers] ["open"] "module") module_declaration
        | _class_body_declaration,

    package_declaration: [modifiers] "package" _qualified_name ";",
    import_declaration: "import" ["static"] _name ("." (_name | "*"))* ";",
    _qualified_name: _name ("." _name)*,

    module_declaration: [modifiers] ["open"] "module" _qualified_name module_body,
    module_body: "{" _module_directive* "}",
    _module_directive:
        requires_module_directive
        | exports_module_directive
        | opens_module_directive
        | uses_module_directive
        | provides_module_directive,
    requires_module_directive: "requires" ("transitive" | "static")* _qualified_name ";",
    exports_module_directive: "exports" _qualified_name ["to" _qualified_name ("," _qualified_name)*] ";",
    opens_module_directive: "opens" _qualified_name ["to" _qualified_name ("," _qualified_name)*] ";",
    uses_module_directive: "uses" _qualified_name ";",
    provides_module_directive: "provides" _qualified_name "with" _qualified_name ("," _qualified_name)* ";",

    // ---- declarations ----

    modifiers: _modifier+,
    _modifier:
        annotation
        | "public" | "protected" | "private" | "static" | "abstract" | "final" | "native"
        | "synchronized" | "transient" | "volatile" | "strictfp" | "default" | "sealed" | "non-sealed",
    /// What a local variable or parameter may have.
    _local_modifiers: (annotation | "final")+,
    /// What a local class, interface, enum or record may have.
    _local_type_modifiers: (annotation | "final" | "abstract" | "static" | "strictfp" | "sealed" | "non-sealed")+,
    annotation: "@" _qualified_name [annotation_argument_list],
    annotation_argument_list: "(" [_annotation_arguments] ")",
    _annotation_arguments: &(_name "=") element_value_pair ("," element_value_pair)* | _element_value,
    element_value_pair: _name "=" _element_value,
    _element_value: annotation | element_value_array_initializer | _expression@cond,
    element_value_array_initializer: "{" (_element_value %? ",") "}",

    class_declaration: [modifiers] "class" _type_identifier [type_parameters] [superclass] [super_interfaces] [permits] class_body,
    interface_declaration: [modifiers] "interface" _type_identifier [type_parameters] [extends_interfaces] [permits] class_body,
    enum_declaration: [modifiers] "enum" _type_identifier [super_interfaces] enum_body,
    record_declaration: [modifiers] "record" _type_identifier [type_parameters] formal_parameters [super_interfaces] class_body,
    annotation_type_declaration: [modifiers] "@interface" _type_identifier class_body,
    superclass: "extends" _type,
    super_interfaces: "implements" _type ("," _type)*,
    extends_interfaces: "extends" _type ("," _type)*,
    permits: "permits" _type ("," _type)*,
    class_body: "{" _class_body_declaration* "}",
    enum_body: "{" (enum_constant %? ",") [enum_body_declarations] "}",
    enum_body_declarations: ";" _class_body_declaration*,
    enum_constant: [modifiers] _name [arguments] [class_body],

    _class_body_declaration:
        &([modifiers] [type_parameters] _type _name formal_parameters) method_declaration
        | &([modifiers] [type_parameters] _type_identifier formal_parameters) constructor_declaration
        | &([modifiers] "class") class_declaration
        | &([modifiers] "interface") interface_declaration
        | &([modifiers] "enum") enum_declaration
        | &([modifiers] "record" _type_identifier) record_declaration
        | &([modifiers] "@interface") annotation_type_declaration
        | &([modifiers] _type_identifier block) compact_constructor_declaration
        | &("static" block) static_initializer
        | block
        | ";"
        | field_declaration,
    method_declaration: [modifiers] [type_parameters] _type _name formal_parameters dimensions* [throws] ["default" _element_value] (block | ";"),
    constructor_declaration: [modifiers] [type_parameters] _type_identifier formal_parameters [throws] block,
    compact_constructor_declaration: [modifiers] _type_identifier block,
    static_initializer: "static" block,
    field_declaration: [modifiers] _type _variable_declarators ";",
    _variable_declarators: variable_declarator ("," variable_declarator)*,
    variable_declarator: _name dimensions* ["=" _variable_initializer],
    _variable_initializer: _expression | array_initializer,
    array_initializer: "{" (_variable_initializer %? ",") "}",
    throws: "throws" _type ("," _type)*,
    formal_parameters: "(" (formal_parameter %? ",") ")",
    formal_parameter: [_local_modifiers] _type annotation* ["..."] _name dimensions*,

    // ---- types ----

    _type: precedence _type_head {
        prefix annotation => annotated_type,
        postfix "." _type_identifier => scoped_type_identifier,
        postfix "." annotation+ _type_identifier => scoped_type_identifier,
        postfix type_arguments => generic_type,
        postfix dimensions => array_type,
        postfix &(annotation+ dimensions) annotation+ dimensions => array_type,
    },
    _type_head: _type_identifier | _primitive_type,
    _creation_type: precedence _type_head {
        postfix "." _type_identifier => scoped_type_identifier,
        postfix "." annotation+ _type_identifier => scoped_type_identifier,
        postfix type_arguments => generic_type,
    },
    _primitive_type: "boolean" | "byte" | "short" | "int" | "long" | "char" | "float" | "double" | "void",
    dimensions: "[" "]",
    type_arguments: "<" (_type_argument %? ",") _gt,
    _type_argument: &(annotation* "?") wildcard | _type,
    wildcard: annotation* "?" [("extends" | "super") _type],
    type_parameters: "<" (type_parameter %? ",") _gt,
    type_parameter: annotation* _type_identifier ["extends" _type ("&" _type)*],
    _gt: ">" | GT_JOINED,

    // ---- statements ----

    block: "{" _statement* "}",
    _statement:
        &([_local_modifiers] _type _name) local_variable_declaration
        | &([_local_type_modifiers] "class") class_declaration
        | &([_local_type_modifiers] "interface") interface_declaration
        | &([_local_type_modifiers] "enum") enum_declaration
        | &([_local_type_modifiers] "record" _type_identifier) record_declaration
        | &(_name ":") labeled_statement
        | &("yield" _expression ";") yield_statement
        | &("switch") switch_expression
        | block
        | ";"
        | if_statement
        | while_statement
        | do_statement
        | for_statement
        | return_statement
        | break_statement
        | continue_statement
        | throw_statement
        | try_statement
        | synchronized_statement
        | assert_statement
        | expression_statement,
    local_variable_declaration: [_local_modifiers] _type _variable_declarators ";",
    labeled_statement: _name ":" _statement,
    yield_statement: "yield" _expression ";",
    expression_statement: _expression ";",
    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: "for" for_header _statement,
    for_header: "(" _for_parts ")",
    _for_parts:
        &([_local_modifiers] _type _name ":") [_local_modifiers] _type _name ":" _expression
        | [_for_init] ";" [_expression] ";" [_expression ("," _expression)*],
    _for_init:
        &([_local_modifiers] _type _name) [_local_modifiers] _type _variable_declarators
        | _expression ("," _expression)*,
    return_statement: "return" [_expression] ";",
    break_statement: "break" [_name] ";",
    continue_statement: "continue" [_name] ";",
    throw_statement: "throw" _expression ";",
    synchronized_statement: "synchronized" parenthesized_expression block,
    assert_statement: "assert" _expression [":" _expression] ";",
    /// With resources a `try` needs nothing after its block; without, a
    /// `catch` or a `finally`.
    try_statement: "try" (resource_specification block catch_clause* [finally_clause] | block (catch_clause+ [finally_clause] | finally_clause)),
    resource_specification: "(" (resource %? ";") ")",
    resource: &([_local_modifiers] _type _name "=") [_local_modifiers] _type _name "=" _expression | _expression,
    catch_clause: "catch" catch_formal_parameter block,
    catch_formal_parameter: "(" [_local_modifiers] _type ("|" _type)* _name ")",
    finally_clause: "finally" block,

    switch_expression: "switch" parenthesized_expression switch_block,
    switch_block: "{" (&(switch_label "->") switch_rule | switch_block_statement_group)* "}",
    switch_rule: switch_label "->" (expression_statement | block | throw_statement),
    switch_block_statement_group: (switch_label ":")+ _statement*,
    switch_label: "case" _case_item ("," _case_item)* [guard] | "default",
    guard: "when" _expression@cond,
    _case_item:
        &(_type record_pattern_body) record_pattern
        | &([_local_modifiers] _type _name) type_pattern
        | "default"
        | _expression@cond,
    type_pattern: [_local_modifiers] _type _name,
    record_pattern: _type record_pattern_body,
    record_pattern_body: "(" (_pattern %? ",") ")",
    _pattern: &(_type record_pattern_body) record_pattern | type_pattern,

    // ---- expressions ----

    _expression: precedence _primary {
