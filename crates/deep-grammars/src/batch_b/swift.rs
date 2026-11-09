//! Swift through `grammar!`: one of Field 6's four languages (Ruby, PHP, Swift,
//! Scala).
//!
//! Swift is brace-delimited and has no statement terminator, so no layout rule
//! is needed: a line break is whitespace and a block ends at its `}`. Node
//! names follow Tree-sitter's Swift where that costs nothing; the tree is not
//! the same tree.
//!
//! What the DSL covers here:
//!
//! - **Declarations carry modifiers and attributes.** `@MainActor public final
//!   class`, `static func` and `private var` share a prefix, so each
//!   declaration alternative is entered through `&([declaration_modifiers]
//!   "keyword")`, the same answer Java's grammar uses.
//! - **`let`/`var` are declarations anywhere.** A local binding and a stored
//!   property are the same rule, because Swift makes them the same construct;
//!   the node is `property_declaration` in both places.
//! - **Closures are the weakest postfix row.** `foo { ... }` is a trailing
//!   closure, and the row sits below assignment so that a statement's
//!   expression takes it while an `if`/`while` condition, entered at
//!   `_expression@assign`, does not: that is what keeps `if x { ... }` a
//!   statement and its body rather than a call.
//! - **Optional chaining is one token.** `a?.b` lexes `?.`, so the ternary `?`
//!   row never sees it.
//! - **`>` before `>` is `GT_JOINED`**, as in the Rust, JavaScript and Java
//!   grammars, so `Foo<Bar<Baz>>` closes two type lists and `a >> b` is
//!   `GT_JOINED ">"`.
//! - **Strings are single tokens.** A string's `\(...)` interpolation is
//!   scanned with its parentheses and inner strings balanced and stays opaque;
//!   a multi-line string and a raw string are one token each.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **A string's interpolation is a mode switch.** `\(` returns to code and
//!   the matching `)` returns to the string; the DSL's one-function-per-token
//!   lexer has no mode stack, so the step lexer scans the whole literal and the
//!   fields stay unparsed. This is why `interpolation` is not a node.
//! - **Custom operators cannot be enumerated.** Swift lets a program declare
//!   any operator; a grammar cannot, so this one takes the fixed set the
//!   standard library uses (`+ - * / % == != < > <= >= && || ?? ... ..< =~`)
//!   and every augmentation of `=`. A file that declares its own operator
//!   lexes it as `UNKNOWN`.
//! - **Automatic semicolon insertion does not exist here.** A line break is
//!   whitespace, so `x = 1` followed by `y = 2` is two statements only because
//!   a second expression cannot continue the first without an operator. A file
//!   relying on a following line that begins with an operator (Swift allows
//!   neither) would parse differently; real Swift does not.
//! - **The type grammar is shallow.** `some`/`any`/`inout`, optionals,
//!   generics, tuples, dictionaries, arrays and function arrows are types;
//!   `where` constraints are parsed as types too. Opaque `some P & Q` and
//!   protocol compositions are not modelled.
//! - **A binding's initialiser and a condition's are different rules.** A
//!   `let` in a condition stops before the block (`if let x = foo {`), so its
//!   initialiser enters the expression table above the trailing-closure row;
//!   a stored property's initialiser takes the whole table, which is what lets
//!   `let doubled = numbers.map { ... }` keep its closure.
//! - **A decision sees three tokens.** Declarations sharing a modifier prefix
//!   and a closure's `(x: Int) in` head both need more, so they use `&(...)`
//!   predicates.
//! - **One kind per rule**, so `class`, `struct`, `enum`, `protocol`,
//!   `extension` and `actor` are six rules.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { SWIFT {
    name = "swift";
    lex = lex;

    /// A string's `\(...)` field is code again. `STRING_BODY` is the literal's
    /// text frame; `INTERPOLATION` is pushed by the field's `\(` and popped by
    /// the `)` that matches it at the field's own bracket depth, so a call
    /// written in a field is an ordinary `call_expression` in the tree.
    modes {
        STRING_BODY = lex_string_text,
        INTERPOLATION = lex_interpolation,
    }
    switch STRING_START => push STRING_BODY;
    switch STRING_END => pop;
    switch STRING_BAD => pop;
    switch INTERP_OPEN => push INTERPOLATION;
    switch INTERP_CLOSE => pop;

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
        /// A string or raw string with no end.
        BAD_LITERAL = "bad_literal" bad,
        /// The opening quote (and `#`s) of a literal that has a field.
        STRING_START = "string_start",
        /// The literal's text between fields.
        STRING_TEXT = "string_text",
        /// The closing quote (and `#`s) of a literal with a field.
        STRING_END = "string_end",
        /// A literal that cannot close: its frame pops and the parser reports it.
        STRING_BAD = "bad_string" bad,
        /// `\(` (or `\#(` in a raw string): code again until the matching `)`.
        INTERP_OPEN = "interpolation_open" open interpolation,
        /// The `)` that matches an interpolation's `(`.
        INTERP_CLOSE = "interpolation_close" close interpolation,

        KW_ACTOR = "actor",
        KW_ANY = "any",
        KW_AS = "as",
        KW_ASYNC = "async",
        KW_ASSOCIATEDTYPE = "associatedtype",
        KW_AWAIT = "await",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CLASS = "class",
        KW_CONTINUE = "continue",
        KW_DEFAULT = "default",
        KW_DEFER = "defer",
        KW_DEINIT = "deinit",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXTENSION = "extension",
        KW_FALSE = "false",
        KW_FOR = "for",
        KW_FUNC = "func",
        KW_GUARD = "guard",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_IN = "in",
        KW_INIT = "init",
        KW_INOUT = "inout",
        KW_IS = "is",
        KW_LET = "let",
        KW_NIL = "nil",
        KW_PROTOCOL = "protocol",
        KW_REPEAT = "repeat",
        KW_RETHROWS = "rethrows",
        KW_RETURN = "return",
        KW_SELF = "self",
        KW_SET = "set",
        KW_SOME = "some",
        KW_STRUCT = "struct",
        KW_SUBSCRIPT = "subscript",
        KW_SUPER = "super",
        KW_SWITCH = "switch",
        KW_THROW = "throw",
        KW_THROWS = "throws",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPEALIAS = "typealias",
        KW_VAR = "var",
        KW_WHERE = "where",
        KW_WHILE = "while",

        /// Declaration modifiers: names everywhere but where a declaration starts.
        KW_PUBLIC = "public",
        KW_PRIVATE = "private",
        KW_FILEPRIVATE = "fileprivate",
        KW_INTERNAL = "internal",
        KW_OPEN = "open",
        KW_FINAL = "final",
        KW_STATIC = "static",
        KW_OVERRIDE = "override",
        KW_MUTATING = "mutating",
        KW_NONMUTATING = "nonmutating",
        KW_LAZY = "lazy",
        KW_WEAK = "weak",
        KW_UNOWNED = "unowned",
        KW_REQUIRED = "required",
        KW_CONVENIENCE = "convenience",
        KW_DYNAMIC = "dynamic",
        KW_INDIRECT = "indirect",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        BACKSLASH = "\\",
        DOTDOT = "..",
        ELLIPSIS = "...",
        RANGE_OPEN = "..<",
        OPTIONAL_CHAIN = "?.",
        COLON = ":",
        QUESTION = "?",
        AT = "@",
        ARROW = "->",
        ASSIGN = "=",
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
        STRICT_EQ = "===",
        NE = "!=",
        STRICT_NE = "!==",
        LE = "<=",
        GE = ">=",
        ANDAND = "&&",
        OROR = "||",
        NULLISH = "??",
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
        /// A `>` right before another `>`: the first half of `>>`, `>>=`, or a
        /// generic argument list closing right before another.
        GT_JOINED = ">joined",
        /// A `<` written right against the name before it: `Foo<Bar>`, not
        /// `a < b`. The grammar reads it only where a type list can open.
        GENERIC_LT = "<generic",
    }

    program: _statement*,

    _statement:
        &([declaration_modifiers] "func") function_declaration
        | &([declaration_modifiers] "class") class_declaration
        | &([declaration_modifiers] "struct") struct_declaration
        | &([declaration_modifiers] "enum") enum_declaration
        | &([declaration_modifiers] "protocol") protocol_declaration
        | &([declaration_modifiers] "extension") extension_declaration
        | &([declaration_modifiers] "actor") actor_declaration
        | &([declaration_modifiers] "init") initializer_declaration
        | &([declaration_modifiers] "subscript") subscript_declaration
        | &([declaration_modifiers] ("let" | "var")) property_declaration
        | &("deinit") deinitializer_declaration
        | &("import") import_declaration
        | &("typealias") typealias_declaration
        | &("associatedtype") associated_type_declaration
        | &(_name ":") labeled_statement
        | if_statement
        | guard_statement
        | while_statement
        | repeat_statement
        | for_statement
        | switch_statement
        | do_statement
        | defer_statement
        | return_statement
        | throw_statement
        | break_statement
        | continue_statement
        | ";"
        | expression_statement,

    // ---- declarations ----

    declaration_modifiers: (attribute | modifier)+,
    modifier:
        "public" | "private" | "fileprivate" | "internal" | "open" | "final" | "static"
        | "override" | "mutating" | "nonmutating" | "lazy" | "weak" | "unowned" | "required"
        | "convenience" | "dynamic" | "indirect"
        | "private" setter_arguments
        | "fileprivate" setter_arguments
        | "internal" setter_arguments,
    /// `private(set)` and friends: the setter is narrower than the getter.
    setter_arguments: "(" "set" ")",
    attribute: "@" _name [attribute_arguments],
    attribute_arguments: "(" (_expression %? ",") ")",

    import_declaration: "import" ["class" | "struct" | "enum" | "protocol" | "func" | "var" | "let" | "typealias"] _name ("." _name)*,
    typealias_declaration: "typealias" _name [generic_parameters] "=" _type,

    class_declaration: [declaration_modifiers] "class" _name [generic_parameters] [inheritance_clause] [where_clause] type_body,
    struct_declaration: [declaration_modifiers] "struct" _name [generic_parameters] [inheritance_clause] [where_clause] type_body,
    enum_declaration: [declaration_modifiers] "enum" _name [generic_parameters] [inheritance_clause] [where_clause] type_body,
    protocol_declaration: [declaration_modifiers] "protocol" _name [inheritance_clause] [where_clause] type_body,
    extension_declaration: [declaration_modifiers] "extension" _name [inheritance_clause] [where_clause] type_body,
    actor_declaration: [declaration_modifiers] "actor" _name [generic_parameters] [inheritance_clause] [where_clause] type_body,
    inheritance_clause: ":" (_type % ","),
    where_clause: "where" (_where_constraint % ","),
    _where_constraint: _type ("==" _type | ":" _type),
    type_body: "{" _type_member* "}",
    _type_member: enum_case_declaration | _statement,
    enum_case_declaration: "case" (_enum_case % ","),
    _enum_case: _name [enum_case_values],
    enum_case_values: "(" (_enum_case_value %? ",") ")",
    _enum_case_value: &(_name ":") labeled_enum_case_value | _type,
    labeled_enum_case_value: _name ":" _type,
    associated_type_declaration: "associatedtype" _name [":" _type] [where_clause],

    function_declaration: [declaration_modifiers] "func" _name [generic_parameters] parameter_clause ["async"] ["throws" | "rethrows"] ["->" _type] [where_clause] [(block | ";")],
    initializer_declaration: [declaration_modifiers] "init" ["?" | "!"] parameter_clause ["async"] ["throws"] [where_clause] (block | ";"),
    deinitializer_declaration: "deinit" block,
    subscript_declaration: [declaration_modifiers] "subscript" parameter_clause "->" _type (block | ";"),
    parameter_clause: "(" (parameter %? ",") ")",
    parameter: simple_parameter | labeled_parameter,
    simple_parameter: _name ":" _type ["..."] ["=" _expression@assign],
    /// `label name: Type`: the label comes first and shadows nothing.
    labeled_parameter: _argument_label simple_parameter,
    property_declaration: [declaration_modifiers] ("let" | "var") (_binding % ","),
    /// A binding names one value or destructures a tuple: `let (a, b) = pair`.
    _binding: _binding_target [":" _type] [block | "=" _expression@assign],
    _binding_target: _name | parenthesized_expression,

    generic_parameters: _generic_open (generic_parameter %? ",") _gt,
    generic_parameter: _name [":" _type],
    _generic_open: "<" | GENERIC_LT,
    _gt: ">" | GT_JOINED,

    // ---- statements ----

    block: "{" _statement* "}",
    expression_statement: _expression,

    if_statement: "if" _condition_list block [else_clause],
    else_clause: "else" (if_statement | block),
    guard_statement: "guard" _condition_list "else" block,
    while_statement: "while" _condition_list block,
    repeat_statement: "repeat" block "while" _condition_list,
    for_statement: "for" ["try"] ["await"] _for_binding "in" _expression@nilco ["where" _expression@nilco] block,
    _for_binding: [_for_owner] _binding_target,
    _for_owner: "let" | "var",
    labeled_statement: _name ":" _statement,
    _condition_list: _condition ("," _condition)*,
    _condition: binding_condition | case_condition | _expression@nilco,
    /// `if let x = ...`, `guard var y = ...`: a binding, where the keyword tells
    /// it from a plain condition.
    binding_condition: ("let" | "var") condition_binding,
    /// `if case let text as String = value`, `guard case .success(let x) = r`.
    case_condition: "case" _pattern "=" _expression@nilco,
    /// A condition's binding stops before the block: its initializer is entered
    /// above the trailing-closure row, `if foo {` and `if let x = foo {` alike.
    condition_binding: _binding_target [":" _type] ["=" _expression@nilco],

    switch_statement: "switch" _expression@nilco switch_body,
    switch_body: "{" switch_case* "}",
    switch_case: case_label ["where" _expression@nilco] ":" _statement*,
    case_label: "case" (_pattern % ",") | "default",
    /// A pattern is an expression or a `let`/`var` destructuring; only case
    /// labels and `case` conditions ask for one. A tuple pattern is a
    /// parenthesized expression, which may hold nested `let`s.
    _pattern: readonly_pattern | _expression@assign,
    readonly_pattern: ("let" | "var") ["case"] pattern_binding,
    pattern_binding: _name ["as" _type] | parenthesized_expression ["as" _type],

    do_statement: "do" block catch_clause*,
    /// `catch`, `catch let error`, `catch let error as E`, `catch E.case`.
    /// A bare `{` is the clause's body, so a pattern rule never starts with one.
    catch_clause: "catch" [_catch_binding] block,
    _catch_binding: typed_catch_binding | catch_path,
    typed_catch_binding: ("let" | "var") _name ["as" _type],
    catch_path: _name ("." _name)*,
    defer_statement: "defer" block,
    return_statement: "return" [_expression],
    throw_statement: "throw" _expression,
    break_statement: "break" [_name],
    continue_statement: "continue" [_name],

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" => assignment,
        cond: postfix "?" _expression ":" _expression@assign => ternary,
        closure: postfix closure => trailing_closure,
        nilco: left "??" => nil_coalescing,
        or: left "||" => binary,
        and: left "&&" => binary,
        eq: left "==" | "!=" | "===" | "!==" => binary,
        cmp: left "<" | ">" | "<=" | ">=" => binary,
        cast: left "is" | "as" ["?" | "!"] => cast,
        range: left "..." | "..<" => range,
        bor: left "|" => binary,
        xor: left "^" => binary,
        band: left "&" => binary,
        shift: left "<<" | GT_JOINED ">" => binary,
        add: left "+" | "-" => binary,
        mul: left "*" | "/" | "%" => binary,
        try: prefix "try" ["?" | "!"] => try_expression,
        await: prefix "await" => await_expression,
        unary: prefix "-" | "+" | "!" | "~" | "&" => prefix_expression,
        postfix "!" => force_unwrap,
        postfix "?." _property => optional_chaining_expression,
        postfix "." _property => navigation_expression,
        postfix arguments => call_expression,
        postfix subscript => subscript_expression,
    },

    _primary:
        NUMBER | FLOAT | STRING
        | "true" | "false" | "nil" | "self" | "super"
        | &(_name GENERIC_LT) generic_specialization
        | _name
        | string_literal
        | parenthesized_expression
        | collection_literal
        | closure
        | implicit_member_expression
        | key_path_expression,

    /// `Endpoint<Item>(...)`: a type list written in expression position.
    generic_specialization: _name (_generic_open (_type %? ",") _gt)+,
    /// A literal with a field: its text and its `\(...)` expressions are
    /// children, so a call written inside one is an ordinary `call_expression`.
    string_literal: STRING_START _string_body STRING_END,
    _string_body: (STRING_TEXT | interpolation)*,
    interpolation: INTERP_OPEN _expression INTERP_CLOSE,

    _name: IDENT,
    _property: IDENT | "self" | "init" | "default" | "class",
    parenthesized_expression: "(" (_paren_item %? ",") ")",
    _paren_item: readonly_item | _expression@assign,
    /// `let value` inside a tuple pattern: a binding with no initialiser.
    readonly_item: ("let" | "var") _name,
    arguments: "(" (_argument %? ",") ")",
    _argument: &(_argument_label ":") labeled_argument | _argument_value,
    /// Swift lets an external argument label be a keyword: `data(for: request)`.
    _argument_label: _name | "for" | "in" | "where" | "case" | "default" | "if" | "else"
        | "switch" | "func" | "class" | "struct" | "enum" | "protocol" | "extension"
        | "import" | "return" | "while" | "do" | "catch" | "defer" | "guard" | "break"
        | "continue" | "throw" | "try" | "as" | "is" | "nil" | "true" | "false" | "self"
        | "super" | "static" | "private" | "public" | "init" | "subscript" | "typealias",
    labeled_argument: _argument_label ":" _argument_value,
    _argument_value: ("let" | "var") _name | _expression@assign,
    subscript: "[" (_argument %? ",") "]",
    /// An array or dictionary literal: one node kind, because `[a: b]` and
    /// `[a, b]` are told apart by a `:` inside the brackets, which the DSL's
    /// group rule cannot look for before it opens the group.
    collection_literal: "[" [collection_body] "]",
    collection_body: ":" | _collection_items,
    _collection_items: _collection_item _collection_more*,
    /// A separator and the element after it, or a trailing separator alone.
    /// The bare comma is told from the pair by a predicate, because entering
    /// the pair and finding no element would record a missing one.
    _collection_more: &("," _collection_item) "," _collection_item | ",",
    /// An element, or a `key: value` pair. One rule, because the `:` is one
    /// token past what a three-token predicate can commit on.
    _collection_item: _expression [":" _expression],
    implicit_member_expression: "." _property,
    key_path_expression: "\\" ("." _property | _name ("." _property)*),
    closure: "{" _closure_item* "}",
    /// The head `(a, b) in` is told from the body by a predicate, so a body
    /// that starts with a name (`{ $0 }`) is a statement, not a signature.
    _closure_item: &(_closure_head "in") closure_signature | _statement,
    closure_signature: _closure_head "in",
    _closure_head: _closure_parameter_list | _closure_parameter_names,
    _closure_parameter_list: closure_parameter_group ["->" _type],
    closure_parameter_group: "(" (_name %? ",") ")",
    _closure_parameter_names: _closure_parameter_name ("," _closure_parameter_name)*,
    _closure_parameter_name: _name ["->" _type],

    // ---- types ----

    _type: [_type_attribute] _type_shape ["async"] ["throws"] ["->" _type],
    /// A type attribute carries no arguments: `@escaping (Data) -> Void`'s
    /// parentheses are the function type, not an argument list.
    _type_attribute: "@" _name,
    _type_shape: [_type_modifier] _type_atom _type_suffix*,
    _type_modifier: "some" | "any" | "inout",
    _type_atom: _name | collection_type | tuple_type,
    _type_suffix: "?" | "!" | "." _name | type_arguments,
    /// `[Int]` or `[String: Int]`: one rule, like the literal.
    collection_type: "[" _type [":" _type] "]",
    tuple_type: "(" (_type %? ",") ")",
    type_arguments: _generic_open (_type %? ",") _gt,
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    if c.pos() == 0 {
        // A new file starts outside every string, and whatever state a parse
        // that stopped inside one left behind is not this file's.
        STRINGS.with(|frames| frames.borrow_mut().clear());
        DEPTHS.with(|depths| depths.borrow_mut().clear());
    }
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
            WHITESPACE
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            block_comment(c);
            COMMENT
        }
        b'#' if c.peek_at(1) == b'"' || c.peek_at(1) == b'#' => string_literal(c),
        b'"' => string_literal(c),
        b'`' => {
            c.bump();
            c.eat_while(|byte| byte != b'`' && byte != b'\n');
            c.eat(b'`');
            IDENT
        }
        b'$' if is_word_start(c.peek_at(1)) || c.peek_at(1).is_ascii_digit() => {
            // A projected property wrapper (`$query`) and a closure argument
            // (`$0`) are ordinary names to the grammar.
            c.bump();
            c.eat_while(is_word_byte);
            IDENT
        }
        b'#' if is_word_start(c.peek_at(1)) => {
            c.bump();
            c.eat_while(is_word_byte);
            IDENT
        }
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() && !c.starts_with(b"..") => number(c),
        b'@' => {
            c.bump();
            AT
        }
        byte if is_word_start(byte) => word(c),
        _ => operator(c),
    }
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    keyword(&start[..length])
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"actor" => KW_ACTOR,
        b"any" => KW_ANY,
        b"as" => KW_AS,
        b"associatedtype" => KW_ASSOCIATEDTYPE,
        b"async" => KW_ASYNC,
        b"await" => KW_AWAIT,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"catch" => KW_CATCH,
        b"class" => KW_CLASS,
        b"continue" => KW_CONTINUE,
        b"default" => KW_DEFAULT,
        b"defer" => KW_DEFER,
        b"deinit" => KW_DEINIT,
        b"do" => KW_DO,
        b"else" => KW_ELSE,
        b"enum" => KW_ENUM,
        b"extension" => KW_EXTENSION,
        b"false" => KW_FALSE,
        b"for" => KW_FOR,
        b"func" => KW_FUNC,
        b"guard" => KW_GUARD,
        b"if" => KW_IF,
        b"import" => KW_IMPORT,
        b"in" => KW_IN,
        b"init" => KW_INIT,
        b"inout" => KW_INOUT,
        b"is" => KW_IS,
        b"let" => KW_LET,
        b"nil" => KW_NIL,
        b"protocol" => KW_PROTOCOL,
        b"repeat" => KW_REPEAT,
        b"rethrows" => KW_RETHROWS,
        b"return" => KW_RETURN,
        b"self" => KW_SELF,
        b"set" => KW_SET,
        b"some" => KW_SOME,
        b"struct" => KW_STRUCT,
        b"subscript" => KW_SUBSCRIPT,
        b"super" => KW_SUPER,
        b"switch" => KW_SWITCH,
        b"throw" => KW_THROW,
        b"throws" => KW_THROWS,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"typealias" => KW_TYPEALIAS,
        b"var" => KW_VAR,
        b"where" => KW_WHERE,
        b"while" => KW_WHILE,
        b"public" => KW_PUBLIC,
        b"private" => KW_PRIVATE,
        b"fileprivate" => KW_FILEPRIVATE,
        b"internal" => KW_INTERNAL,
        b"open" => KW_OPEN,
        b"final" => KW_FINAL,
        b"static" => KW_STATIC,
        b"override" => KW_OVERRIDE,
        b"mutating" => KW_MUTATING,
        b"nonmutating" => KW_NONMUTATING,
        b"lazy" => KW_LAZY,
        b"weak" => KW_WEAK,
        b"unowned" => KW_UNOWNED,
        b"required" => KW_REQUIRED,
        b"convenience" => KW_CONVENIENCE,
        b"dynamic" => KW_DYNAMIC,
        b"indirect" => KW_INDIRECT,
        _ => IDENT,
    }
}

