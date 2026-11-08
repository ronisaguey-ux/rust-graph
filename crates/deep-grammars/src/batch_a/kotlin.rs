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

    block: "{" _block_item* "}",
    _block_item: _statement | LINE_END | ";",
    _statement:
        &_declaration _declaration
        | &(block) block
        | &(_name "@") labeled_statement
        | while_statement
        | do_statement
        | for_statement
        | return_statement
        | throw_statement
        | break_statement
        | continue_statement
        | expression_statement,

    labeled_statement: _name "@" _statement,
    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression,
    for_statement: "for" for_header _statement,
    for_header: "(" _for_variable "in" _expression ")",
    _for_variable: _name | destructuring_declaration,
    destructuring_declaration: "(" (_name %? ",") ")",
    when_statement: "when" [when_subject] when_body,
    when_subject: "(" _when_subject_body ")",
    _when_subject_body: ("val" | "var") _name "=" _expression | _expression,
    when_body: "{" _when_item* "}",
    _when_item: when_entry | LINE_END | ";",
    when_entry: [annotations] _when_condition ("," _when_condition)* "->" (&(block) block | _statement),
    _when_condition: "else" | "in" _expression | NOT_IN _expression | "is" _type | NOT_IS _type | _expression,
    try_statement: "try" block (catch_clause+ [finally_clause] | finally_clause),
    catch_clause: "catch" catch_parameter block,
    catch_parameter: "(" _name ":" _type ")",
    finally_clause: "finally" block,
    return_statement: "return" ["@" _name] [_expression],
    throw_statement: "throw" _expression,
    break_statement: "break" ["@" _name],
    continue_statement: "continue" ["@" _name],
    expression_statement: _expression,

    parenthesized_expression: "(" _expression ")",
    arguments: "(" (_argument %? ",") ")",
    _argument: &(_name "=") named_argument | _expression,
    named_argument: _name "=" _expression,
    annotation_arguments: "(" (_annotation_argument %? ",") ")",
    _annotation_argument: _expression,

    // ---- expressions ----

    _expression: precedence _primary {
        right "=" | "+=" | "-=" | "*=" | "/=" | "%=" => assignment_expression,
        elvis: right "?:" => elvis_expression,
        left "||" => binary_expression,
        left "&&" => binary_expression,
        eq: left "==" | "!=" | "===" | "!==" => binary_expression,
        rel: left "<" | ">" | "<=" | ">=" => binary_expression,
        named: left "in" | NOT_IN => binary_expression,
        type_test: postfix "is" _type => is_expression,
        postfix NOT_IS _type => is_expression,
        cast: postfix "as" ["?"] _type => as_expression,
        range: left ".." | "..<" => range_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        unary: prefix "-" | "+" | "!" | "~" => unary_expression,
        prefix "++" | "--" => prefix_expression,
        postfix "++" | "--" => postfix_expression,
        postfix arguments => call_expression,
        postfix &(type_arguments arguments) type_arguments arguments => call_expression,
        postfix &(type_arguments lambda_literal) type_arguments lambda_literal => call_expression,
        postfix lambda_literal => call_expression,
        postfix subscript => index_expression,
        postfix "." _name => navigation_expression,
        postfix "?." _name => navigation_expression,
        postfix "::" _name => callable_reference,
        postfix BANG_BANG => not_null_expression,
    },

    _primary:
        _literal
        | _name
        | "this"
        | "super"
        | parenthesized_expression
        | lambda_literal
        | object_expression
        | if_statement
        | when_statement
        | try_statement,
    _literal: INTEGER | FLOAT | CHAR | STRING | RAW_STRING | "true" | "false" | "null",
    subscript: "[" _expression ("," _expression)* "]",
    lambda_literal: "{" lambda_body "}",
    lambda_body: &(_lambda_parameters "->") _lambda_parameters "->" _block_item* | lambda_items,
    lambda_items: _block_item+,
    _lambda_parameters: _lambda_parameter ("," _lambda_parameter)*,
    _lambda_parameter: destructuring_parameter | _name [":" _type],
    destructuring_parameter: "(" (_name %? ",") ")",
    object_expression: "object" [":" supertypes] [class_body],

    // ---- types ----

    _type: precedence _type_head {
        postfix "?" => nullable_type,
        postfix type_arguments => generic_type,
        postfix "." _name => qualified_type,
    },
    _type_head: _name | function_type,
    function_type: function_parameters "->" _type,
    function_parameters: "(" (_type %? ",") ")",

    _name:
        IDENT | "abstract" | "actual" | "annotation" | "by" | "companion" | "const"
        | "constructor" | "context" | "crossinline" | "data" | "delegate" | "dynamic" | "enum"
        | "expect" | "external" | "field" | "final" | "get" | "infix" | "init" | "inline"
        | "inner" | "internal" | "lateinit" | "noinline" | "open" | "operator" | "out"
        | "override" | "private" | "protected" | "public" | "reified" | "sealed" | "set"
        | "suspend" | "tailrec" | "value" | "vararg" | "where",
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    if c.pos() == 0 {
        DEPTH.with(|depth| depth.set(0));
    }
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => {
            let mut line_break = false;
            let mut anonymous = false;
            c.eat_while(|byte| {
                line_break |= byte == b'\n';
                anonymous |= byte == b'\r';
                matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
            });
            if (line_break || anonymous)
                && DEPTH.with(|depth| depth.get()) == 0
                && ends_statement(c.previous())
                && !continues(c)
            {
                LINE_END
            } else {
                WHITESPACE
            }
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
            // Kotlin block comments nest.
            let mut depth = 1u32;
            c.bump_n(2);
            while !c.at_end() && depth > 0 {
                if c.starts_with(b"/*") {
                    depth += 1;
                    c.bump_n(2);
                } else if c.starts_with(b"*/") {
                    depth -= 1;
                    c.bump_n(2);
                } else {
                    c.bump();
                }
            }
            COMMENT
        }
        b'"' if c.starts_with(b"\"\"\"") => {
            if raw_string(c) { RAW_STRING } else { BAD_LITERAL }
        }
        b'"' => {
            c.bump();
            if string_body(c) { STRING } else { BAD_LITERAL }
        }
        b'\'' => {
            c.bump();
            if c.quoted(b'\'', true) { CHAR } else { BAD_LITERAL }
        }
        b'`' => {
            c.bump();
            if c.eat_while(|byte| byte != b'`' && byte != b'\n') > 0 && c.eat(b'`') {
                IDENT
            } else {
                c.eat_while(|byte| byte != b'\n');
                BAD_LITERAL
            }
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

/// Whether a line break after a token of this kind may end a statement.
fn ends_statement(kind: Kind) -> bool {
    matches!(
        kind,
        IDENT | INTEGER | FLOAT | CHAR | STRING | RAW_STRING | BAD_LITERAL | RPAREN
            | RBRACKET | RBRACE | KW_RETURN | KW_BREAK | KW_CONTINUE | KW_THIS | KW_SUPER
            | KW_NULL | KW_TRUE | KW_FALSE | INCREMENT | DECREMENT | BANG_BANG
    )
}

/// Whether the token at the cursor continues the previous line, so that no
/// statement ends before it.
fn continues(c: &Cursor) -> bool {
    match c.peek() {
        b'.' | b'?' | b':' | b',' | b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>'
        | b'!' | b'&' | b'|' | b')' | b']' | b'}' | b';' | b'{' => true,
        byte if is_word_start(byte) => matches!(
            word_at(c.rest()),
            b"else" | b"catch" | b"finally" | b"where" | b"by" | b"as" | b"is" | b"in"
        ),
        _ => false,
    }
}

fn word_at(rest: &[u8]) -> &[u8] {
    let end = rest.iter().position(|byte| !is_word_byte(*byte)).unwrap_or(rest.len());
    &rest[..end]
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"as" => KW_AS,
        b"break" => KW_BREAK,
        b"class" => KW_CLASS,
        b"continue" => KW_CONTINUE,
        b"do" => KW_DO,
        b"else" => KW_ELSE,
        b"false" => KW_FALSE,
        b"for" => KW_FOR,
        b"fun" => KW_FUN,
        b"if" => KW_IF,
        b"in" => KW_IN,
        b"interface" => KW_INTERFACE,
        b"is" => KW_IS,
        b"null" => KW_NULL,
        b"object" => KW_OBJECT,
        b"package" => KW_PACKAGE,
        b"return" => KW_RETURN,
        b"super" => KW_SUPER,
        b"this" => KW_THIS,
        b"throw" => KW_THROW,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"typealias" => KW_TYPEALIAS,
        b"val" => KW_VAL,
        b"var" => KW_VAR,
        b"when" => KW_WHEN,
        b"while" => KW_WHILE,
        b"import" => KW_IMPORT,
        b"finally" => KW_FINALLY,
        b"catch" => KW_CATCH,
        b"abstract" => KW_ABSTRACT,
        b"actual" => KW_ACTUAL,
        b"annotation" => KW_ANNOTATION,
        b"by" => KW_BY,
        b"companion" => KW_COMPANION,
        b"const" => KW_CONST,
        b"constructor" => KW_CONSTRUCTOR,
        b"context" => KW_CONTEXT,
        b"crossinline" => KW_CROSSINLINE,
        b"data" => KW_DATA,
        b"delegate" => KW_DELEGATE,
        b"dynamic" => KW_DYNAMIC,
        b"enum" => KW_ENUM,
        b"expect" => KW_EXPECT,
        b"external" => KW_EXTERNAL,
        b"field" => KW_FIELD,
        b"final" => KW_FINAL,
        b"get" => KW_GET,
        b"infix" => KW_INFIX,
        b"init" => KW_INIT,
        b"inline" => KW_INLINE,
        b"inner" => KW_INNER,
        b"internal" => KW_INTERNAL,
        b"lateinit" => KW_LATEINIT,
        b"noinline" => KW_NOINLINE,
        b"open" => KW_OPEN,
        b"operator" => KW_OPERATOR,
        b"out" => KW_OUT,
        b"override" => KW_OVERRIDE,
        b"private" => KW_PRIVATE,
        b"protected" => KW_PROTECTED,
        b"public" => KW_PUBLIC,
        b"reified" => KW_REIFIED,
        b"sealed" => KW_SEALED,
        b"set" => KW_SET,
        b"suspend" => KW_SUSPEND,
        b"tailrec" => KW_TAILREC,
        b"value" => KW_VALUE,
        b"vararg" => KW_VARARG,
        b"where" => KW_WHERE,
        _ => IDENT,
    }
}

