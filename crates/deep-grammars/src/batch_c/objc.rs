//! Objective-C through `grammar!`: C's skeleton plus `@interface`,
//! `@implementation`, `@protocol`, protocols, properties and message sends.
//!
//! `#import` is an ordinary preprocessor directive, so it is trivia. A message
//! is one postfix row, `[receiver selector]`, which is also how `a[0]` reads:
//! Objective-C has no separate subscript syntax, and the tree names it
//! `message_expression` either way.
//!
//! Not modelled: blocks (`^{...}`), `@available`, generics on classes
//! (`NSArray<NSString *> *`), NSString format-checking attributes, and
//! property `getter=`/`setter=` selectors are read as plain identifiers.

use super::{directive, is_word_byte, is_word_start, literal_prefix, number, quoted, trivia};
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, UNKNOWN};

rust_graph_deep::grammar! { OBJC {
    name = "objc";
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
        KW_IN = "in",
        KW_INLINE = "inline",
        KW_INT = "int",
        KW_LONG = "long",
        KW_REGISTER = "register",
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
        KW_BOOL = "BOOL",
        KW_ID = "id",
        KW_SEL = "SEL",
        KW_CLASS = "Class",
        KW_SELF = "self",
        KW_SUPER = "super",
        KW_NIL = "nil",
        KW_YES = "YES",
        KW_NO = "NO",

        AT_INTERFACE = "@interface",
        AT_IMPLEMENTATION = "@implementation",
        AT_PROTOCOL = "@protocol",
        AT_END = "@end",
        AT_CLASS = "@class",
        AT_PROPERTY = "@property",
        AT_SYNTHESIZE = "@synthesize",
        AT_DYNAMIC = "@dynamic",
        AT_SELECTOR = "@selector",
        AT_ENCODE = "@encode",
        AT_OPTIONAL = "@optional",
        AT_REQUIRED = "@required",
        AT_TRY = "@try",
        AT_CATCH = "@catch",
        AT_FINALLY = "@finally",
        AT_THROW = "@throw",
        AT_AUTORELEASEPOOL = "@autoreleasepool",
        AT_SYNCHRONIZED = "@synchronized",
        AT_IMPORT = "@import",
        AT_DEFS = "@defs",

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
        | &(plain_type_name _declarator "{") user_function
        | objc_interface
        | objc_implementation
        | &("@protocol" IDENT (";" | ",")) objc_forward_declaration
        | objc_protocol
        | objc_class_declaration
        | objc_import
        | declaration_statement,

    function_definition: declaration_specifiers _declarator compound_statement,
    user_function: plain_type_name _declarator compound_statement,

    declaration_statement: _user_type_declaration | &(declaration_specifiers plain_type_name _declarator) _qualified_user_declaration | _default_declaration,
    _qualified_user_declaration: declaration_specifiers plain_type_name (_init_declarator % ",") ";",
    _user_type_declaration: plain_type_name (_init_declarator % ",") ";",
    _default_declaration: declaration_specifiers (_init_declarator % ",") ";",
    _init_declarator: _declarator ["=" _initializer],
    _initializer: designated_field | &(array_suffix "=") designated_index | _expression@assign | initializer_list,
    initializer_list: "{" (_initializer %? ",") "}",
    designated_field: "." IDENT "=" _initializer,
    designated_index: array_suffix "=" _initializer,


    declaration_specifiers: _declaration_specifier+,
    _declaration_specifier:
        "typedef" | "extern" | "static" | "auto" | "register" | "inline" | "const" | "volatile"
        | type_specifier,
    type_specifier:
        "void" | "char" | "short" | "int" | "long" | "float" | "double"
        | "signed" | "unsigned" | "id" | "SEL" | "Class" | "BOOL"
        | struct_or_union_specifier
        | enum_specifier,
    plain_type_name: IDENT,

    struct_or_union_specifier: ("struct" | "union") [IDENT] [struct_body],
    struct_body: "{" _struct_declaration* "}",
    _struct_declaration: declaration_specifiers (_struct_declarator % ",") ";",
    _struct_declarator: _declarator [":" _expression@assign],

    enum_specifier: "enum" [IDENT] [enum_body],
    enum_body: "{" (_enumerator %? ",") "}",
    _enumerator: IDENT ["=" _expression@assign],

    _declarator: pointer* direct_declarator,
    pointer: "*" _pointer_qualifier*,
    _pointer_qualifier: "const" | "volatile",
    direct_declarator: _direct_base _declarator_suffix*,
    _direct_base: IDENT | parenthesized_declarator,
    parenthesized_declarator: "(" _declarator ")",
    _declarator_suffix: array_suffix | parameter_list,
    array_suffix: "[" [_expression] "]",
    parameter_list: "(" (_parameter % ",") ")",
    _parameter: _user_type_parameter | _default_parameter | "...",
    _user_type_parameter: plain_type_name _declarator,
    _default_parameter: declaration_specifiers [_declarator],

    _abstract_declarator: pointer+ abstract_suffix*,
    abstract_suffix: array_suffix | parameter_list,
    type_name: _type_prefixed | _user_type_name,
    _type_prefixed: declaration_specifiers [_abstract_declarator],
    _user_type_name: plain_type_name [_abstract_declarator],

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
        | synchronized_statement
        | autoreleasepool_statement
        | objc_synthesize
        | objc_dynamic
        | &(declaration_specifiers) declaration_statement
        | &(plain_type_name _declarator) declaration_statement
        | expression_statement,

    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    switch_statement: "switch" parenthesized_expression switch_body,
    switch_body: "{" _switch_item* "}",
    _switch_item: case_statement | default_statement | _statement,
    case_statement: "case" _expression@assign ":" _statement*,
    default_statement: "default" ":" _statement*,
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: &("for" for_in_header) for_in_statement | classic_for_statement,
    classic_for_statement: "for" for_header _statement,
    for_header: "(" [_for_init] ";" [_expression] ";" [_expression] ")",
    _for_init: &(declaration_specifiers) _for_declaration | _expression,
    _for_declaration: declaration_specifiers (_init_declarator % ","),
    for_in_statement: "for" for_in_header _statement,
    for_in_header: "(" (declaration_specifiers [_declarator] | plain_type_name _declarator) "in" _expression ")",
    parenthesized_expression: "(" _expression ")",
    return_statement: "return" [_expression] ";",
    break_statement: "break" ";",
    continue_statement: "continue" ";",
    goto_statement: "goto" (IDENT | "*" _expression) ";",
    labeled_statement: IDENT ":" _statement,
    try_statement: "@try" compound_statement catch_clause* [finally_clause],
    catch_clause: "@catch" catch_parameter compound_statement,
    catch_parameter: "(" [_parameter] ")",
    finally_clause: "@finally" compound_statement,
    throw_statement: "@throw" [_expression] ";",
    synchronized_statement: "@synchronized" synchronized_arguments compound_statement,
    synchronized_arguments: "(" _expression ")",
    autoreleasepool_statement: "@autoreleasepool" compound_statement,
    expression_statement: _expression ";",

    // ---- @-declarations ----

    objc_interface: "@interface" IDENT [":" IDENT] [protocol_list] [ivar_block] _interface_item* "@end",
    protocol_list: "<" (protocol_name % ",") _gt,
    protocol_name: IDENT,
    _gt: ">" | GT_JOINED,
    ivar_block: "{" ivar_declaration* "}",
    ivar_declaration: (declaration_specifiers | plain_type_name) (_declarator % ",") ";",
    _interface_item: objc_method_declaration | objc_property_declaration | declaration_statement | ";",

    objc_implementation: "@implementation" IDENT [":" IDENT] _implementation_item* "@end",
    _implementation_item: objc_method_definition | &(declaration_specifiers _declarator "{") function_definition | objc_synthesize | objc_dynamic | declaration_statement | ";",
    objc_method_definition: ("-" | "+") [method_type] method_selector compound_statement,
    objc_method_declaration: ("-" | "+") [method_type] method_selector ";",
    method_type: "(" type_name ")",
    method_selector: IDENT | _selector_keyword+,
    _selector_keyword: IDENT ":" [method_type] [IDENT],

    objc_protocol: "@protocol" IDENT [protocol_list] _protocol_item* "@end",
    _protocol_item: objc_method_declaration | objc_property_declaration | optional_marker | required_marker | declaration_statement | ";",
    optional_marker: "@optional",
    required_marker: "@required",
    objc_forward_declaration: "@protocol" IDENT ("," IDENT)* ";",
    objc_class_declaration: "@class" (IDENT % ",") ";",

    objc_property_declaration: "@property" [property_attributes] _property_type (_declarator % ",") ";",
    _property_type: declaration_specifiers | plain_type_name,
    property_attributes: "(" (_property_attribute % ",") ")",
    _property_attribute: IDENT ["=" IDENT],

    objc_synthesize: "@synthesize" (_synthesized_name % ",") ";",
    _synthesized_name: IDENT ["=" IDENT],
    objc_dynamic: "@dynamic" (IDENT % ",") ";",
    objc_import: "@import" (IDENT % ",") ";",

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
        postfix "++" | "--" => update_expression,
        postfix arguments => call_expression,
        postfix message => message_expression,
        postfix "." _member => field_expression,
        postfix "->" _member => field_expression,
    },

    _primary:
        &(cast_type) cast_expression
        | sizeof_expression
        | selector_expression
        | encode_expression
        | parenthesized_expression
        | message
        | string
        | "self"
        | "super"
        | "nil"
        | "YES"
        | "NO"
        | _name
        | NUMBER
        | CHAR_LITERAL,

    _name: IDENT,
    _member: IDENT,
    string: STR+,
    cast_expression: cast_type _expression@unary,
    cast_type: "(" (_type_prefixed | _user_type_name) ")",
    sizeof_expression: "sizeof" sizeof_operand,
    sizeof_operand: &(cast_type) cast_type | _expression@unary,
    selector_expression: "@selector" selector_arguments,
    selector_arguments: "(" method_selector ")",
    encode_expression: "@encode" encode_arguments,
    encode_arguments: "(" type_name ")",
    arguments: "(" (_argument %? ",") ")",
    _argument: _expression@assign,
    message: "[" _expression [_message_selector] "]",
    _message_selector: IDENT | _message_keyword+,
    _message_keyword: IDENT ":" _expression ("," _expression)*,
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
        b'@' => at_word(c),
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

/// `@keyword`, or an Objective-C string literal `@"..."`.
fn at_word(c: &mut Cursor) -> Kind {
    if c.peek_at(1) == b'"' {
        c.bump();
        c.bump();
        return if quoted(c, b'"') { STR } else { BAD_LITERAL };
    }
    let start = c.rest();
    c.bump();
    let length = c.eat_while(is_word_byte);
    match &start[1..length + 1] {
        b"interface" => AT_INTERFACE,
        b"implementation" => AT_IMPLEMENTATION,
        b"protocol" => AT_PROTOCOL,
        b"end" => AT_END,
        b"class" => AT_CLASS,
        b"property" => AT_PROPERTY,
        b"synthesize" => AT_SYNTHESIZE,
        b"dynamic" => AT_DYNAMIC,
        b"selector" => AT_SELECTOR,
        b"encode" => AT_ENCODE,
        b"optional" => AT_OPTIONAL,
        b"required" => AT_REQUIRED,
        b"try" => AT_TRY,
        b"catch" => AT_CATCH,
        b"finally" => AT_FINALLY,
        b"throw" => AT_THROW,
        b"autoreleasepool" => AT_AUTORELEASEPOOL,
        b"synchronized" => AT_SYNCHRONIZED,
        b"import" => AT_IMPORT,
        b"defs" => AT_DEFS,
        _ => UNKNOWN,
    }
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    if matches!(c.peek(), b'"' | b'\'') {
        if literal_prefix(text).is_some() {
            let quote = c.peek();
            c.bump();
            return match (quoted(c, quote), quote) {
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
        b"in" => KW_IN,
        b"inline" => KW_INLINE,
        b"int" => KW_INT,
        b"long" => KW_LONG,
        b"register" => KW_REGISTER,
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
        b"BOOL" => KW_BOOL,
        b"id" => KW_ID,
        b"SEL" => KW_SEL,
        b"Class" => KW_CLASS,
        b"self" => KW_SELF,
        b"super" => KW_SUPER,
        b"nil" => KW_NIL,
        b"YES" => KW_YES,
        b"NO" => KW_NO,
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
