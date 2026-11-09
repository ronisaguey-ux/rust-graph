//! C through `grammar!`: translation unit, declarations, declarators,
//! statements and one precedence table. The preprocessor is trivia (see
//! `mod.rs`); braces are plain tokens so a predicate can find the `{` that
//! makes a definition.
//!
//! What real translation units need, and how it is decided without a symbol
//! table:
//!
//! - **A typedef name is a type by shape.** `Foo x;` is two names in a row and
//!   no expression can be that, so it is a declaration (`bare_user_declaration`);
//!   `Foo *x;` is a declaration only at file scope (`user_pointer_declaration`),
//!   because inside a function it must stay the multiplication `x * y`. A
//!   keyword in front settles it: `const Foo *p;` is a declaration.
//! - **A macro before a declaration is a qualifier.** `NPY_NO_EXPORT int f(void);`
//!   and `XMLPUBFUN xmlRegexpPtr f(...)` are read as `macro_qualified_item`,
//!   which only matches when a whole declaration follows the name.
//! - **A macro with no declarator is a field.** `PyObject_HEAD` inside a struct
//!   is `macro_field`; the predicate above it claims every name that does have
//!   a declarator, so the two cannot be confused.
//!
//! Not modelled: `_Generic`, compound literals, K&R parameter declarations, a
//! pointer declarator of a typedef name inside a function body (it reads as a
//! multiplication, which is the older and still legal reading), and operator
//! spelling other than the punctuation forms. Everything listed here as
//! modelled has a fixture or a test.

use crate::batch_c::{directive, is_word_byte, is_word_start, literal_prefix, number, quoted, trivia};
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, UNKNOWN};

