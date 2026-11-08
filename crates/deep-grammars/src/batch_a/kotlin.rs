//! Kotlin through `grammar!`, with semicolon inference done in the step lexer
//! (a line break terminates a statement when the previous token can end one
//! and the next cannot continue it).
//!
//! Node names follow Tree-sitter's Kotlin where that costs nothing; the tree
//! is not the same tree.
//!
//! **Modelled.** Package headers, imports (aliases, wildcards), file-level
//! declarations; classes, interfaces, enum classes (with constructor arguments
//! and bodies), data/value/sealed/annotation classes, objects and companion
//! objects, primary constructors with `val`/`var` parameters, secondary
//! constructors, `init` blocks, supertype lists with constructor calls,
//! generic type parameters, `where` constraints, function declarations
//! (generic, `suspend`, extension receivers, default arguments, expression
//! bodies), property declarations (`val`/`var`, custom getters and setters,
//! delegation via `by`), type aliases; statements: local declarations, `if`/
//! `else`, `while`, `do`/`while`, `for` with destructuring, `when` with
//! subject or subjectless entries, `try`/`catch`/`finally`, `return`/`throw`/
//! `break`/`continue` with labels, expression statements; expressions:
//! assignment and compound assignment, lambdas (with and without parameters,
//! trailing and standalone), `?:` elvis, `||`, `&&`, equality (including
//! referential `===`/`!==`), comparisons, `in`/`!in`, `is`/`!is`, `as`/`as?`,
//! ranges `..` and `..<`, arithmetic, unary operators, calls (including
//! generic calls and trailing lambdas), indexing, member access, safe calls
//! `?.`, callable references `::`, `!!`, postfix `++`/`--`, object
//! expressions, string templates (single token), every literal form.
//!
//! **Not modelled.** Infix function calls (`1 to 2`, `a shl b`) are not
//! recognised: the callee is just a name, so `1 to 2` leaves `to 2` as an
//! error. String templates are single tokens: `$name` and `${...}` are opaque.
//! Annotation use-site targets are kept as `@` plus a name. Operator
//! conventions (`plus`, `get`) and context receivers are not modelled. There
//! is no `expect`/`actual` pairing or multiplatform source-set logic. A
//! property's getter on the following line is a member of its own
//! (`accessor`), not attached to the property, because the class body cannot
//! tell a trailing accessor from the next declaration. Function types are
//! modelled as `(A, B) -> C` only; a parenthesised non-arrow type is not.
//!
//! **DSL gaps.** (1) Semicolon inference needs to know bracket depth, which a
//! token-at-a-time lexer cannot see; the lexer keeps a `thread_local` depth of
//! `(`/`[` so a line break inside an argument list does not become `LINE_END`.
//! This is lexer state the DSL has no place for, so it lives in the step
//! function. (2) A trailing lambda is a postfix `{...}` row; because the DSL
//! has one node kind per rule, it produces a `call_expression` rather than a
//! distinct `call_with_trailing_lambda`. (3) `if`, `when` and `try` are used
//! both as statements and as expressions; one rule each is used from both
//! positions, so the same node kind appears in either context. (4) A lambda's
//! parameters are only taken when a `->` follows, which needs the predicate
//! `&(_lambda_parameters "->")`; a plain optional parameter list would swallow
//! the first expression of an implicit-`it` lambda.

use std::cell::Cell;

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