/// A one-line string after its opening quote, with `${...}` and `$name`
/// templates skipped. False if the line ends first.
fn string_body(c: &mut Cursor) -> bool {
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'"' => {
                c.bump();
                return true;
            }
            b'\n' | b'\r' => return false,
            b'\\' => c.bump_n(2),
            b'$' if c.peek_at(1) == b'{' => {
                c.bump_n(2);
                if !template(c) {
                    return false;
                }
            }
            _ => c.bump(),
        }
    }
}

/// The code of a `${...}` template after its brace, with strings skipped and
/// nested braces balanced. False if it never closes.
fn template(c: &mut Cursor) -> bool {
    let mut depth = 1u32;
    while !c.at_end() {
        match c.peek() {
            b'{' => {
                depth += 1;
                c.bump();
            }
            b'}' => {
                depth -= 1;
                c.bump();
                if depth == 0 {
                    return true;
                }
            }
            b'"' => {
                if c.starts_with(b"\"\"\"") {
                    if !raw_string(c) {
                        return false;
                    }
                } else {
                    c.bump();
                    if !string_body(c) {
                        return false;
                    }
                }
            }
            b'\'' => {
                c.bump();
                if !c.quoted(b'\'', true) {
                    return false;
                }
            }
            _ => c.bump(),
        }
    }
    false
}