rust_graph_deep::grammar! { C {
    name = "c";
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
        /// One preprocessor line, with its continuations: trivia.
        DIRECTIVE = "preprocessor" trivia,

        KW_AUTO = "auto",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CHAR = "char",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DEFAULT = "default",
        KW_DO = "do",
        KW_DOUBLE = "double",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXTERN = "extern",
        KW_FLOAT = "float",
        KW_FOR = "for",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_INLINE = "inline",
        KW_INT = "int",
        KW_LONG = "long",
        KW_REGISTER = "register",
        KW_RESTRICT = "restrict",
        KW_RETURN = "return",
        KW_SHORT = "short",
        KW_SIGNED = "signed",
        KW_SIZEOF = "sizeof",
        KW_STATIC = "static",
        KW_STRUCT = "struct",
        KW_SWITCH = "switch",
        KW_TYPEDEF = "typedef",
        KW_UNION = "union",
        KW_UNSIGNED = "unsigned",
        KW_VOID = "void",
        KW_VOLATILE = "volatile",
        KW_WHILE = "while",
        KW_BOOL = "_Bool",
        KW_COMPLEX = "_Complex",
        KW_IMAGINARY = "_Imaginary",
        KW_ALIGNAS = "_Alignas",
        KW_ALIGNOF = "_Alignof",
        KW_ATOMIC = "_Atomic",
        KW_NORETURN = "_Noreturn",
        KW_STATIC_ASSERT = "_Static_assert",
        KW_THREAD_LOCAL = "_Thread_local",
        /// GCC's attribute introducer: `__attribute__((packed))`.
        KW_ATTRIBUTE = "__attribute__",
        /// MSVC's: `__declspec(dllexport)`.
        KW_DECLSPEC = "__declspec",
        /// GCC's "ignore the following construct's extension warning".
        KW_EXTENSION = "__extension__",
        /// `asm`, `__asm`, `__asm__`: one inline-assembly statement.
        KW_ASM = "asm",
        /// `typeof`, `__typeof`, `__typeof__`: the GNU type-of operator.
        KW_TYPEOF = "typeof",

        COMMA = ",",
        SEMI = ";",
        DOT = ".",
        ELLIPSIS = "...",
        QUESTION = "?",
        COLON = ":",
        ARROW = "->",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        /// A `>` directly before another `>`: the first of the pair, so a
        /// template closer and a shift can both use it.
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
        &(declaration_specifiers _declarator "{") function_definition
        | &(declaration_specifiers user_type_name _declarator "{") user_function_definition
        | &(user_type_name _declarator "{") bare_user_function_definition
        | &("extern" string) linkage_specification
        | &(IDENT IDENT _qualified_tail) double_qualified_item
        | &(IDENT _qualified_tail) macro_qualified_item
        | &(declaration_specifiers IDENT _qualified_tail) specified_macro_item
        | &(IDENT parameter_list "{") implicit_function
        | &(user_type_name "*") user_pointer_declaration
        | declaration_statement
        | static_assert_statement
        | asm_statement
        | ";"
        | macro_call_statement,

    function_definition: declaration_specifiers _declarator compound_statement,
    /// A definition whose return type is a typedef name: `PyObject *f(void) {`.
    user_function_definition: declaration_specifiers user_type_name _declarator compound_statement,
    bare_user_function_definition: user_type_name _declarator compound_statement,
    /// `PyDoc_STRVAR(name, "...");`: a macro that expands to a declaration, at
    /// file scope. Nothing else is a statement there, so this is the only
    /// expression-shaped form the root accepts.
    macro_call_statement: IDENT arguments ";",
    /// `extern "C" { ... }` and `extern "C" int f(void);`: a linkage block.
    linkage_specification: "extern" string ("{" _top_item* "}" | _top_item),
    /// A visibility macro in front of a declaration: `NPY_NO_EXPORT int f(void);`,
    /// `XMLPUBFUN xmlRegexpPtr xmlRegexpCompile(...)`. The guard above claims
    /// the name only when a whole declaration follows it, so a macro *call*
    /// statement at file scope is still an expression.
    macro_qualified_item: IDENT _qualified_tail,
    /// `XML_DEPRECATED XMLPUBFUN xmlRegexpPtr f(void);`: two visibility macros.
    double_qualified_item: IDENT IDENT _qualified_tail,
    /// `extern NPY_NO_EXPORT PyTypeObject PyUFunc_Type;`: the macro comes after
    /// a storage class. The guard above keeps it out of the way of every plain
    /// declaration, and marks the name where no declarator follows.
    specified_macro_item: declaration_specifiers IDENT _qualified_tail,
    _qualified_tail:
        &(declaration_specifiers _declarator "{") function_definition
        | &(declaration_specifiers user_type_name _declarator "{") user_function_definition
        | &(user_type_name _declarator "{") bare_user_function_definition
        | &(IDENT parameter_list "{") implicit_function
        | &(user_type_name "*") user_pointer_declaration
        | declaration_statement
        | static_assert_statement,
    /// A definition whose return type is supplied by the macro above it:
    /// `PyMODINIT_FUNC ENTRY_FUNC_NAME(void) { ... }`.
    implicit_function: IDENT parameter_list compound_statement,

    declaration_statement:
        &(declaration_specifiers user_type_name ("*" | IDENT)) specified_user_declaration
        | &("typedef" user_type_name _declarator) typedef_user_declaration
        | &(user_type_name IDENT) bare_user_declaration
        | _default_declaration,
    /// `static const Foo *f(void);`: a keyword-qualified typedef name.
    specified_user_declaration: declaration_specifiers user_type_name (_init_declarator % ",") ";",
    /// `buffer *buffer_new(int size);`: a typedef name and a pointer declarator.
    /// Only file scope accepts this: inside a function `x * y;` must stay a
    /// multiplication.
    user_pointer_declaration: user_type_name _pointer_named_declarator ("," _pointer_named_declarator)* ";",
    _pointer_named_declarator: pointer* IDENT _declarator_suffix* attribute* ["=" _initializer],
    /// `typedef Foo (*handler)(void);`: a typedef whose type is a typedef name.
    typedef_user_declaration: "typedef" user_type_name _init_declarator ("," _init_declarator)* ";",
    /// `Foo x;`: a typedef name and a named declarator. It never starts with a
    /// `*`, so `x * y;` in a function stays a multiplication.
    bare_user_declaration: user_type_name _named_init_declarator ("," _named_init_declarator)* ";",
    _named_init_declarator: IDENT _declarator_suffix* attribute* ["=" _initializer],
    _default_declaration: declaration_specifiers (_init_declarator % ",") ";",
    /// A typedef name where a type specifier is expected.
    user_type_name: IDENT,
    _init_declarator: _declarator attribute* ["=" _initializer],
    _initializer: designated_field | designated_index | _expression@assign | initializer_list,
    initializer_list: "{" (_initializer %? ",") "}",
    /// C99 `.name = value` inside an initializer list.
    designated_field: "." IDENT "=" _initializer,
    /// C99 `[index] = value` inside an initializer list.
    designated_index: array_suffix "=" _initializer,

    static_assert_statement: "_Static_assert" static_assert_arguments ";",
    static_assert_arguments: "(" _expression ["," string] ")",

    /// `__asm__ __volatile__ ("..." : "=m"(x) : : "memory");`. The template and
    /// each operand are string literals, so the colons are separators rather
    /// than punctuation the expression grammar would have to swallow.
    asm_statement: "asm" _asm_qualifier* asm_body ";",
    _asm_qualifier: "volatile" | "inline" | "goto",
    asm_body: "(" [asm_template] _asm_operand_list* ")",
    /// A template is string literals with macros spliced between them:
    /// `"addu "REG_FP",%0\n"`.
    asm_template: (string | IDENT)+,
    _asm_operand_list: ":" (_asm_operand %? ","),
    _asm_operand: [asm_operand_name] (string | IDENT) [asm_operand_expression],
    asm_operand_name: "[" IDENT "]",
    asm_operand_expression: "(" _expression ")",

    declaration_specifiers: _declaration_specifier+,
    _declaration_specifier:
        "typedef" | "extern" | "static" | "auto" | "register" | "inline" | "_Noreturn"
        | "_Thread_local" | "const" | "volatile" | "restrict" | "_Atomic"
        | "_Alignas" alignment_arguments
        | "__extension__"
        | attribute
        | type_specifier,
    alignment_arguments: "(" _expression ")",

    /// `__attribute__((packed, aligned(4)))` and `__declspec(dllexport)`.
    attribute: ("__attribute__" | "__declspec") attribute_arguments,
    attribute_arguments: "(" attribute_body ")",
    attribute_body: attribute_group | _attribute_argument ("," _attribute_argument)*,
    attribute_group: "(" (_attribute_argument %? ",") ")",
    /// An attribute argument is a name with an optional call: `packed`,
    /// `aligned(4)`, `optimize("O2")`. It never starts with `(`, so the group
    /// above is told apart from a bare list in one token.
    _attribute_argument: IDENT [arguments],

    type_specifier:
        "void" | "char" | "short" | "int" | "long" | "float" | "double"
        | "signed" | "unsigned" | "_Bool" | "_Complex" | "_Imaginary"
        | typeof_specifier
        | struct_or_union_specifier
        | enum_specifier,
    typeof_specifier: "typeof" typeof_arguments,
    typeof_arguments: "(" _expression ")",

    struct_or_union_specifier: ("struct" | "union") [IDENT] [struct_body],
    struct_body: "{" (_struct_declaration | ";")* "}",
    _struct_declaration:
        &(user_type_name _member_declarator_list ";") user_struct_declaration
        | declaration_specifiers [user_type_name] (_struct_declarator % ",") ";"
        | macro_field,
    /// `PyObject *weakreflist;`: a typedef name and its declarators. A struct
    /// body holds declarations only, so the `x * y;` ambiguity does not exist
    /// here and the bare name is safe to accept.
    user_struct_declaration: user_type_name _member_declarator_list ";",
    _member_declarator_list: _member_declarator ("," _member_declarator)*,
    /// A field macro that expands to whole members and has no declarator and
    /// no semicolon of its own: `PyObject_HEAD`. The predicate above claims
    /// every name that does have one, so this is the only reading left.
    macro_field: IDENT,
    /// A struct member's declarator: no parameter list, so a bare field macro
    /// can never be mistaken for the start of `Foo Bar(...)`.
    _member_declarator: pointer* _member_direct attribute* [trailing_macro] [":" _expression@assign],
    /// A deprecation macro after a member's declarator: `char **names
    /// XML_DEPRECATED_MEMBER;`.
    trailing_macro: IDENT,
    _member_direct: IDENT array_suffix* | parenthesized_declarator _declarator_suffix*,
    _struct_declarator: _declarator attribute* [":" _expression@assign],

    enum_specifier: "enum" [IDENT] [enum_body],
    enum_body: "{" (_enumerator %? ",") "}",
    _enumerator: IDENT ["=" _expression@assign],

    _declarator: pointer* direct_declarator,
    pointer: "*" _pointer_qualifier*,
    _pointer_qualifier: "const" | "volatile" | "restrict",
    direct_declarator: _direct_base _declarator_suffix*,
    _direct_base: IDENT | parenthesized_declarator,
    parenthesized_declarator: "(" _declarator ")",
    _declarator_suffix: array_suffix | parameter_list,
    array_suffix: "[" [_expression] "]",
    parameter_list: "(" (_parameter % ",") ")",
    _parameter:
        declaration_specifiers [user_type_name] [_declarator_or_abstract]
        | user_parameter
        | "...",
    /// `Foo *self`, `Foo*`, `Foo`: a typedef name and either a named or an
    /// abstract declarator.
    user_parameter: user_type_name _declarator_or_abstract,
    _declarator_or_abstract: pointer* [direct_declarator] abstract_suffix*,

    /// `*`, `**`, `*(*)(void)`: pointers, an optional parenthesised part and
    /// suffixes. It always starts with a `*`, so a `(` after a type id is still
    /// an argument list (`new T(5)`, `int f(void)`).
    _abstract_declarator: pointer+ [parenthesized_abstract] abstract_suffix*,
    parenthesized_abstract: "(" _abstract_declarator ")",
    abstract_suffix: array_suffix | parameter_list,
    type_name: declaration_specifiers [user_type_name] [_type_abstract_declarator],
    /// In a type name a parenthesised part may come first: `int (*)(void)` is
    /// the type of a function pointer, so `sizeof(int (*)(void))` parses.
    _type_abstract_declarator:
        pointer+ [parenthesized_abstract] abstract_suffix*
        | parenthesized_abstract abstract_suffix*,

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
        | static_assert_statement
        | asm_statement
        | declaration_statement
        | expression_statement,

    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    switch_statement: "switch" parenthesized_expression switch_body,
    switch_body: "{" _switch_item* "}",
    _switch_item: case_statement | default_statement | _statement,
    case_statement: "case" _expression@assign ":" _statement*,
    default_statement: "default" ":" _statement*,
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: "for" for_header _statement,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression] ")",
    _for_init: _for_declaration | _expression,
    _for_declaration: declaration_specifiers (_init_declarator % ","),
    parenthesized_expression: "(" _expression ")",
    return_statement: "return" [_expression] ";",
    break_statement: "break" ";",
    continue_statement: "continue" ";",
    goto_statement: "goto" (IDENT | "*" _expression) ";",
    labeled_statement: IDENT ":" _statement,
    expression_statement: _expression ";",

    _expression: precedence _primary {
        comma: left "," => sequence_expression,
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" => assignment_expression,
        cond: postfix "?" _expression@assign ":" _expression@assign => conditional_expression,
        lor: left "||" => binary_expression,
        land: left "&&" => binary_expression,
        bor: left "|" => binary_expression,
        bxor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        eq: left "==" | "!=" => binary_expression,
        rel: left "<" | ">" | "<=" | ">=" => binary_expression,
        shift: left "<<" | GT_JOINED ">" => binary_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        unary: prefix "!" | "~" | "-" | "+" | "*" | "&" => unary_expression,
        prefix "++" | "--" => update_expression,
        postfix "++" | "--" => update_expression,
        postfix arguments => call_expression,
        postfix subscript => subscript_expression,
        postfix "." _member => field_expression,
        postfix "->" _member => field_expression,
    },

    _primary:
        &(cast_type) cast_expression
        | &(user_cast_head (IDENT | NUMBER | STR | CHAR_LITERAL)) user_cast_expression
        | sizeof_expression
        | alignof_expression
        | parenthesized_expression
        | string
        | _name
        | NUMBER
        | CHAR_LITERAL,

    _name: IDENT,
    _member: IDENT,
    string: STR+,
    cast_expression: cast_type _expression@unary,
    /// `(PyCFunction) f`: a cast to a typedef name. It is read as a cast only
    /// when an operand that cannot be a binary operator follows, so `(a) * b`
    /// stays a multiplication.
    user_cast_expression: user_cast_head _expression@unary,
    user_cast_head: "(" user_type_name ")",

    cast_type: "(" _cast_type_body ")",
    /// `(int *)`, `(struct s *)` and `(Foo *)`: a keyword type or a typedef name.
    _cast_type_body: type_name | _user_cast_type,
    /// A cast to a typedef name only counts when a `*` or a `(...)` follows it:
    /// `(Foo)` alone is a parenthesised expression, as it has always been.
    _user_cast_type: user_type_name _nonempty_abstract_declarator,
    _nonempty_abstract_declarator:
        pointer+ [parenthesized_abstract] abstract_suffix*
        | parenthesized_abstract abstract_suffix*,
    sizeof_expression: "sizeof" sizeof_operand,
    sizeof_operand: &(cast_type) cast_type | _expression@unary,
    alignof_expression: "_Alignof" cast_type,
    arguments: "(" (_argument %? ",") ")",
    _argument: _expression@assign,
    subscript: "[" _expression "]",
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    if let Some(kind) = trivia(c) {
        return kind;
    }
    if directive(c) {
        return DIRECTIVE;
    }
    match c.peek() {
        b'"' => {
            c.bump();
            if quoted(c, b'"') { STR } else { BAD_LITERAL }
        }
        b'\'' => {
            c.bump();
            if quoted(c, b'\'') { CHAR_LITERAL } else { BAD_LITERAL }
        }
        b'0'..=b'9' => {
            number(c, false);
            NUMBER
        }
        b'.' if c.peek_at(1).is_ascii_digit() => {
            number(c, false);
            NUMBER
        }
        byte if is_word_start(byte) => word(c),
        _ => operator(c),
    }
}