thread_local! {
    /// Parenthesis and bracket depth, for Kotlin's "a newline inside
    /// parentheses is not a statement end" rule. Reset at the start of a file.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

rust_graph_deep::grammar! { KOTLIN {
    name = "kotlin";
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
        RAW_STRING = "raw_string_literal",
        /// A string with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_AS = "as",
        KW_BREAK = "break",
        KW_CLASS = "class",
        KW_CONTINUE = "continue",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_FALSE = "false",
        KW_FOR = "for",
        KW_FUN = "fun",
        KW_IF = "if",
        KW_IN = "in",
        KW_INTERFACE = "interface",
        KW_IS = "is",
        KW_NULL = "null",
        KW_OBJECT = "object",
        KW_PACKAGE = "package",
        KW_RETURN = "return",
        KW_SUPER = "super",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPEALIAS = "typealias",
        KW_VAL = "val",
        KW_VAR = "var",
        KW_WHEN = "when",
        KW_WHILE = "while",
        KW_IMPORT = "import",
        KW_FINALLY = "finally",
        KW_CATCH = "catch",
        /// Contextual keywords: names everywhere but where they mean something.
        KW_ABSTRACT = "abstract",
        KW_ACTUAL = "actual",
        KW_ANNOTATION = "annotation",
        KW_BY = "by",
        KW_COMPANION = "companion",
        KW_CONST = "const",
        KW_CONSTRUCTOR = "constructor",
        KW_CONTEXT = "context",
        KW_CROSSINLINE = "crossinline",
        KW_DATA = "data",
        KW_DELEGATE = "delegate",
        KW_DYNAMIC = "dynamic",
        KW_ENUM = "enum",
        KW_EXPECT = "expect",
        KW_EXTERNAL = "external",
        KW_FIELD = "field",
        KW_FINAL = "final",
        KW_GET = "get",
        KW_INFIX = "infix",
        KW_INIT = "init",
        KW_INLINE = "inline",
        KW_INNER = "inner",
        KW_INTERNAL = "internal",
        KW_LATEINIT = "lateinit",
        KW_NOINLINE = "noinline",
        KW_OPEN = "open",
        KW_OPERATOR = "operator",
        KW_OUT = "out",
        KW_OVERRIDE = "override",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_PUBLIC = "public",
        KW_REIFIED = "reified",
        KW_SEALED = "sealed",
        KW_SET = "set",
        KW_SUSPEND = "suspend",
        KW_TAILREC = "tailrec",
        KW_VALUE = "value",
        KW_VARARG = "vararg",
        KW_WHERE = "where",
        /// A line break where Kotlin infers a statement end.
        LINE_END = "line_end",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        RANGE = "..",
        RANGE_UNTIL = "..<",
        SAFE_CALL = "?.",
        ELVIS = "?:",
        BANG_BANG = "!!",
        COLONCOLON = "::",
        ARROW = "->",
        AT = "@",
        COLON = ":",
        QUESTION = "?",
        ASSIGN = "=",
        EQ = "==",
        STRICT_EQ = "===",
        NE = "!=",
        STRICT_NE = "!==",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        /// A `>` right before another `>`: the first half of a type argument
        /// list closing right before another.
        GT_JOINED = ">joined",
        BANG = "!",
        NOT_IN = "!in",
        NOT_IS = "!is",
        TILDE = "~",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
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
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
    }

    kotlin_file: _top*,
    _top: package_header | import_header | _declaration | LINE_END | ";",

    package_header: "package" _qualified_name,
    import_header: "import" _qualified_name ["as" _name],
    _qualified_name: _name ("." (_name | "*"))*,

    modifiers: _modifier+,
    _modifier: annotation | _modifier_word,
    _modifier_word:
        "public" | "private" | "protected" | "internal" | "abstract" | "final" | "open"
        | "sealed" | "data" | "value" | "inner" | "override" | "lateinit" | "const" | "suspend"
        | "inline" | "infix" | "operator" | "tailrec" | "external" | "annotation"
        | "vararg" | "noinline" | "crossinline" | "reified" | "expect" | "actual"
        | "dynamic" | "init" | "delegate",
    annotation: "@" _annotation_target _qualified_name [annotation_arguments],
    _annotation_target: [_name ":"],

    // ---- declarations ----

    _declaration:
        &([modifiers] "enum" "class") enum_class_declaration
        | &([modifiers] "class") class_declaration
        | &([modifiers] "interface") interface_declaration
        | &([modifiers] "object") object_declaration
        | &([modifiers] "companion") companion_object
        | &([modifiers] "fun") function_declaration
        | &([modifiers] ("val" | "var")) property_declaration
        | &([modifiers] "constructor") secondary_constructor
        | &([modifiers] "init") init_block
        | typealias_declaration,

    class_declaration: [modifiers] "class" _name [type_parameters] [primary_constructor] [":" supertypes] [type_constraints] [class_body],
    interface_declaration: [modifiers] "interface" _name [type_parameters] [":" supertypes] [type_constraints] [class_body],
    enum_class_declaration: [modifiers] ["enum"] "class" _name [type_parameters] [primary_constructor] [":" supertypes] [type_constraints] enum_body,
    object_declaration: [modifiers] "object" _name [":" supertypes] [class_body],
    companion_object: [modifiers] "companion" "object" [_name] [":" supertypes] [class_body],
    typealias_declaration: [modifiers] "typealias" _name [type_parameters] "=" _type,

    primary_constructor: [modifiers] ["constructor"] class_parameters,
    class_parameters: "(" (class_parameter %? ",") ")",
    class_parameter: [parameter_modifiers] ["val" | "var"] _name ":" _type ["=" _expression],
    secondary_constructor: [modifiers] "constructor" parameter_list [":" constructor_delegation] block,
    constructor_delegation: ("this" | "super") arguments,
    init_block: [modifiers] "init" block,
    supertypes: _supertype ("," _supertype)*,
    _supertype: &(superclass_constructor) superclass_constructor | _type,
    superclass_constructor: _type arguments,
    type_constraints: "where" type_constraint ("," type_constraint)*,
    type_constraint: _name ":" _type,

    class_body: "{" _member* "}",
    _member:
        &([modifiers] "fun") function_declaration
        | &([modifiers] ("val" | "var")) property_declaration
        | &([modifiers] "constructor") secondary_constructor
        | &([modifiers] "init") init_block
        | &([modifiers] "companion") companion_object
        | &([modifiers] "class") class_declaration
        | &([modifiers] "interface") interface_declaration
        | &([modifiers] "enum" "class") enum_class_declaration
        | &([modifiers] "object") object_declaration
        | typealias_declaration
        | &(accessor) accessor
        | LINE_END
        | ";",

    function_declaration:
        [modifiers] "fun" [type_parameters] function_head parameter_list [":" _type]
        [type_constraints] [function_body],
    function_body: block | "=" _expression,
    function_head: &(receiver_head) receiver_head | _name,
    receiver_head: receiver_type "." _name,
    receiver_type: _type_head [type_arguments] ["?"],
    parameter_list: "(" (parameter %? ",") ")",
    parameter: [parameter_modifiers] _name ":" _type ["=" _expression],
    parameter_modifiers: (annotation | "public" | "private" | "protected" | "internal" | "vararg" | "noinline" | "crossinline" | "out" | "override")+,
    property_declaration:
        [modifiers] ("val" | "var") [type_parameters] property_head [":" _type]
        (["by" _expression | "=" _expression])?,
    property_head: &(_type ".") receiver_head | _name,
    accessor: [modifiers] ("get" | "set") [accessor_parameters] ("=" _expression | block),
    accessor_parameters: "(" [parameter] ")",

    enum_body: "{" (enum_entry %? ",") [";" _member*] "}",
    enum_entry: [annotations] _name [arguments] [class_body],
    annotations: annotation+,

    type_parameters: "<" (type_parameter %? ",") _gt,
    type_parameter: [modifiers] _name [":" _type],
    type_arguments: "<" (_type_argument %? ",") _gt,
    _type_argument: "*" | _type,
    _gt: ">" | GT_JOINED,

    // ---- statements ----