/// `/* ... */`: Swift's block comments nest.
fn block_comment(c: &mut Cursor) {
    c.bump_n(2);
    let mut depth = 1u32;
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
}

/// One open literal: what closes it and whether it is raw.
#[derive(Clone, Copy)]
struct Frame {
    quote: u8,
    hashes: usize,
    triple: bool,
}

thread_local! {
    /// One frame per literal the `STRING_BODY` mode is inside, innermost last:
    /// a nested literal restores the outer one on the way out.
    static STRINGS: std::cell::RefCell<Vec<Frame>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Open brackets inside each `\(...)` field, one entry per field. The
    /// frame pops at the `)` that matches the field's own `(`.
    static DEPTHS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn current_frame() -> Option<Frame> {
    STRINGS.with(|frames| frames.borrow().last().copied())
}

fn pop_frame() {
    STRINGS.with(|frames| {
        frames.borrow_mut().pop();
    });
}

/// A literal from its first byte: `"`, `"""`, `#"` or `###"`. A literal with a
/// `\(...)` field is split into start/text/field/end tokens so the field is
/// code; one without a field stays a single opaque token.
fn string_literal(c: &mut Cursor) -> Kind {
    let rest = c.rest();
    let hashes = rest.iter().take_while(|&&byte| byte == b'#').count();
    if rest.get(hashes) != Some(&b'"') {
        return BAD_LITERAL;
    }
    let triple = rest.get(hashes + 1) == Some(&b'"') && rest.get(hashes + 2) == Some(&b'"');
    let open = hashes + if triple { 3 } else { 1 };
    if has_field(rest, open, b'"', hashes, triple) {
        c.bump_n(open);
        STRINGS.with(|frames| frames.borrow_mut().push(Frame { quote: b'"', hashes, triple }));
        STRING_START
    } else {
        string(c)
    }
}

/// Whether the cursor stands on the closing quotes (and `#`s) of a literal.
/// In a raw literal a quote is only closing when its `#`s follow, so
/// `#"a "quoted" word"#` is one literal.
fn closes_literal(c: &Cursor, hashes: usize, triple: bool) -> bool {
    let quotes = if triple { 3 } else { 1 };
    if !(0..quotes).all(|at| c.peek_at(at) == b'"') {
        return false;
    }
    (0..hashes).all(|at| c.peek_at(quotes + at) == b'#')
}

/// Whether the literal body starting at `from` holds a field before its
/// closing quote. `\\(` is a field in a plain literal and `\#(` in one with
/// `#` hashes; everything else that follows a backslash is text.
fn has_field(bytes: &[u8], from: usize, quote: u8, hashes: usize, triple: bool) -> bool {
    let mut at = from;
    while at < bytes.len() {
        let byte = bytes[at];
        let quotes = if triple { 3 } else { 1 };
        let closing = (0..quotes).all(|next| bytes.get(at + next) == Some(&quote))
            && (0..hashes).all(|next| bytes.get(at + quotes + next) == Some(&b'#'));
        if closing {
            return false;
        }
        if !triple && matches!(byte, b'\n' | b'\r') {
            return false;
        }
        if byte == b'\\' {
            let width = field_width(bytes, at, hashes);
            if width > 0 {
                return true;
            }
            at += if hashes == 0 { 2 } else { 1 };
            continue;
        }
        at += 1;
    }
    false
}

/// The byte width of the field opener at `at`, or 0 for an escape that is text.
fn field_width(bytes: &[u8], at: usize, hashes: usize) -> usize {
    if bytes.get(at) != Some(&b'\\') {
        return 0;
    }
    if hashes == 0 {
        return usize::from(bytes.get(at + 1) == Some(&b'(')) * 2;
    }
    let mut seen = 0;
    while seen < hashes && bytes.get(at + 1 + seen) == Some(&b'#') {
        seen += 1;
    }
    if seen == hashes && bytes.get(at + 1 + hashes) == Some(&b'(') { hashes + 2 } else { 0 }
}

/// The `STRING_BODY` mode: the literal's text up to a field, its closing
/// quotes, or (in a one-line literal) a line break, which ends it badly.
pub fn lex_string_text(c: &mut Cursor) -> Kind {
    let Some(frame) = current_frame() else {
        // The frame stack and the mode stack disagree; take one byte as text so
        // lexing always advances and the parser still gets a tree.
        c.bump();
        return STRING_TEXT;
    };
    if c.at_end() {
        pop_frame();
        return STRING_BAD;
    }
    let quote = frame.quote;
    let triple = frame.triple;
    let closes = closes_literal(c, frame.hashes, triple);
    if closes {
        c.bump_n(if triple { 3 } else { 1 });
        let mut seen = 0usize;
        while seen < frame.hashes && c.peek() == b'#' {
            c.bump();
            seen += 1;
        }
        pop_frame();
        return if seen == frame.hashes { STRING_END } else { STRING_BAD };
    }
    if !triple && matches!(c.peek(), b'\n' | b'\r') {
        c.bump();
        c.eat(b'\n');
        pop_frame();
        return STRING_BAD;
    }
    if field_width(c.rest(), 0, frame.hashes) > 0 {
        c.bump_n(field_width(c.rest(), 0, frame.hashes));
        DEPTHS.with(|depths| depths.borrow_mut().push(0));
        return INTERP_OPEN;
    }
    let start = c.pos();
    loop {
        if c.at_end() {
            break;
        }
        if closes_literal(c, frame.hashes, triple) {
            break;
        }
        if !triple && matches!(c.peek(), b'\n' | b'\r') {
            break;
        }
        if field_width(c.rest(), 0, frame.hashes) > 0 {
            break;
        }
        if c.peek() == b'\\' {
            c.bump_n(if frame.hashes == 0 { 2 } else { 1 });
            continue;
        }
        c.bump_char();
    }
    if c.pos() == start {
        c.bump_char();
    }
    STRING_TEXT
}

/// The `INTERPOLATION` mode: ordinary code until the `)` that matches the
/// field's `(`. Brackets inside the field are counted so a call, a closure or
/// a subscript keeps its own close.
pub fn lex_interpolation(c: &mut Cursor) -> Kind {
    if c.at_end() {
        DEPTHS.with(|depths| {
            depths.borrow_mut().pop();
        });
        return STRING_BAD;
    }
    let depth = DEPTHS.with(|depths| depths.borrow().last().copied()).unwrap_or(0);
    if c.peek() == b')' && depth == 0 {
        c.bump();
        DEPTHS.with(|depths| {
            depths.borrow_mut().pop();
        });
        return INTERP_CLOSE;
    }
    let kind = lex(c);
    match kind {
        LPAREN | LBRACKET | LBRACE => DEPTHS.with(|depths| {
            if let Some(top) = depths.borrow_mut().last_mut() {
                *top += 1;
            }
        }),
        RPAREN | RBRACKET | RBRACE => DEPTHS.with(|depths| {
            if let Some(top) = depths.borrow_mut().last_mut() {
                *top = top.saturating_sub(1);
            }
        }),
        _ => {}
    }
    kind
}

/// A string from its first byte: plain, multi-line (`"""`) or raw (`#"`).
fn string(c: &mut Cursor) -> Kind {
    let mut hashes = 0usize;
    while c.peek() == b'#' {
        hashes += 1;
        c.bump();
    }
    if c.peek() != b'"' {
        return BAD_LITERAL;
    }
    let triple = c.peek_at(1) == b'"' && c.peek_at(2) == b'"';
    let open = if triple { 3 } else { 1 };
    c.bump_n(open);
    loop {
        match c.peek() {
            0 if c.at_end() => return BAD_LITERAL,
            b'\\' if hashes == 0 => {
                if c.peek_at(1) == b'(' {
                    c.bump_n(2);
                    interpolation(c);
                } else {
                    c.bump_n(2);
                }
            }
            b'\\' if c.peek_at(1) == b'#' => {
                let mut at = 1;
                while at <= hashes && c.peek_at(at) == b'#' {
                    at += 1;
                }
                if at == hashes + 1 && c.peek_at(at) == b'(' {
                    c.bump_n(at + 1);
                    interpolation(c);
                } else {
                    c.bump_n(at);
                }
            }
            b'"' if closes_literal(c, hashes, triple) => {
                c.bump_n(if triple { 3 } else { 1 });
                return closing_hashes(c, hashes);
            }
            _ => c.bump(),
        }
    }
}

/// The `#`s after a closing quote of a raw string, which must be there.
fn closing_hashes(c: &mut Cursor, hashes: usize) -> Kind {
    let mut seen = 0usize;
    while seen < hashes && c.peek() == b'#' {
        c.bump();
        seen += 1;
    }
    if seen == hashes { STRING } else { BAD_LITERAL }
}

/// The code of a `\(...)` field, up to its matching `)`, with strings and
/// nested parentheses balanced.
fn interpolation(c: &mut Cursor) {
    let mut depth = 1u32;
    while !c.at_end() {
        match c.peek() {
            b'(' => {
                depth += 1;
                c.bump();
            }
            b')' => {
                depth -= 1;
                c.bump();
                if depth == 0 {
                    return;
                }
            }
            b'"' => {
                string(c);
            }
            b'#' if c.peek_at(1) == b'"' => {
                string(c);
            }
            _ => c.bump(),
        }
    }
}

fn number(c: &mut Cursor) -> Kind {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_hexdigit() || byte == b'_');
        if c.peek() == b'.' {
            c.bump();
            c.eat_while(|byte| byte.is_ascii_hexdigit() || byte == b'_');
        }
        if matches!(c.peek(), b'p' | b'P') {
            c.bump();
            if matches!(c.peek(), b'+' | b'-') {
                c.bump();
            }
            c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
        }
        return if c.peek() == b'.' || false { FLOAT } else { NUMBER };
    }
    if c.peek() == b'0' && matches!(c.peek_at(1), b'b' | b'B' | b'o' | b'O') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return NUMBER;
    }
    let mut float = false;
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.peek() == b'.' && c.peek_at(1).is_ascii_digit() {
        float = true;
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'e' | b'E') && (c.peek_at(1).is_ascii_digit() || (matches!(c.peek_at(1), b'+' | b'-') && c.peek_at(2).is_ascii_digit())) {
        float = true;
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if float { FLOAT } else { NUMBER }
}