/// A word, or a literal with one of the C prefixes (`L"..."`, `u8"..."`).
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    if matches!(c.peek(), b'"' | b'\'') {
        if literal_prefix(text).is_some() {
            let quote = c.peek();
            c.bump();
            let closed = quoted(c, quote);
            return match (closed, quote) {
                (false, _) => BAD_LITERAL,
                (true, b'"') => STR,
                (true, _) => CHAR_LITERAL,
            };
        }
    }
    keyword(text)
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        // The alternate spellings GCC and MSVC add for the same keywords: they
        // are the same token, so a grammar rule never has to name both.
        b"__inline" | b"__inline__" => KW_INLINE,
        b"__restrict" | b"__restrict__" => KW_RESTRICT,
        b"__const" | b"__const__" => KW_CONST,
        b"__volatile" | b"__volatile__" => KW_VOLATILE,
        b"__signed" | b"__signed__" => KW_SIGNED,
        b"__thread" => KW_THREAD_LOCAL,
        b"__attribute" | b"__attribute__" => KW_ATTRIBUTE,
        b"__declspec" => KW_DECLSPEC,
        b"__extension__" => KW_EXTENSION,
        b"asm" | b"__asm" | b"__asm__" => KW_ASM,
        b"typeof" | b"__typeof" | b"__typeof__" => KW_TYPEOF,
        b"auto" => KW_AUTO,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"char" => KW_CHAR,
        b"const" => KW_CONST,
        b"continue" => KW_CONTINUE,
        b"default" => KW_DEFAULT,
        b"do" => KW_DO,
        b"double" => KW_DOUBLE,
        b"else" => KW_ELSE,
        b"enum" => KW_ENUM,
        b"extern" => KW_EXTERN,
        b"float" => KW_FLOAT,
        b"for" => KW_FOR,
        b"goto" => KW_GOTO,
        b"if" => KW_IF,
        b"inline" => KW_INLINE,
        b"int" => KW_INT,
        b"long" => KW_LONG,
        b"register" => KW_REGISTER,
        b"restrict" => KW_RESTRICT,
        b"return" => KW_RETURN,
        b"short" => KW_SHORT,
        b"signed" => KW_SIGNED,
        b"sizeof" => KW_SIZEOF,
        b"static" => KW_STATIC,
        b"struct" => KW_STRUCT,
        b"switch" => KW_SWITCH,
        b"typedef" => KW_TYPEDEF,
        b"union" => KW_UNION,
        b"unsigned" => KW_UNSIGNED,
        b"void" => KW_VOID,
        b"volatile" => KW_VOLATILE,
        b"while" => KW_WHILE,
        b"_Bool" => KW_BOOL,
        b"_Complex" => KW_COMPLEX,
        b"_Imaginary" => KW_IMAGINARY,
        b"_Alignas" => KW_ALIGNAS,
        b"_Alignof" => KW_ALIGNOF,
        b"_Atomic" => KW_ATOMIC,
        b"_Noreturn" => KW_NORETURN,
        b"_Static_assert" => KW_STATIC_ASSERT,
        b"_Thread_local" => KW_THREAD_LOCAL,
        _ => IDENT,
    }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'<', b'<', _) => (SHL, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'<', _, _) => (LT, 1),
        // A `>` before another is one token; the next step takes the second,
        // so `>>` is a pair and `>>=` is this plus `>=`.
        (b'>', b'>', _) => (GT_JOINED, 1),
        (b'>', b'=', _) => (GE, 2),
        (b'>', _, _) => (GT, 1),
        (b'=', b'=', _) => (EQ, 2),
        (b'=', _, _) => (ASSIGN, 1),
        (b'!', b'=', _) => (NE, 2),
        (b'!', _, _) => (BANG, 1),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'&', _, _) => (AMP, 1),
        (b'|', b'|', _) => (OROR, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'|', _, _) => (PIPE, 1),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'^', _, _) => (CARET, 1),
        (b'+', b'+', _) => (INCREMENT, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'+', _, _) => (PLUS, 1),
        (b'-', b'-', _) => (DECREMENT, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'-', _, _) => (MINUS, 1),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'*', _, _) => (STAR, 1),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'/', _, _) => (SLASH, 1),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'%', _, _) => (PERCENT, 1),
        (b'~', _, _) => (TILDE, 1),
        (b'(', _, _) => (LPAREN, 1),
        (b')', _, _) => (RPAREN, 1),
        (b'[', _, _) => (LBRACKET, 1),
        (b']', _, _) => (RBRACKET, 1),
        (b'{', _, _) => (LBRACE, 1),
        (b'}', _, _) => (RBRACE, 1),
        (b',', _, _) => (COMMA, 1),
        (b';', _, _) => (SEMI, 1),
        (b':', _, _) => (COLON, 1),
        (b'?', _, _) => (QUESTION, 1),
        (b'.', _, _) => (DOT, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    c.bump_n(width);
    kind
}