/// A raw string from its first `"""`, to the closing `"""`.
fn raw_string(c: &mut Cursor) -> bool {
    c.bump_n(3);
    loop {
        if c.at_end() {
            return false;
        }
        if c.starts_with(b"\"\"\"") {
            c.bump_n(3);
            return true;
        }
        if c.peek() == b'$' && c.peek_at(1) == b'{' {
            c.bump_n(2);
            if !template(c) {
                return false;
            }
        } else {
            c.bump();
        }
    }
}

/// Decimal, hexadecimal and binary numbers with `_` separators, fractions and
/// exponents, and the `f`, `F`, `L` and `u` suffixes.
fn number(c: &mut Cursor) -> Kind {
    let mut kind = INTEGER;
    let hex = c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X');
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B') {
        c.bump_n(2);
    }
    let digits = |c: &mut Cursor| {
        if hex {
            c.eat_while(|byte| byte.is_ascii_hexdigit() || byte == b'_');
        } else {
            c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
        }
    };
    digits(c);
    if c.peek() == b'.' && !is_word_start(c.peek_at(1)) && c.peek_at(1) != b'.' {
        kind = FLOAT;
        c.bump();
        digits(c);
    }
    if matches!(c.peek(), b'e' | b'E') && (c.peek_at(1).is_ascii_digit() || (matches!(c.peek_at(1), b'+' | b'-') && c.peek_at(2).is_ascii_digit())) {
        kind = FLOAT;
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    match c.peek() {
        b'f' | b'F' => {
            c.bump();
            FLOAT
        }
        b'u' | b'U' => {
            c.bump();
            kind
        }
        b'L' => {
            c.bump();
            kind
        }
        _ => kind,
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
        (b'.', b'.', b'<') => (RANGE_UNTIL, 3),
        (b'.', b'.', _) => (RANGE, 2),
        (b'.', _, _) => (DOT, 1),
        (b'?', b'.', _) => (SAFE_CALL, 2),
        (b'?', b':', _) => (ELVIS, 2),
        (b'?', _, _) => (QUESTION, 1),
        (b'!', b'!', _) => (BANG_BANG, 2),
        (b'!', b'=', b'=') => (STRICT_NE, 3),
        (b'!', b'=', _) => (NE, 2),
        (b'!', b'i', b'n') if !is_word_byte(c.peek_at(3)) => (NOT_IN, 3),
        (b'!', b'i', b's') if !is_word_byte(c.peek_at(3)) => (NOT_IS, 3),
        (b'!', _, _) => (BANG, 1),
        (b':', b':', _) => (COLONCOLON, 2),
        (b':', _, _) => (COLON, 1),
        (b'-', b'>', _) => (ARROW, 2),
        (b'=', b'=', b'=') => (STRICT_EQ, 3),
        (b'=', b'=', _) => (EQ, 2),
        (b'=', _, _) => (ASSIGN, 1),
        (b'<', b'=', _) => (LE, 2),
        (b'<', _, _) => (LT, 1),
        (b'>', b'>', _) => (GT_JOINED, 1),
        (b'>', b'=', _) => (GE, 2),
        (b'>', _, _) => (GT, 1),
        (b'@', _, _) => (AT, 1),
        (b'~', _, _) => (TILDE, 1),
        (b'+', b'+', _) => (INCREMENT, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'+', _, _) => (PLUS, 1),
        (b'-', b'-', _) => (DECREMENT, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'-', _, _) => (MINUS, 1),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'*', _, _) => (STAR, 1),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'/', _, _) => (SLASH, 1),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'%', _, _) => (PERCENT, 1),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'&', _, _) => (AMP, 1),
        (b'|', b'|', _) => (OROR, 2),
        (b'|', _, _) => (PIPE, 1),
        (b'^', _, _) => (CARET, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    // Parenthesis depth, for the newline rule. Braces are not counted: a
    // newline inside a class body still ends statements.
    match kind {
        LPAREN | LBRACKET => DEPTH.with(|depth| depth.set(depth.get() + 1)),
        RPAREN | RBRACKET => DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1))),
        _ => {}
    }
    c.bump_n(width);
    kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_a::fixtures;

    const VALID: &[(&str, &str)] = &[
        ("Cache.kt", "package_header import_header import_header import_header class_declaration"),
        ("Extensions.kt", "package_header import_header function_declaration function_declaration function_declaration function_declaration function_declaration"),
        ("Main.kt", "package_header import_header import_header import_header property_declaration function_declaration function_declaration function_declaration"),
        ("Models.kt", "package_header import_header enum_class_declaration class_declaration interface_declaration typealias_declaration"),
        ("Parser.kt", "package_header class_declaration class_declaration"),
        ("Repository.kt", "package_header import_header interface_declaration class_declaration class_declaration"),
        ("Routes.kt", "package_header import_header import_header class_declaration class_declaration"),
        ("Service.kt", "package_header import_header import_header class_declaration"),
    ];

    const MALFORMED: &[(&str, &str)] = &[
        ("broken_signature.kt", "package_header class_declaration"),
        ("unclosed_brace.kt", "package_header class_declaration"),
        ("unterminated_string.kt", "package_header class_declaration"),
    ];

    #[test]
    fn kotlin_fixtures_parse_cleanly() {
        fixtures::valid("kotlin", &KOTLIN, VALID);
    }

    #[test]
    fn kotlin_malformed_fixtures_keep_the_root_and_report_inside_it() {
        fixtures::malformed("kotlin", &KOTLIN, MALFORMED);
    }

    #[test]
    fn kotlin_lexer_infers_statement_ends() {
        let kinds = fixtures::tokens(&KOTLIN, "val a = 1\nval b = 2 + 3\n");
        assert_eq!(kinds.iter().filter(|kind| **kind == "line_end").count(), 2);
        let kinds = fixtures::tokens(&KOTLIN, "foo(\n  a,\n  b\n)\n");
        assert_eq!(kinds.iter().filter(|kind| **kind == "line_end").count(), 1);
        assert!(fixtures::clean(&KOTLIN, "fun f() = 1\n"));
        assert!(!fixtures::clean(&KOTLIN, "fun f( { }\n"));
    }

}