#[cfg(test)]
mod tests {
    //! The corpus under `fixtures/swift_corpus/` is real-shaped Swift (the box
    //! has no `.swift` checkout to read, so the files are hand-written from
    //! idiomatic Foundation/SwiftUI/concurrency code). Each file must parse
    //! completely with no error node; the ignored inventory prints what is
    //! still red, and `PROBES` pins one construct per entry.

    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::deep::syntax::ERROR;
    use crate::deep::tree::HAS_ERROR;
    use crate::deep::{Budget, Workspace};

    use super::SWIFT;

    const PROBES: &[(&str, &str)] = &[
        ("argument_labels", "func move(from start: Point, to end: Point) {}"),
        ("optionals", "func find(_ id: Int?) -> String! { return nil }"),
        ("trailing_closure", "let done = items.filter { item in item.ready }.map { $0 }"),
        ("guard", "func f(_ x: Int?) -> Int {\n    guard let value = x else { return 0 }\n    return value\n}"),
        ("generic_constraint", "func largest<T: Comparable>(_ values: [T]) -> T? where T: Hashable { return nil }"),
        ("interpolation_call", "let text = \"total \\(format(count)) items\""),
        ("async_await", "func load() async throws -> Data {\n    let (data, _) = try await session.data(for: request)\n    return data\n}"),
        ("protocol_extension", "protocol P { func run() }\nextension P { func twice() { run(); run() } }"),
        ("enum_associated_values", "enum Result {\n    case success(Int)\n    case failure(String, code: Int)\n}"),
        ("tuple_binding", "let (data, response) = try await load()"),
        ("labeled_loop", "outer: for (left, right) in pairs {\n    break outer\n}"),
        ("if_case", "for value in values {\n    if case let text as String = value { return text }\n}"),
        ("catch_typed", "do { try run() } catch let error as CocoaError { print(error) }"),
        ("for_where", "for value in values where value > 0 { print(value) }"),
        ("escaping_closure_type", "func withDefaults(_ handler: @escaping (Result<[String], Error>) -> Void) {}"),
        ("closure_array", "let operations: [(String) -> Int] = [\n    { text in text.count },\n    { text in text.uppercased().count * 2 },\n]"),
        ("catch_block", "func f() {\n    do {\n        try run()\n    } catch {\n        if attempt < 3 {\n            sleep()\n        }\n    }\n}"),
        ("init_defaults", "struct S {\n    init(path: String, query: [String: String] = [:]) {}\n}"),
        ("array_trailing_comma", "let values = [1, 2,];"),
        ("array_closure_last", "let values = [1, { text in text.count },];"),
        ("array_closure_no_comma", "let values = [1, { text in text.count }];"),
    ];

