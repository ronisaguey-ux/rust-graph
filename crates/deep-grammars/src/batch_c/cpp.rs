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

    // ---- expressions ----

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
        prefix "++" | "--" => update_expression,
        postfix "++" | "--" => update_expression,
        postfix arguments => call_expression,
        postfix &(template_arguments arguments) template_arguments arguments => call_expression,
        postfix subscript => subscript_expression,
        postfix "." _member => field_expression,
        postfix "->" _member => field_expression,
        postfix "::" _member => qualified_expression,
    },

    _primary:
        &(cast_type) cast_expression
        | named_cast
        | sizeof_expression
        | new_expression
        | delete_expression
        | lambda_expression
        | parenthesized_expression
        | string
        | "this"
        | "true"
        | "false"
        | "nullptr"
        | _name
        | NUMBER
        | CHAR_LITERAL,

    _name: IDENT,
    _member: IDENT,
    string: STR+,
    cast_expression: cast_type _expression@unary,
    cast_type: "(" (_type_prefixed | _scoped_type_id) ")",
    sizeof_expression: "sizeof" sizeof_operand,
    sizeof_operand: &(cast_type) cast_type | _expression@unary,
    named_cast: named_cast_head parenthesized_expression,
    named_cast_head: ("static_cast" | "dynamic_cast" | "reinterpret_cast" | "const_cast") "<" _type_id _gt,
    new_expression: "new" [new_placement] _type_id [arguments] [array_suffix],
    new_placement: "(" (_argument %? ",") ")",
    delete_expression: "delete" [empty_brackets] _expression@unary,
    empty_brackets: "[" "]",
    lambda_expression: lambda_capture [parameter_list] compound_statement,
    lambda_capture: "[" (_capture_item %? ",") "]",
    _capture_item: "=" IDENT | "&" IDENT | "=" | "&" | "this" | IDENT,
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
            number(c, true);
            NUMBER
        }
        b'.' if c.peek_at(1).is_ascii_digit() => {
            number(c, true);
            NUMBER
        }
        byte if is_word_start(byte) => word(c),
        _ => operator(c),
    }
}

/// A word; a literal prefix (`L"..."`, `u8R"(...)"`) starts a literal.
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    match c.peek() {
        b'"' if raw_prefix(text) => {
            c.bump();
            if raw_string(c) { STR } else { BAD_LITERAL }
        }
        b'"' | b'\'' if string_prefix(text) => {
            let quote = c.peek();
            c.bump();
            match (quoted(c, quote), quote) {
                (false, _) => BAD_LITERAL,
                (true, b'"') => STR,
                (true, _) => CHAR_LITERAL,
            }
        }
        _ => keyword(text),
    }
}

/// `R`, `LR`, `uR`, `UR`, `u8R`: a raw string follows.
fn raw_prefix(word: &[u8]) -> bool {
    matches!(word, b"R" | b"LR" | b"uR" | b"UR" | b"u8R")
}

/// `L`, `u`, `U`, `u8`, and the raw forms' non-`R` prefixes.
fn string_prefix(word: &[u8]) -> bool {
    matches!(word, b"L" | b"u" | b"U" | b"u8" | b"R" | b"LR" | b"uR" | b"UR" | b"u8R")
}