/// The corpus measurement: not part of the suite, run by hand on real code.
///
/// ```text
/// DEEP_C_CORPUS=/tmp/opencode/ccorpus cargo test -p deep-grammars --lib \
///   c::corpus::clean_files -- --ignored --nocapture
/// ```
#[cfg(test)]
mod corpus {
    use std::fs;

    use crate::batch_c::c::C;
    use crate::deep::syntax::ERROR;
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    /// Every file in the directory, as `(name, did it parse with no error node)`.
    fn scan(dir: &str, only: &[&str]) -> Vec<(String, bool, usize)> {
        let mut out = Vec::new();
        let mut names: Vec<_> = fs::read_dir(dir).expect("corpus dir").filter_map(Result::ok).map(|e| e.path()).collect();
        names.sort();
        for path in names {
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !only.contains(&extension) {
                continue;
            }
            let Ok(src) = fs::read(&path) else { continue };
            let mut workspace = Workspace::default();
            let Ok(outcome) = workspace.parse(&src, &C, &Budget::default()) else { continue };
            if !outcome.complete() {
                continue;
            }
            let nodes = &workspace.tree.nodes;
            let errors = nodes.iter().filter(|node| node.kind == ERROR || node.flags & HAS_ERROR != 0 && node.flags & TOKEN != 0).count();
            out.push((path.file_name().unwrap().to_string_lossy().into_owned(), errors == 0, errors));
        }
        out
    }

