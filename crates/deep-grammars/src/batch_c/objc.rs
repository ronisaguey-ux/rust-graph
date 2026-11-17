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
        KW_COVARIANT = "__covariant",
        KW_CONTRAVARIANT = "__contravariant",

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
        AT_MACRO = "@macro",
        /// `IBOutlet` / `IBInspectable`: interface-builder words that stand
        /// between a property's attribute list and its type.
        KW_IBOUTLET = "IBOutlet",
        /// `(__bridge_transfer id)x`: the ownership transfer a cast names in
        /// front of the type it casts to.
        /// `(nullable NSDate *)date`: a nullability qualifier in front of the
        /// type in a parameter, which every modern Apple header writes.
        /// `__block RACCommand *command;`, `__weak id target;`: the ownership
        /// qualifiers a block or a capture list writes in front of a type.
        KW_BLOCK = "__block",
        KW_WEAK = "__weak",
        KW_STRONG = "__strong",
        KW_UNSAFE_UNRETAINED = "__unsafe_unretained",
        KW_AUTORELEASING = "__autoreleasing",
        KW_KINDOF = "__kindof",
        KW_UNUSED = "__unused",
        KW_NULLABLE = "nullable",
        KW_NONNULL = "nonnull",
        KW_NULL_UNSPECIFIED = "null_unspecified",
        KW_NULL_RESETTABLE = "null_resettable",
        KW_BRIDGE = "__bridge",
        KW_BRIDGE_TRANSFER = "__bridge_transfer",
        KW_BRIDGE_RETAINED = "__bridge_retained",
        KW_IBINSPECTABLE = "IBInspectable",
        AT_LBRACE = "@{",
        AT_LBRACKET = "@[",
        AT_NUMBER = "@number",

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
        &(IDENT "@interface") bare_macro_item
        | &(IDENT "@protocol") bare_macro_item
        | &(IDENT "@implementation") bare_macro_item
        | &(IDENT "@class") bare_macro_item
        | &("typedef" IDENT arguments) macro_enum_declaration
        | &(declaration_specifiers _declarator "{") function_definition
        | &(plain_type_name _declarator "{") user_function
        | &("static" plain_type_name _declarator "{") static_user_function
        | objc_interface
        | objc_implementation
        | &("@protocol" IDENT (";" | ",")) objc_forward_declaration
        | objc_protocol
        | objc_class_declaration
        | objc_import
        | &(IDENT arguments) macro_call_statement
        // `FOUNDATION_EXPORT double IQKeyboardManagerSwiftVersionNumber;`:
        // an export macro in front of a declaration is one statement to the
        // one-pass reader - the macro's name, then the type and the name it
        // declares - and with no rule the macro was the whole item and the
        // declaration under it was recovered.
        | &(IDENT declaration_specifiers _declarator) export_macro_declaration
        | declaration_statement
        // `NS_ASSUME_NONNULL_BEGIN`, and every other bare macro that stands as
        // an item of its own: without this alternative the item reached the
        // tree only through error recovery, and a file that ended on one - which
        // is every modern Apple header - kept a trailing ERROR node.
        | &(IDENT) bare_macro_item,

    /// `EXPMatcherInterface(endWith, (id expected));`, `NS_ENUM_BEGIN(…)`:
    /// a macro invocation with a semicolon that expands to something the
    /// grammar cannot see. Expecta and the UIKit headers are full of them and
    /// every one was an error node at file scope.
    /// `QuickSpecBegin(Spec)` and `EXPMatcherInterface(endWith, (id expected));`:
    /// a macro invocation with or without the semicolon. The Quick/Spec framework
    /// writes its spec headers as a bare call with no semicolon at file scope, so
    /// an optional `;` and one alternative covers both shapes - two alternatives
    /// would be indistinguishable at `IDENT "("`.
    macro_call_statement: IDENT arguments [";"],
    /// `FOUNDATION_EXPORT double VersionNumber;`: an export or visibility
    /// macro in front of a declaration. The macro is a name to the one-pass
    /// reader and the declaration follows it on the same line.
    export_macro_declaration: IDENT declaration_specifiers (_init_declarator % ",") ";",
    /// `NS_ASSUME_NONNULL_BEGIN` on a line of its own: a bare macro with no
    /// arguments and no semicolon, which every modern Apple header opens and
    /// closes with. Nothing else at file scope is a single name, so the guard
    /// is the top item that follows it.
    bare_macro_item: IDENT,

    function_definition: declaration_specifiers _declarator compound_statement,
    user_function: plain_type_name _declarator compound_statement,
    /// `static dispatch_queue_t f(void) { … }`: a storage class in front of a
    /// typedef'd return type. `declaration_specifiers` has no room for an
    /// unknown type name, so this shape - which AFNetworking and ReactiveObjC
    /// open a private helper with - was two error nodes.
    static_user_function: "static" plain_type_name _declarator compound_statement,

    declaration_statement: _user_type_declaration | &(declaration_specifiers plain_type_name _declarator) _qualified_user_declaration | _default_declaration,
    _qualified_user_declaration: declaration_specifiers plain_type_name (_init_declarator % ",") ";",
    _user_type_declaration: plain_type_name [_type_arguments] (_init_declarator % ",") ";",
    _default_declaration: declaration_specifiers (_init_declarator % ",") ";",
    _init_declarator: _declarator ["=" _initializer],
    _initializer: designated_field | &(array_suffix "=") designated_index | _expression@assign | initializer_list,
    initializer_list: "{" (_initializer %? ",") "}",
    designated_field: "." IDENT "=" _initializer,
    designated_index: array_suffix "=" _initializer,


    declaration_specifiers: _declaration_specifier+,
    _declaration_specifier:
        "typedef" | "extern" | "static" | "auto" | "register" | "inline" | "const" | "volatile"
        | KW_BLOCK | KW_WEAK | KW_STRONG | KW_UNSAFE_UNRETAINED | KW_AUTORELEASING | KW_KINDOF
        | KW_UNUSED
        | type_specifier,
    type_specifier:
        "void" | "char" | "short" | "int" | "long" | "float" | "double"
        | "signed" | "unsigned" | _qualified_id | "SEL" | "Class" | "BOOL"
        | struct_or_union_specifier
        | enum_specifier,
    _qualified_id: "id" [_type_arguments],
    plain_type_name: IDENT,

    struct_or_union_specifier: ("struct" | "union") [IDENT] [struct_body],
    struct_body: "{" _struct_declaration* "}",
    _struct_declaration: declaration_specifiers (_struct_declarator % ",") ";",
    _struct_declarator: _declarator [":" _expression@assign],

    /// `typedef NS_ENUM(NSUInteger, YYTextGrabberDirection) { kStart = 1, … };`
    /// and `typedef NS_OPTIONS(NSUInteger, X) { … };`: the Foundation macro that
    /// declares an enum. The macro's arguments sit between `typedef` and the
    /// body, so the ordinary declaration path saw `typedef NS_ENUM` as a type
    /// and then a brace it could not place — one error node per enum, and every
    /// Apple-flavoured header has one.
    macro_enum_declaration: "typedef" IDENT arguments enum_body ";",

    enum_specifier: "enum" [IDENT] [enum_body],
    enum_body: "{" (_enumerator %? ",") "}",
    _enumerator: IDENT ["=" _expression@assign],

    _declarator: pointer* direct_declarator,
    pointer: "*" _pointer_qualifier*,
    _pointer_qualifier: "const" | "volatile",
    direct_declarator: _direct_base _declarator_suffix*,
    _direct_base: block_direct_declarator | IDENT | parenthesized_declarator,
    /// `@property (copy) id (^handler)(id);`: the property holds a block, so
    /// its name sits inside a caret's parentheses and its parameter list
    /// follows as an ordinary declarator suffix.
    block_direct_declarator: "(" "^" [IDENT] ")",
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
    _user_type_name: plain_type_name [_type_arguments] [_abstract_declarator],
    _type_arguments: "<" (_type_argument % ",") _gt,
    _type_argument: [_variance] type_name,
    _variance: "__covariant" | "__contravariant",

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
        | &(plain_type_name _type_arguments) declaration_statement
        // `Foo f;`, `Foo *p = 0;`, `Foo a, b;`: a declaration whose type is a
        // class name. The guard names the line end, because `expect(command)
        // .notTo(beNil());` is also a name and a parenthesised declarator - and
        // reading it as a declaration left the call chain as error nodes, which
        // in turn took down the block that held it.
        | &(plain_type_name _declarator ";") declaration_statement
        | &(plain_type_name _declarator "=") declaration_statement
        | &(plain_type_name _declarator ",") declaration_statement
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

    // A category is the same line with a parenthesised name where a
    // superclass would be: `@interface Foo ()` is the anonymous extension a
    // class continues itself with, `@interface Foo (Bar)` a named one.
    objc_interface: "@interface" IDENT [_type_arguments] [":" IDENT] [category] [protocol_list] [ivar_block] _interface_item* "@end",
    category: "(" [IDENT] ")",
    protocol_list: "<" (protocol_name % ",") _gt,
    protocol_name: IDENT,
    _gt: ">" | GT_JOINED,
    ivar_block: "{" ivar_declaration* "}",
    ivar_declaration: (declaration_specifiers | plain_type_name) (_declarator % ",") ";",
    _interface_item: objc_method_declaration | objc_property_declaration | declaration_statement | ";",

    objc_implementation: "@implementation" IDENT [":" IDENT] [category] [ivar_block] _implementation_item* "@end",
    _implementation_item: objc_method_definition | &(declaration_specifiers _declarator "{") function_definition | objc_synthesize | objc_dynamic | declaration_statement | ";",
    objc_method_definition: ("-" | "+") [method_type] method_selector compound_statement,
    /// `- (instancetype)initWithFrame:(CGRect)frame NS_REQUIRES_SUPER;`: a
    /// declaration may carry attribute macros after its parameter list, and a
    /// required `;` straight after the selector made every one of those an
    /// ERROR - which took the whole interface with it.
    objc_method_declaration: ("-" | "+") [method_type] method_selector macro_attribute* ";",
    macro_attribute: IDENT,
    /// `- (void)run:(void (^)(void))done;` and `+ (instancetype)f:(id (^)(id))b;`:
    /// a block-typed parameter. The type is a type, a caret in parentheses and
    /// the block's own parameter list; the grammar had no such type, so every
    /// method that takes a completion block was an error node.
    /// `(nullable NSDate *)date`, `(nonnull NSString *)name`: a nullability
    /// qualifier in front of a parameter's type is written on every modern
    /// header, and the type rule had no room for the word.
    method_type: "(" [nullability] (&(block_type_name block_caret_group) block_pointer_type | type_name) ")",
    nullability: KW_NULLABLE | KW_NONNULL | KW_NULL_UNSPECIFIED | KW_NULL_RESETTABLE,
    block_pointer_type: block_type_name block_pointer_parameters,
    /// `(RACDisposable * (^)(id s))`: a block's return type may be a class
    /// name and a pointer, and the rule only took the name - so every block
    /// parameter that returns an object was an error node.
    block_type_name: (plain_type_name | "void" | "id") pointer* | pointer+,
    block_pointer_parameters: block_caret_group parameter_list,
    block_caret_group: "(" "^" [declaration_specifiers] ")",
    method_selector: IDENT | _selector_keyword+,
    _selector_keyword: IDENT ":" [method_type] [IDENT],

    objc_protocol: "@protocol" IDENT [protocol_list] _protocol_item* "@end",
    _protocol_item: objc_method_declaration | objc_property_declaration | optional_marker | required_marker | declaration_statement | ";",
    optional_marker: "@optional",
    required_marker: "@required",
    objc_forward_declaration: "@protocol" IDENT ("," IDENT)* ";",
    objc_class_declaration: "@class" (_class_name % ",") ";",
    _class_name: IDENT [_type_arguments],

    /// `@property (strong, nonatomic) IBOutlet UIImageView *imageView;`: an
    /// interface-builder modifier is a bare macro between the attribute list and
    /// the type, and the rule had no room for one - every IBOutlet property in
    /// the corpus was four ERRORs and took its class body with it.
    objc_property_declaration: "@property" [property_attributes] property_qualifier* _property_type (_declarator % ",") ";",
    property_qualifier: KW_IBOUTLET | KW_IBINSPECTABLE,
    _property_type: declaration_specifiers | plain_type_name [_type_arguments],
    property_attributes: "(" (_property_attribute % ",") ")",
    /// `@property (nonatomic, weak, readonly, nullable) id target;`: the
    /// nullability words are attributes here, and making them tokens for the
    /// parameter rule turned every such property into an error node.
    _property_attribute: _property_attribute_name ["=" IDENT],
    _property_attribute_name:
        IDENT | KW_NULLABLE | KW_NONNULL | KW_NULL_UNSPECIFIED | KW_NULL_RESETTABLE,

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
        | at_macro_expression
        | dictionary_literal
        | array_literal
        | encode_expression
        | parenthesized_expression
        | block_literal
        | message
        | string
        | "self"
        | "super"
        | "nil"
        | "YES"
        | "NO"
        | _name
        | NUMBER
        | AT_NUMBER
        | CHAR_LITERAL,

    /// `^{ … }`, `^void (void) { … }`, `^(int x) { … }`: a block literal is a
    /// caret, an optional return type, an optional parameter list and a body.
    /// It was not modelled at all, so every `dispatch_sync(q, ^{…})` — the
    /// shape most Objective-C files are written in — was one error node and the
    /// whole file fell back.
    block_literal: "^" [block_return_type] [block_parameters] compound_statement,
    block_return_type: &(declaration_specifiers) declaration_specifiers | plain_type_name,
    block_parameters: "(" [_block_parameter %? ","] ")",
    _block_parameter: &(declaration_specifiers) declaration_specifiers [_declarator] | plain_type_name [_declarator],

    _name: IDENT,
    _member: IDENT,
    string: STR+,
    cast_expression: cast_type _expression@unary,
    /// `(__bridge CFStringRef)domain`, `(__bridge_transfer id)obj`: a bridged
    /// cast names the ownership transfer and then the type, and the rule had no
    /// room for the word - every bridged cast in AFNetworking and YYKit was two
    /// ERRORs inside its statement.
    cast_type: "(" [bridge_qualifier] (_type_prefixed | _user_type_name) ")",
    bridge_qualifier: KW_BRIDGE | KW_BRIDGE_TRANSFER | KW_BRIDGE_RETAINED,
    sizeof_expression: "sizeof" sizeof_operand,
    sizeof_operand: &(cast_type) cast_type | _expression@unary,
    selector_expression: "@selector" selector_arguments,
    /// `@weakify(self)`, `@keypath(self.enabled)`: a macro that opens with an
    /// `@word` and takes arguments. The one-pass reader files it as a call of
    /// the word without the `@`, so the tree has to see a call here.
    at_macro_expression: AT_MACRO [arguments],
    /// `@{ @"key": @1 }` and `@[ @1, @2 ]`: the literal forms every modern
    /// Objective-C file writes instead of dictionaryWithObjectsAndKeys.
    dictionary_literal: AT_LBRACE (dictionary_entry %? ",") "}",
    dictionary_entry: _expression ":" _expression,
    array_literal: AT_LBRACKET (_expression %? ",") "]",
    selector_arguments: "(" method_selector ")",
    encode_expression: "@encode" encode_arguments,
    encode_arguments: "(" type_name ")",
    arguments: "(" (_argument %? ",") ")",
    _argument: _expression@assign,
    message: "[" _expression [_message_selector] "]",
    _message_selector: _message_name | _message_keyword+,
    _message_keyword: _message_name ":" _expression ("," _expression)*,
    /// `[RACSignal return:value]`, `[formatter default]`: a C keyword is a
    /// perfectly good Objective-C selector, and a lexer that keeps `return` a
    /// keyword leaves an error node inside every message that spells one - which
    /// in turn took down the block or the call that held it.
    _message_name:
        IDENT | KW_RETURN | KW_DEFAULT | KW_IN | KW_FOR | KW_IF | KW_ELSE | KW_CASE
        | KW_WHILE | KW_DO | KW_BREAK | KW_CONTINUE | KW_GOTO | KW_ENUM | KW_STRUCT
        | KW_UNION | KW_SIGNED | KW_UNSIGNED | KW_STATIC | KW_AUTO | KW_EXTERN
        | KW_CONST | KW_VOLATILE | KW_INLINE | KW_SWITCH,
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
    // `@{ @"k": @1 }` and `@[ @1, @2 ]`: the collection literals take their
    // bracket with the `@`, so the rule that opens the literal sees one token.
    if matches!(c.peek_at(1), b'{' | b'[') {
        let kind = if c.peek_at(1) == b'{' { AT_LBRACE } else { AT_LBRACKET };
        c.bump();
        c.bump();
        return kind;
    }
    // `@1`, `@3.5`: a boxed number, which is how every modern file writes an
    // NSNumber in a collection literal.
    if c.peek_at(1).is_ascii_digit() {
        c.bump();
        number(c, false);
        return AT_NUMBER;
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
        // `@weakify(self)`, `@keypath(self.enabled)`, `@onExit { … }`: an
        // `@word` the grammar has no keyword for is a libextobjc macro. Reading
        // it as unknown bytes made every file that uses one an error node.
        _ => AT_MACRO,
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
        b"IBOutlet" => KW_IBOUTLET,
        b"__block" => KW_BLOCK,
        b"__weak" => KW_WEAK,
        b"__strong" => KW_STRONG,
        b"__unsafe_unretained" => KW_UNSAFE_UNRETAINED,
        b"__autoreleasing" => KW_AUTORELEASING,
        b"__kindof" => KW_KINDOF,
        b"__unused" => KW_UNUSED,
        b"nullable" => KW_NULLABLE,
        b"nonnull" => KW_NONNULL,
        b"null_unspecified" => KW_NULL_UNSPECIFIED,
        b"null_resettable" => KW_NULL_RESETTABLE,
        b"__bridge" => KW_BRIDGE,
        b"__bridge_transfer" => KW_BRIDGE_TRANSFER,
        b"__bridge_retained" => KW_BRIDGE_RETAINED,
        b"IBInspectable" => KW_IBINSPECTABLE,
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
        b"__covariant" => KW_COVARIANT,
        b"__contravariant" => KW_CONTRAVARIANT,
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
