//! Scala through `grammar!`: one of Field 6's four languages (Ruby, PHP, Swift,
//! Scala).
//!
//! The grammar is the brace-and-parenthesis Scala 2/3 subset every source file
//! uses: `package`, `import`, `class`/`trait`/`object`/`enum`, `def`, `val` and
//! `var`, `if`/`while`/`for`/`match`/`try`, and one expression table. Node names
//! follow Tree-sitter's Scala where that costs nothing; the tree is not the
//! same tree.
//!
//! What the DSL covers here:
//!
//! - **Declarations share a modifier prefix.** `@deprecated private final case
//!   class` and `sealed trait` are entered through `&(definition_modifiers
//!   "class")` and friends, exactly as Java's grammar does.
//! - **`match` is a postfix row.** `x match { case ... }` is
//!   `match_expression`, and the row sits just below the lambda arrow so a
//!   `case` pattern (`_expression@or`) cannot swallow the `=>` that follows it.
//! - **A brace after an operand is an argument block.** `xs.map { x => ... }`
//!   and `xs.map { case ... }` are a postfix `block` row, which is safe in
//!   Scala because `if`, `for` and `while` all parenthesise their headers.
//! - **Strings are single tokens.** `s"..."` and `f"..."` interpolations,
//!   `${...}` fields, `"""..."""` blocks and a raw string's body are all one
//!   `string` token scanned with its braces and inner strings balanced.
//! - **`>` before `>` is `GT_JOINED`**, as in the four other grammars, so
//!   `Map[String, List[Int]]` closes both lists and `a >> b` and `a >>> b` are
//!   `GT_JOINED` runs.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **Indentation syntax (Scala 3) is not modelled.** `object X:` with a
//!   significant-indent body has no terminator, and the DSL describes a block
//!   by its brackets; the fixtures use braces. A source using the indentation
//!   form is the one valid Scala file this grammar cannot parse.
//! - **Operator characters are open-ended.** Scala lets a library define any
//!   operator out of punctuation. A grammar cannot enumerate that set, so the
//!   lexer takes the standard operators and the common assignments; an unknown
//!   run becomes `UNKNOWN`, which is an error, not a crash.
//! - **A string's interpolation is a mode switch.** `${...}` returns to code
//!   and the matching `}` returns to the string; the step lexer scans the whole
//!   literal instead, so a field's expression is not parsed.
//! - **A `case` body is statements, not definitions.** `case x => def y = 1`
//!   is legal Scala but rare; a case body here is `_statement*`, so a nested
//!   definition in a case would be an error node.
//! - **A decision sees three tokens.** The modifier-prefixed declarations and
//!   the `case` pattern forms (`e: Type` against `e`) use `&(...)` predicates.
//! - **One kind per rule**, so `class_definition`, `trait_definition`,
//!   `object_definition` and `enum_definition` are four rules.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { SCALA {
    name = "scala";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        NUMBER = "integer",
        FLOAT = "float",
        STRING = "string",
        CHAR = "character",
        SYMBOL = "symbol",
        /// A string, character or symbol literal with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_ABSTRACT = "abstract",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CLASS = "class",
        KW_DEF = "def",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXTENDS = "extends",
        KW_FALSE = "false",
        KW_FINALLY = "finally",
        KW_FOR = "for",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_IMPLICIT = "implicit",
        KW_LAZY = "lazy",
        KW_MATCH = "match",
        KW_NEW = "new",
        KW_NULL = "null",
        KW_OBJECT = "object",
        KW_OVERRIDE = "override",
        KW_PACKAGE = "package",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_RETURN = "return",
        KW_SEALED = "sealed",
        KW_SUPER = "super",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRAIT = "trait",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPE = "type",
        KW_VAL = "val",
        KW_VAR = "var",
        KW_WHILE = "while",
        KW_WITH = "with",
        KW_FINAL = "final",
        KW_OPEN = "open",
        KW_INLINE = "inline",
        KW_TRANSPARENT = "transparent",
        KW_INFIX = "infix",
        KW_GIVEN = "given",
        KW_USING = "using",
        KW_EXPORT = "export",
        KW_YIELD = "yield",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        COLON = ":",
        COLONCOLON = "::",
        ASSIGN = "=",
        ARROW = "=>",
        LARROW = "<-",
        AT = "@",
        UNDERSCORE_DOT = "_.",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        EQ = "==",
        NE = "!=",
        LE = "<=",
        GE = ">=",
        ANDAND = "&&",
        OROR = "||",
        SHL = "<<",
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
        LT = "<",
        GT = ">",
        /// A `>` right before another `>`: the first half of `>>`, `>>>`, `>>=`
        /// or a type argument list closing right before another.
        GT_JOINED = ">joined",
        PLUSPLUS = "++",
        MINUSMINUS = "--",
        BOUND = "<:",
        LOWER_BOUND = ">:",
    }

    program: _top*,

    _top: package_clause | import_declaration | _member,

    package_clause: "package" _qualified_name,
    import_declaration: "import" (_import_expr % ","),
    _import_expr: _qualified_name ["." "*"],
    _qualified_name: _name ("." _name)*,

    _member: _definition | _statement,

    // ---- definitions ----

    definition_modifiers: (annotation | modifier)+,
    modifier:
        "private" | "protected" | "override" | "final" | "sealed" | "abstract" | "lazy"
        | "implicit" | "case" | "open" | "inline" | "transparent" | "infix",
    annotation: "@" _qualified_name [annotation_arguments],
    annotation_arguments: "(" (_expression %? ",") ")",

    _definition:
        &([definition_modifiers] "class") class_definition
        | &([definition_modifiers] "trait") trait_definition
        | &([definition_modifiers] "object") object_definition
        | &([definition_modifiers] "enum") enum_definition
        | &([definition_modifiers] "def") def_definition
        | &([definition_modifiers] ("val" | "var")) val_definition
        | &([definition_modifiers] "type") type_definition,

    class_definition: [definition_modifiers] "class" _name [type_parameters] [class_parameters] [extends_clause] [template_body],
    trait_definition: [definition_modifiers] "trait" _name [type_parameters] [extends_clause] [template_body],
    object_definition: [definition_modifiers] "object" _name [extends_clause] [template_body],
    enum_definition: [definition_modifiers] "enum" _name [type_parameters] [class_parameters] [extends_clause] [template_body],
    def_definition: [definition_modifiers] "def" _name [type_parameters] parameter_clause* [":" _type] [def_body],
    def_body: "=" (block | _expression) | block,
    val_definition: [definition_modifiers] ("val" | "var") (_binding % ","),
    _binding: _name [":" _type] ["=" _expression],
    type_definition: [definition_modifiers] "type" _name [type_parameters] "=" _type,

    class_parameters: "(" (class_parameter %? ",") ")",
    class_parameter: [definition_modifiers] [_class_parameter_kind] _name ":" _type ["=" _expression],
    _class_parameter_kind: "val" | "var",
    parameter_clause: "(" (parameter %? ",") ")",
    parameter: [_parameter_kind] _name ":" _type ["=" _expression],
    _parameter_kind: "implicit" | "using",

    extends_clause: "extends" _type,
    template_body: "{" _member* "}",
    type_parameters: "[" (type_parameter %? ",") "]",
    type_parameter: ["+" | "-"] _name [(":" | "<:" | ">:") _type],

    // ---- statements ----

    _statement:
        if_statement
        | while_statement
        | do_statement
        | for_statement
        | try_statement
        | return_statement
        | throw_statement
        | block
        | ";"
        | expression_statement,

    /// A block may hold definitions too: local `val`s and nested `def`s are
    /// ordinary Scala.
    block: "{" _member* "}",
    expression_statement: _expression,
    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression,
    for_statement: "for" for_header _statement,
    for_header: "(" _enumerators ")",
    _enumerators: _enumerator (";" _enumerator)*,
    _enumerator: generator | _expression@lambda,
    generator: _name "<-" _expression@assign,
    try_statement: "try" try_body catch_clause* [finally_clause],
    try_body: block | _expression,
    catch_clause: "catch" catch_block,
    catch_block: "{" case_clause* "}",
    finally_clause: "finally" try_body,
    return_statement: "return" [_expression],
    throw_statement: "throw" _expression,

    case_clause: "case" _pattern ["if" _expression@or] "=>" _statement*,
    _pattern: &(_name ":") typed_pattern | _expression@or,
    typed_pattern: _name ":" _pattern_type,
    /// A typed pattern's type, without the `=>` suffix: the case arrow is not a
    /// function type.
    _pattern_type: _type_atom (type_arguments | "with" _type)*,
    match_block: "{" case_clause* "}",

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" => assignment,
        lambda: right "=>" => lambda_expression,
        match: postfix "match" match_block => match_expression,
        or: left "||" => binary,
        and: left "&&" => binary,
        bor: left "|" => binary,
        xor: left "^" => binary,
        band: left "&" => binary,
        eq: left "==" | "!=" => binary,
        cmp: left "<" | ">" | "<=" | ">=" => binary,
        cons: right "::" => binary,
        concat: right "++" | "--" => binary,
        shift: left "<<" | GT_JOINED ">" | GT_JOINED GT_JOINED ">" => binary,
        add: left "+" | "-" => binary,
        mul: left "*" | "/" | "%" => binary,
        unary: prefix "-" | "+" | "!" | "~" => unary,
        block_call: postfix block => block_argument,
        call: postfix arguments => call,
        index: postfix subscript => subscript_expression,
        select: postfix "." _name => select,
    },

    _primary:
        NUMBER | FLOAT | STRING | CHAR | SYMBOL
        | "true" | "false" | "null"
        | "this" | "super"
        | _name
        | parenthesized_expression
        | collection_literal
        | new_expression,

    _name: IDENT,
    /// `(x)` or `(a, b)`: a parenthesised expression or a tuple.
    parenthesized_expression: "(" (_expression %? ",") ")",
    arguments: "(" (_argument %? ",") ")",
    _argument: &(_name "=") named_argument | _expression@assign,
    named_argument: _name "=" _expression@assign,
    subscript: "[" _expression "]",
    collection_literal: "[" (_expression %? ",") "]",
    new_expression: "new" _type,

    // ---- types ----

    _type: _type_atom _type_suffix*,
    _type_suffix: type_arguments | "*" | "=>" _type | "with" _type | "&" _type | "|" _type,
    _type_atom: _name | tuple_type,
    tuple_type: "(" (_type %? ",") ")",
    type_arguments: "[" (_type %? ",") "]",
} }