    /// The first error's own text plus the line it starts on, for clustering.
    fn detail(dir: &str, name: &str) -> String {
        let path = std::path::Path::new(dir).join(name);
        let Ok(src) = fs::read(&path) else { return String::new() };
        let mut workspace = Workspace::default();
        let Ok(_) = workspace.parse(&src, &C, &Budget::default()) else { return String::new() };
        let mut nodes: Vec<_> = workspace.tree.nodes.iter().filter(|node| node.kind == ERROR || node.flags & HAS_ERROR != 0).collect();
        nodes.sort_by_key(|node| (node.start, node.end));
        let Some(node) = nodes.first() else {
            return String::new();
        };
        let at = (node.start as usize).min(src.len());
        let end = (node.end as usize).min(src.len()).max(at);
        let start = src[..at].iter().rposition(|byte| *byte == b'\n').map_or(0, |at| at + 1);
        let line = String::from_utf8_lossy(&src[start..end.max(at)]).split_whitespace().collect::<Vec<_>>().join(" ");
        line.chars().take(80).collect()
    }

    /// One file's error regions, with the source each one covers.
    #[test]
    #[ignore]
    fn one_file() {
        let Ok(path) = std::env::var("DEEP_C_ONE") else { return };
        let src = fs::read(&path).expect("file");
        let mut workspace = Workspace::default();
        workspace.parse(&src, &C, &Budget::default()).expect("parse");
        let mut nodes: Vec<_> = workspace.tree.nodes.iter().filter(|node| node.kind == ERROR || node.flags & HAS_ERROR != 0).collect();
        nodes.sort_by_key(|node| (node.start, node.end));
        for node in nodes {
            let at = (node.start as usize).min(src.len());
            let end = (node.end as usize).min(src.len());
            let text = String::from_utf8_lossy(&src[at..end.max(at).min(at + 120)]);
            let line = src[..at].iter().filter(|byte| **byte == b'\n').count() + 1;
            println!("{}..{} L{} {:?}", at, end, line, text.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }

    #[test]
    #[ignore]
    fn clean_files() {
        let Ok(dir) = std::env::var("DEEP_C_CORPUS") else { return };
        let results = scan(&dir, &["c", "h"]);
        let clean = results.iter().filter(|(_, clean, _)| *clean).count();
        for (name, clean, errors) in &results {
            if !*clean {
                println!("FAIL {name} {errors} {}", detail(&dir, name));
            }
        }
        println!("C: {clean}/{} clean", results.len());
    }
}

/// The fixtures this grammar owns, and the construct each one proves: one node
/// kind and the exact source text it must span.
///
/// ```text
/// fixtures/c_*.c  ->  deep-grammars/src/batch_c/c.rs
/// ```
#[cfg(test)]
mod kinds {
    use std::fs;
    use std::path::PathBuf;

    use crate::batch_c::c::C;
    use crate::deep::syntax::{Kind, Language, ERROR, FIRST_LANGUAGE_KIND};
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    /// `(fixture, first node of this kind, exact text it spans)`.
    const FIXTURES: &[(&str, &[(&str, &str)])] = &[
        (
            "c_attributes.c",
            &[
                ("attribute", "__attribute__((noreturn))"),
                ("function_definition", "__attribute__((always_inline)) static inline int twice(int value) {\n    return value + value;\n}"),
            ],
        ),
        (
            "c_designated_initializers.c",
            &[
                ("designated_field", ".x = 0"),
                ("designated_index", "[1] = 7"),
                ("sequence_expression", "left = input, right = input + 1"),
            ],
        ),
        (
            "c_function_pointers.c",
            &[
                ("typedef_user_declaration", "typedef outcome_t (*outcome_handler)(int value);"),
                ("user_struct_declaration", "handler dispatch;"),
                ("user_cast_expression", "(handler) invoke"),
            ],
        ),
        (
            "c_inline_asm.c",
            &[
                ("asm_statement", "__asm__ volatile(\"mov %0, sp\" : \"=r\"(sp));"),
                ("asm_operand_expression", "(sp)"),
            ],
        ),
        (
            "c_linkage_block.c",
            &[(
                "linkage_specification",
                "extern \"C\" {\n#endif\n\nint widget_open(const char *name);\nint widget_close(int handle);\n\n#ifdef __cplusplus\n}",
            )],
        ),
        (
            "c_macro_field.c",
            &[("macro_field", "HEADER_FIELDS"), ("user_struct_declaration", "holder_t **names;")],
        ),
        (
            "c_macro_qualifiers.c",
            &[
                ("macro_qualified_item", "EXPORT int public_entry(int argc, char **argv);"),
                ("specified_macro_item", "static EXPORT int hidden_entry(void) {\n    return 0;\n}"),
            ],
        ),
        (
            "c_typedef_names.c",
            &[
                ("specified_user_declaration", "static const buffer *buffer_empty(void);"),
                ("user_pointer_declaration", "buffer *buffer_new(int size);"),
                ("function_definition", "int fill(void) {\n    buffer *slot;\n    const buffer *limit;\n    return slot == limit;\n}"),
            ],
        ),
    ];

    fn fixtures() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    fn kind_of(language: &'static Language, name: &str) -> Kind {
        let at = language
            .kinds
            .iter()
            .position(|info| info.name == name)
            .unwrap_or_else(|| panic!("the C grammar has no kind named {name}"));
        FIRST_LANGUAGE_KIND + at as Kind
    }

    #[test]
    fn every_c_fixture_parses_and_spans_the_construct_it_names() {
        let dir = fixtures();
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("c_") && name.ends_with(".c"))
            .collect();
        names.sort();
        let listed: Vec<String> = FIXTURES.iter().map(|(name, _)| (*name).to_string()).collect();
        assert_eq!(names, listed, "fixtures/: the table and the directory disagree");

        for (name, expectations) in FIXTURES {
            let path = dir.join(name);
            let src = fs::read(&path).unwrap();
            let mut workspace = Workspace::default();
            let outcome = workspace.parse(&src, &C, &Budget::default()).expect("too large");
            assert!(outcome.complete(), "{name}: the parse did not finish");
            let nodes = &workspace.tree.nodes;
            let errors = nodes.iter().filter(|node| node.kind == ERROR || node.flags & HAS_ERROR != 0 && node.flags & TOKEN != 0).count();
            assert_eq!(errors, 0, "{name}: {errors} error nodes:\n{}", workspace.tree.sexp(&C, true));
            for (kind_name, text) in *expectations {
                let kind = kind_of(&C, kind_name);
                let node = nodes.iter().find(|node| node.kind == kind).unwrap_or_else(|| panic!("{name}: no {kind_name} node"));
                let span = String::from_utf8_lossy(&src[node.start as usize..node.end as usize]).into_owned();
                assert_eq!(span, *text, "{name}: the first {kind_name} spans something else");
            }
        }
    }
}