/// A raw string body after its opening quote: `delim( ... )delim"`. True if it
/// closed; an unclosed one runs to the end of the file.
fn raw_string(c: &mut Cursor) -> bool {
    let start = c.pos();
    let mut open = 0usize;
    while !c.at_end() && c.peek() != b'(' && open < 16 {
        if c.peek() == b')' || c.peek() == b'\\' || c.peek() == b'"' {
            return false;
        }
        c.bump();
        open += 1;
    }
    if c.peek() != b'(' {
        return false;
    }
    let delimiter: Vec<u8> = c.since(start).to_vec();
    c.bump();
    loop {
        if c.at_end() {
            return false;
        }
        if c.peek() == b')' && c.rest().get(1..).is_some_and(|rest| rest.starts_with(&delimiter)) {
            c.bump_n(delimiter.len() + 1);
            return c.eat(b'"');
        }
        c.bump();
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"auto" => KW_AUTO,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"catch" => KW_CATCH,
        b"char" => KW_CHAR,
        b"class" => KW_CLASS,
        b"const" => KW_CONST,
        b"constexpr" => KW_CONSTEXPR,
        b"continue" => KW_CONTINUE,
        b"decltype" => KW_DECLTYPE,
        b"default" => KW_DEFAULT,
        b"delete" => KW_DELETE,
        b"do" => KW_DO,
        b"double" => KW_DOUBLE,
        b"else" => KW_ELSE,
        b"enum" => KW_ENUM,
        b"explicit" => KW_EXPLICIT,
        b"extern" => KW_EXTERN,
        b"false" => KW_FALSE,
        b"final" => KW_FINAL,
        b"float" => KW_FLOAT,
        b"for" => KW_FOR,
        b"friend" => KW_FRIEND,
        b"goto" => KW_GOTO,
        b"if" => KW_IF,
        b"inline" => KW_INLINE,
        b"int" => KW_INT,
        b"long" => KW_LONG,
        b"mutable" => KW_MUTABLE,
        b"namespace" => KW_NAMESPACE,
        b"new" => KW_NEW,
        b"noexcept" => KW_NOEXCEPT,
        b"nullptr" => KW_NULLPTR,
        b"operator" => KW_OPERATOR,
        b"override" => KW_OVERRIDE,
        b"private" => KW_PRIVATE,
        b"protected" => KW_PROTECTED,
        b"public" => KW_PUBLIC,
        b"register" => KW_REGISTER,
        b"return" => KW_RETURN,
        b"short" => KW_SHORT,
        b"signed" => KW_SIGNED,
        b"sizeof" => KW_SIZEOF,
        b"static" => KW_STATIC,
        b"static_assert" => KW_STATIC_ASSERT,
        b"static_cast" => KW_STATIC_CAST,
        b"dynamic_cast" => KW_DYNAMIC_CAST,
        b"reinterpret_cast" => KW_REINTERPRET_CAST,
        b"const_cast" => KW_CONST_CAST,
        b"struct" => KW_STRUCT,
        b"switch" => KW_SWITCH,
        b"template" => KW_TEMPLATE,
        b"this" => KW_THIS,
        b"throw" => KW_THROW,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"typedef" => KW_TYPEDEF,
        b"typename" => KW_TYPENAME,
        b"union" => KW_UNION,
        b"unsigned" => KW_UNSIGNED,
        b"using" => KW_USING,
        b"virtual" => KW_VIRTUAL,
        b"void" => KW_VOID,
        b"volatile" => KW_VOLATILE,
        b"while" => KW_WHILE,
        b"bool" => KW_BOOL,
        _ => IDENT,
    }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b':', b':', _) => (SCOPE, 2),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'<', b'<', _) => (SHL, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'<', _, _) => (LT, 1),
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
/// DEEP_CPP_CORPUS=/tmp/opencode/ccorpus cargo test -p deep-grammars --lib \
///   cpp::corpus::clean_files -- --ignored --nocapture
/// ```
#[cfg(test)]
mod corpus {
    use std::fs;

    use crate::batch_c::cpp::CPP;
    use crate::deep::syntax::ERROR;
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    /// Every file in the directory, as `(name, did it parse with no error node, error count)`.
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
            let Ok(outcome) = workspace.parse(&src, &CPP, &Budget::default()) else { continue };
            if !outcome.complete() {
                continue;
            }
            let nodes = &workspace.tree.nodes;
            let errors = nodes.iter().filter(|node| node.kind == ERROR || node.flags & HAS_ERROR != 0 && node.flags & TOKEN != 0).count();
            out.push((path.file_name().unwrap().to_string_lossy().into_owned(), errors == 0, errors));
        }
        out
    }

    /// One file's error regions, with the source each one covers.
    #[test]
    #[ignore]
    fn one_file() {
        let Ok(path) = std::env::var("DEEP_CPP_ONE") else { return };
        let src = fs::read(&path).expect("file");
        let mut workspace = Workspace::default();
        workspace.parse(&src, &CPP, &Budget::default()).expect("parse");
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
        let Ok(dir) = std::env::var("DEEP_CPP_CORPUS") else { return };
        let results = scan(&dir, &["cpp", "hpp", "cc", "h"]);
        let clean = results.iter().filter(|(_, clean, _)| *clean).count();
        for (name, clean, errors) in &results {
            if !*clean {
                println!("FAIL {name} {errors}");
            }
        }
        println!("C++: {clean}/{} clean", results.len());
    }
}