    fn parse(src: &[u8]) -> (bool, usize, String) {
        let mut workspace = Workspace::default();
        let outcome = workspace.parse(src, &SWIFT, &Budget::default()).expect("input too large");
        let nodes = &workspace.tree.nodes;
        let errors = (0..nodes.len())
            .filter(|&i| nodes[i].kind == ERROR || nodes[i].flags & HAS_ERROR != 0 && nodes[i].flags & crate::deep::tree::TOKEN != 0)
            .count();
        (outcome.complete(), errors, workspace.tree.sexp(&SWIFT, true))
    }

    fn corpus() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/swift_corpus")
    }

    fn corpus_files() -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(corpus())
            .expect("fixtures/swift_corpus")
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "swift"))
            .collect();
        files.sort();
        files
    }

    /// The field of a literal is code: `\(format(count))` is an
    /// `interpolation` node whose child call has exactly that span, so a scan
    /// that counts call sites sees the call inside a string.
    #[test]
    fn interpolation_is_a_span_of_code() {
        const SRC: &str =
            "let text = \"total \\(format(count)) items\"\nlet nested = \"outer \\(items.map { value in \"\\(value)\" }.joined())\"\nlet raw = #\"no \\(field) here\"#\n";
        let mut workspace = Workspace::default();
        workspace.parse(SRC.as_bytes(), &SWIFT, &Budget::default()).expect("input too large");
        let nodes = &workspace.tree.nodes;
        let text = |index: usize| {
            let node = &nodes[index];
            std::str::from_utf8(&SRC.as_bytes()[node.start as usize..node.end as usize]).unwrap()
        };
        let kinds = |wanted: &str| -> Vec<usize> {
            (0..nodes.len()).filter(|&index| SWIFT.name(nodes[index].kind) == wanted).collect()
        };

        let literals = kinds("string_literal");
        assert_eq!(literals.len(), 3, "the first literal, the nested one, and its inner literal");
        let interpolations = kinds("interpolation");
        assert_eq!(interpolations.len(), 3, "one field in the first literal, two in the nested one");

        let first = interpolations[0];
        assert_eq!(text(first), "\\(format(count))", "the whole field is the node's span");
        let children: Vec<&str> = workspace
            .tree
            .children(first as u32)
            .map(|child| SWIFT.name(nodes[child as usize].kind))
            .filter(|kind| !matches!(*kind, "interpolation_open" | "interpolation_close"))
            .collect();
        assert_eq!(children, ["call_expression"], "the field's expression is a child of the field");

        let calls = kinds("call_expression");
        let inside: Vec<&str> = calls.iter().map(|&index| text(index)).collect();
        assert!(inside.contains(&"format(count)"), "the call inside the field is visible: {inside:?}");
        assert_eq!(text(interpolations[2]), "\\(value)", "the nested literal's field is its own span");
        assert!(
            calls.iter().any(|&index| text(index) == "items.map { value in \"\\(value)\" }.joined()"),
            "the outer call keeps its trailing closure and the nested literal: {inside:?}"
        );

        let raw = kinds("string");
        assert_eq!(raw.len(), 1, "a raw literal with no field stays one opaque token");
        assert_eq!(text(raw[0]), "#\"no \\(field) here\"#");
    }

    #[test]
    fn corpus_parses_cleanly() {
        let files = corpus_files();
        assert!(files.len() >= 8, "the corpus needs at least eight files, has {}", files.len());
        for path in &files {
            let src = fs::read(path).unwrap();
            let (complete, errors, tree) = parse(&src);
            assert!(complete, "{}: the parse did not finish", path.display());
            assert_eq!(errors, 0, "{}: {errors} error nodes:\n{tree}", path.display());
        }
    }

    #[test]
    fn every_probe_parses_without_an_error_node() {        let mut failed = Vec::new();
        for (name, src) in PROBES {
            let (complete, errors, tree) = parse(src.as_bytes());
            if !complete || errors > 0 {
                failed.push(format!("{name}: complete={complete} errors={errors}\n{tree}"));
            }
        }
        assert!(failed.is_empty(), "{} of {} probes failed:\n{}", failed.len(), PROBES.len(), failed.join("\n"));
    }

    #[test]
    #[ignore]
    fn inventory() {
        for path in corpus_files() {
            let src = fs::read(&path).unwrap();
            let (complete, errors, _) = parse(&src);
            let name = path.file_name().unwrap().to_string_lossy();
            println!("{name}: complete={complete} errors={errors}");
            if errors > 0 {
                println!("{}", error_context(&src));
            }
        }
        for (name, src) in PROBES {
            let (complete, errors, tree) = parse(src.as_bytes());
            let status = if complete && errors == 0 { "ok" } else { "FAIL" };
            println!("probe {name}: {status} complete={complete} errors={errors}");
            if errors > 0 {
                println!("{}", error_context(src.as_bytes()));
                let cut = tree.len().min(1200);
                println!("{}", &tree[..cut]);
            }
        }
    }

    /// Every error node's span and the source around it, for retuning.
    fn error_context(src: &[u8]) -> String {
        let mut workspace = Workspace::default();
        workspace.parse(src, &SWIFT, &Budget::default()).expect("input too large");
        let nodes = &workspace.tree.nodes;
        let mut out = String::new();
        for (index, node) in nodes.iter().enumerate() {
            let bad = node.kind == ERROR
                || node.flags & HAS_ERROR != 0 && node.flags & crate::deep::tree::TOKEN != 0;
            if !bad {
                continue;
            }
            let start = node.start as usize;
            let end = node.end as usize;
            let from = start.saturating_sub(50);
            let to = (end + 50).min(src.len());
            let text = String::from_utf8_lossy(&src[from..to]);
            out.push_str(&format!("#{index} {} {}..{end} 「{}」\n", SWIFT.name(node.kind), start, text.replace('\n', "\\n")));
        }
        out
    }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'<', _, _) if is_word_byte(c.behind(1)) && !matches!(b, b'=' | b'<') => (GENERIC_LT, 1),
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'.', b'.', b'<') => (RANGE_OPEN, 3),
        (b'.', b'.', _) => (DOTDOT, 2),
        (b'=', b'=', b'=') => (STRICT_EQ, 3),
        (b'!', b'=', b'=') => (STRICT_NE, 3),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'?', b'.', _) if !d.is_ascii_digit() => (OPTIONAL_CHAIN, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'?', b'?', _) => (NULLISH, 2),
        (b'<', b'<', _) => (SHL, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'>', b'>', _) => (GT_JOINED, 1),
        (b'\\', ..) => (BACKSLASH, 1),
        (b'(', ..) => (LPAREN, 1),        (b')', ..) => (RPAREN, 1),
        (b'[', ..) => (LBRACKET, 1),
        (b']', ..) => (RBRACKET, 1),
        (b'{', ..) => (LBRACE, 1),
        (b'}', ..) => (RBRACE, 1),
        (b';', ..) => (SEMI, 1),
        (b',', ..) => (COMMA, 1),
        (b'.', ..) => (DOT, 1),
        (b':', ..) => (COLON, 1),
        (b'?', ..) => (QUESTION, 1),
        (b'=', ..) => (ASSIGN, 1),
        (b'+', ..) => (PLUS, 1),
        (b'-', ..) => (MINUS, 1),
        (b'*', ..) => (STAR, 1),
        (b'/', ..) => (SLASH, 1),
        (b'%', ..) => (PERCENT, 1),
        (b'|', ..) => (PIPE, 1),
        (b'&', ..) => (AMP, 1),
        (b'^', ..) => (CARET, 1),
        (b'~', ..) => (TILDE, 1),
        (b'!', ..) => (BANG, 1),
        (b'<', ..) => (LT, 1),
        (b'>', ..) => (GT, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    c.bump_n(width);
    kind
}
