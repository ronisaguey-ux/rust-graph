//! Crystal: `def`/`class`/`struct`/`module`/`enum`/`lib` definitions, calls,
//! assignments and the `end`-delimited control forms, plus a step lexer for its
//! comments, strings, symbols, regexes, numbers and sigils.
//!
//! Crystal is Ruby-shaped but stricter: no command calls without parentheses in
//! this grammar either (see the gaps), `end` closes every construct, and `do`
//! ... `end` and `{ ... }` are block forms. The tree is Tree-sitter's Crystal
//! where that costs nothing; it is not the same tree.
//!
//! # What is modelled
//!
//! - **Definitions.** `require`, `include`/`extend`/`prepend`, `class` with a
//!   superclass and type parameters, `struct`, `module`, `enum`, `lib` with
//!   `fun`/`struct` declarations, `annotation`, `alias`, `def` (with a receiver,
//!   parameters, a return type and a body), `private`/`protected` prefixes.
//! - **Statements.** `if`/`elsif`/`else`, `unless`, `while`, `until`,
//!   `for ... in`, `case`/`when`/`else`, `begin`/`rescue`/`ensure`, `return`,
//!   `break`, `next`, `yield`, and modifiers are not statements (see below).
//! - **Expressions.** One precedence table from assignment down to calls,
//!   indexing, member access and blocks; `foo(x)` is a call and `xs.each { }` /
//!   `xs.each do ... end` are call-with-block rows.
//! - **Calls without parentheses** for a literal argument: `puts "text"`,
//!   `raise Exception.new("x")` is not (it is an expression), but `p 42` is.
//! - **The lexer** knows `#` comments, `"..."` with `#{}` interpolation and
//!   escapes, `'...'`, `%w[...]`/`%q(...)`-style literals, heredocs, `:symbol`,
//!   `@ivar`, `@@cvar`, `$global`, `/regex/` against `/` division, and numbers
//!   with `_` and suffixes.
//!
//! # What is missing, named here rather than accepted silently
//!
//! - **Command calls with a non-literal argument.** `puts x` is not a call
//!   here: a newline is whitespace, so `a` then `b` on the next line would read
//!   as `a(b)`. Write `puts(x)`; `puts "text"` works because a literal can
//!   never start a statement.
//! - **Macros.** `macro name ... end` bodies are token soup with `{{ ... }}`
//!   interpolation, so a `macro` definition is not modelled at all.
//! - **String interpolation is opaque.** `"#{a + b}"` is one string token; the
//!   code inside is not parsed.
//! - **Modifier forms.** `return x if y` parses `if y` as a new statement, not
//!   as a modifier on the first.
//! - **`select`/`with`/`pointerof`** and `asm` blocks are not modelled.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { CRYSTAL {
    name = "crystal";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,

        IDENT = "identifier",
        CONSTANT = "constant",
        NUMBER = "number",
        STRING = "string",
        SYMBOL = "symbol",
        REGEX = "regex",
        INSTANCE_VAR = "instance_variable",
        CLASS_VAR = "class_variable",
        GLOBAL_VAR = "global_variable",
        BAD_LITERAL = "bad_literal" bad,

        KW_ABSTRACT = "abstract",
        KW_ALIAS = "alias",
        KW_ANNOTATION = "annotation",
        KW_AS = "as",
        KW_BEGIN = "begin",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CLASS = "class",
        KW_DEF = "def",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ELSIF = "elsif",
        KW_END = "end",
        KW_ENSURE = "ensure",
        KW_ENUM = "enum",
        KW_EXTEND = "extend",
        KW_FALSE = "false",
        KW_FOR = "for",
        KW_FUN = "fun",
        KW_IF = "if",
        KW_IN = "in",
        KW_INCLUDE = "include",
        KW_LIB = "lib",
        KW_MODULE = "module",
        KW_NEXT = "next",
        KW_NIL = "nil",
        KW_OF = "of",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_REQUIRE = "require",
        KW_RESCUE = "rescue",
        KW_RETURN = "return",
        KW_SELF = "self",
        KW_STRUCT = "struct",
        KW_SUPER = "super",
        KW_THEN = "then",
        KW_TRUE = "true",
        KW_TYPE = "type",
        KW_TYPEOF = "typeof",
        KW_UNLESS = "unless",
        KW_UNTIL = "until",
        KW_WHEN = "when",
        KW_WHILE = "while",
        KW_WITH = "with",
        KW_YIELD = "yield",
        KW_ASM = "asm",
        KW_OFFSETOF = "offsetof",
        KW_SIZEOF = "sizeof",
        KW_INSTANCE_SIZEOF = "instance_sizeof",
        KW_POINTEROF = "pointerof",
        KW_UNINITIALIZED = "uninitialized",
        KW_UNION = "union",
        KW_SELECT = "select",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        COLON = ":",
        COLONCOLON = "::",
        QUESTION = "?",
        FATARROW = "=>",
        ARROW = "->",
        ASSIGN = "=",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        STARSTAR_EQ = "**=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        ANDAND_EQ = "&&=",
        OROR_EQ = "||=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        STARSTAR = "**",
        SLASH = "/",
        PERCENT = "%",
        PIPE = "|",
        AMP = "&",
        CARET = "^",
        TILDE = "~",
        BANG = "!",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        CMP = "<=>",
        EQ = "==",
        TEQ = "===",
        NE = "!=",
        MATCH = "=~",
        NMATCH = "!~",
        ANDAND = "&&",
        OROR = "||",
        SHL = "<<",
        SHR = ">>",
        DOTDOT = "..",
        ELLIPSIS = "...",
        AT = "@",
        DOLLAR = "$",
    }

    program: _statement*,

    _statement:
        require_statement
        | include_statement
        | module_definition
        | class_definition
        | struct_definition
        | enum_definition
        | lib_definition
        | annotation_definition
        | alias_declaration
        | method_definition
        | if_statement
        | unless_statement
        | while_statement
        | until_statement
        | for_statement
        | case_statement
        | begin_statement
        | return_statement
        | break_statement
        | next_statement
        | &(_name _command_argument) command_call
        | instance_variable_declaration
        | expression_statement
        | ";",

    require_statement: "require" _expression,
    include_statement: ("include" | "extend") _constant_path [generic_arguments],

    module_definition: "module" _constant_path body "end",
    class_definition: ["abstract"] "class" _constant_path [generic_parameters] ["<" _type] body "end",
    struct_definition: ["abstract"] "struct" _constant_path [generic_parameters] ["<" _type] body "end",

    enum_definition: "enum" _constant_path [":" _type] body "end",

    lib_definition: "lib" _constant_path lib_body "end",
    lib_body: _lib_member*,
    _lib_member:
        fun_declaration
        | struct_definition
        | alias_declaration
        | type_declaration,
    fun_declaration: "fun" IDENT [parameters] [":" _type],
    type_declaration: "type" IDENT "=" _type,

    annotation_definition: "annotation" _constant_path body "end",
    alias_declaration: "alias" CONSTANT "=" _type,

    method_definition: [visibility] ["abstract"] "def" method_name [parameters] [":" _type] body "end",
    visibility: "private" | "protected",
    method_name: ["self" "."] _method_ident,
    _method_ident: IDENT,
    parameters: "(" (_parameter %? ",") ")",
    _parameter: ["*" | "**" | "&"] parameter_name [":" _type] ["=" _expression@assign],
    parameter_name: IDENT | INSTANCE_VAR | CLASS_VAR,
    generic_parameters: "(" (_generic_parameter %? ",") ")",
    _generic_parameter: CONSTANT ["<" _type],

    body: _statement*,

    if_statement: "if" _expression [then_token] body elsif_clause* [else_clause] "end",
    elsif_clause: "elsif" _expression [then_token] body,
    else_clause: "else" body,
    then_token: "then",
    unless_statement: "unless" _expression [then_token] body [else_clause] "end",
    while_statement: "while" _expression body "end",
    until_statement: "until" _expression body "end",
    for_statement: "for" _name ["," _name] "in" _expression body "end",

    case_statement: "case" [_expression] when_clause+ [else_clause] "end",
    when_clause: "when" (_expression % ",") [then_token] body,

    begin_statement: "begin" body rescue_clause* [else_clause] [ensure_clause] "end",
    rescue_clause: "rescue" [_rescue_list] body,
    _rescue_list: _constant_path ("," _constant_path)*,
    ensure_clause: "ensure" body,

    return_statement: "return" [_expression ("," _expression)*],
    break_statement: "break" [_expression],
    next_statement: "next" [_expression],
    expression_statement: _expression,

    command_call: _name _command_argument+,
    _command_argument: _literal | _name [":" _type] | array | hash | parenthesized_expression,

    instance_variable_declaration: (INSTANCE_VAR | CLASS_VAR) ":" _type ["=" _expression@assign],

    brace_block: "{" [block_parameters] _statement* "}",
    do_block: "do" [block_parameters] _statement* "end",
    block_parameters: "|" (_name %? ",") "|",

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "**=" | "&=" | "|=" | "^=" | "&&=" | "||=" | "<<=" | ">>=" => assignment,
        cond: postfix "?" _expression ":" _expression@assign => conditional,
        range: left ".." | "..." => range,
        or: left "||" => binary,
        and: left "&&" => binary,
        bor: left "|" => binary,
        xor: left "^" => binary,
        band: left "&" => binary,
        eq: left "==" | "!=" | "===" | "=~" | "!~" | "<=>" => binary,
        cmp: left "<" | ">" | "<=" | ">=" => binary,
        shift: left "<<" | ">>" => binary,
        add: left "+" | "-" => binary,
        mul: left "*" | "/" | "%" => binary,
        exp: right "**" => binary,
        unary: prefix "!" | "-" | "+" | "~" => unary,
        block: postfix brace_block => call_with_block,
        do_block: postfix do_block => call_with_block,
        call: postfix arguments => call,
        index: postfix subscript => element_reference,
        member: postfix "." _property => member,
        scope: postfix "::" CONSTANT => scope,

    },

    _primary:
        _literal
        | _variable
        | _name
        | CONSTANT
        | "self"
        | parenthesized_expression
        | array
        | hash
        | proc_literal
        | super_call
        | yield_expression
        | typeof_expression,
    _literal: NUMBER | STRING | SYMBOL | REGEX | BAD_LITERAL | "nil" | "true" | "false",
    _name: IDENT,
    _variable: INSTANCE_VAR | CLASS_VAR | GLOBAL_VAR,
    super_call: "super" [arguments],
    yield_expression: "yield" [arguments],
    typeof_expression: "typeof" typeof_arguments,
    typeof_arguments: "(" _expression ")",
    proc_literal: "->" [parameters] proc_body,
    proc_body: "{" _statement* "}",
    parenthesized_expression: "(" _expression ")",
    array: "[" (_expression %? ",") "]",
    hash: "{" (_hash_entry %? ",") "}",
    _hash_entry: &(_name ":") label_pair | arrow_pair,
    label_pair: _name ":" _expression,
    arrow_pair: _expression@bor "=>" _expression,
    arguments: "(" (_argument %? ",") ")",
    _argument: &(_name ":") label_argument | _expression,
    label_argument: _name ":" _expression,
    subscript: "[" (_expression %? ",") "]",

    generic_arguments: "(" (type_argument %? ",") ")",
    type_argument: _name ":" _type | _type,
    _type: type_path [_type_tail],
    type_path: (_name | CONSTANT) ("::" CONSTANT)*,
    _type_tail: union_type | nilable_type | pointer_type | generic_arguments,
    union_type: "|" _type,
    nilable_type: "?",
    pointer_type: "*",

    _constant_path: CONSTANT ("::" CONSTANT)*,

    /// After `.` every keyword is a method name too: `x.class`.
    _property:
        IDENT | "class" | "then" | "begin" | "end" | "nil" | "self" | "true" | "false"
        | "type" | "union" | "of" | "in" | "do" | "as" | "case" | "when" | "if"
        | "unless" | "while" | "return" | "yield" | "def" | "module" | "require"
        | "super" | "sizeof" | "pointerof" | "instance_sizeof" | "offsetof"
        | "uninitialized" | "select" | "with" | "asm" | "fun" | "lib" | "alias"
        | "struct" | "enum" | "annotation" | "abstract" | "private" | "protected"
        | "extend" | "include" | "until" | "elsif" | "else" | "rescue" | "ensure"
        | "next" | "break" | "for",
} }
