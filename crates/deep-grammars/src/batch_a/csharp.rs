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
