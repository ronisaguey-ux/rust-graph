//! C through `grammar!`: translation unit, declarations, declarators,
//! statements and one precedence table. The preprocessor is trivia (see
//! `mod.rs`); braces are plain tokens so a predicate can find the `{` that
//! makes a definition.
//!
//! Not modelled: typedef names (a bare typedef type reads as an expression
//! statement), `_Generic`, compound literals, K&R parameter declarations, and
//! attributes. Everything listed in the module docs as "modelled" has a
//! fixture or a test.

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
        | &("extern" string) linkage_specification
        | declaration_statement
        | static_assert_statement
        | asm_statement
        | ";"
        | expression_statement,

    function_definition: declaration_specifiers _declarator compound_statement,
    /// A definition whose return type is a typedef name: `PyObject *f(void) {`.
    user_function_definition: declaration_specifiers user_type_name _declarator compound_statement,
    /// `extern "C" { ... }` and `extern "C" int f(void);`: a linkage block.
    linkage_specification: "extern" string ("{" _top_item* "}" | _top_item),

    declaration_statement:
        &(declaration_specifiers user_type_name ("*" | IDENT)) specified_user_declaration
        | &(user_type_name IDENT) bare_user_declaration
        | _default_declaration,
    /// `static const Foo *f(void);`: a keyword-qualified typedef name.
    specified_user_declaration: declaration_specifiers user_type_name (_init_declarator % ",") ";",
    /// `Foo x;`: two names in a row, which no expression can be.
    bare_user_declaration: user_type_name (_init_declarator % ",") ";",
    _default_declaration: declaration_specifiers (_init_declarator % ",") ";",
    /// A typedef name where a type specifier is expected.
    user_type_name: IDENT,
    _init_declarator: _declarator _attribute* ["=" _initializer],
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
    asm_template: string,
    _asm_operand_list: ":" (_asm_operand %? ","),
    _asm_operand: string [asm_operand_expression],
    asm_operand_expression: "(" _expression ")",

    declaration_specifiers: _declaration_specifier+,
    _declaration_specifier:
        "typedef" | "extern" | "static" | "auto" | "register" | "inline" | "_Noreturn"
        | "_Thread_local" | "const" | "volatile" | "restrict" | "_Atomic"
        | "_Alignas" alignment_arguments
        | "__extension__"
        | _attribute
        | type_specifier,
    alignment_arguments: "(" _expression ")",

    /// `__attribute__((packed, aligned(4)))` and `__declspec(dllexport)`.
    _attribute: ("__attribute__" | "__declspec") attribute_arguments,
    attribute_arguments: "(" attribute_body ")",
    attribute_body: attribute_group | _attribute_argument % ",",
    attribute_group: "(" (_attribute_argument %? ",") ")",
    _attribute_argument: _expression@assign | parenthesized_expression,

    type_specifier:
        "void" | "char" | "short" | "int" | "long" | "float" | "double"
        | "signed" | "unsigned" | "_Bool" | "_Complex" | "_Imaginary"
        | typeof_specifier
        | struct_or_union_specifier
        | enum_specifier,
    typeof_specifier: "typeof" typeof_arguments,
    typeof_arguments: "(" _expression ")",

    struct_or_union_specifier: ("struct" | "union") [IDENT] [struct_body],
    struct_body: "{" (_struct_declaration | ";" | macro_field)* "}",
    _struct_declaration:
        &(declaration_specifiers user_type_name) specified_struct_declaration
        | user_struct_declaration
        | declaration_specifiers (_struct_declarator % ",") ";",
    /// `PyObject *weakreflist;`: a typedef name and its declarator. A struct
    /// body holds declarations only, so the `x * y;` ambiguity does not exist
    /// here and the bare name is safe to accept.
    user_struct_declaration: user_type_name (_struct_declarator % ",") ";",
    specified_struct_declaration: declaration_specifiers user_type_name (_struct_declarator % ",") ";",
    /// A field macro that expands to whole members: `PyObject_HEAD`.
    macro_field: IDENT,
    _struct_declarator: _declarator _attribute* [":" _expression@assign],

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
        &(declaration_specifiers user_type_name) specified_user_parameter
        | parameter_declaration
        | user_parameter
        | _unnamed_user_parameter
        | "...",
    parameter_declaration: declaration_specifiers [_declarator],
    /// `const Foo *self`: a qualifier and then a typedef name.
    specified_user_parameter: declaration_specifiers user_type_name [_declarator],
    /// `Foo *self`: a typedef name with no qualifier.
    user_parameter: user_type_name _declarator,
    /// `Foo*`: an unnamed parameter of typedef type.
    _unnamed_user_parameter: user_type_name _abstract_declarator,

    _abstract_declarator: pointer+ abstract_suffix*,
    abstract_suffix: array_suffix | parameter_list,
    type_name: declaration_specifiers [_abstract_declarator],

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
        postfix "++" | "--" => update_expression,
        postfix arguments => call_expression,
        postfix subscript => subscript_expression,
        postfix "." _member => field_expression,
        postfix "->" _member => field_expression,
    },

    _primary:
        &(cast_type) cast_expression
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
    cast_type: "(" type_name ")",
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
        let nodes = &workspace.tree.nodes;
        let Some(node) = nodes.iter().find(|node| node.kind == ERROR || node.flags & HAS_ERROR != 0) else {
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
        let nodes = &workspace.tree.nodes;
        for node in nodes.iter() {
            if node.kind != ERROR && node.flags & HAS_ERROR == 0 {
                continue;
            }
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
