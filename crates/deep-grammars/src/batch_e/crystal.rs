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

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
            WHITESPACE
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'#' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        b'"' => if string(c) { STRING } else { BAD_LITERAL },
        b'\'' => {
            c.bump();
            if lex::quoted(c, b'\'') { STRING } else { BAD_LITERAL }
        }
        b'%' if !ends_operand(c.previous()) && percent_delimiter(c.peek_at(1)) => percent_literal(c),
        b'<' if c.peek_at(1) == b'<' && !ends_operand(c.previous()) && heredoc_start(c) => heredoc(c),
        b'/' if c.peek_at(1) != b'=' && !ends_operand(c.previous()) => {
            if regex(c) {
                REGEX
            } else {
                SLASH
            }
        }
        b':' if c.peek_at(1) != b':' && is_symbol_start(c.peek_at(1)) && !ends_operand(c.previous()) => symbol(c),
        b'@' if is_word_start(c.peek_at(1)) || c.peek_at(1) == b'@' => variable(c),
        b'$' if is_word_start(c.peek_at(1)) || c.peek_at(1).is_ascii_digit() => variable(c),
        b'0'..=b'9' => {
            lex::number(c);
            NUMBER
        }
        b'.' if c.peek_at(1).is_ascii_digit() && !ends_operand(c.previous()) => {
            c.bump();
            lex::number(c);
            NUMBER
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
fn is_symbol_start(byte: u8) -> bool {
    is_word_start(byte) || matches!(byte, b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' | b'!' | b'?' | b'@' | b'[' | b']')
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// A token that ends an operand, so a following `/`, `%`, `<` or `:` is an
/// operator, not the start of a literal.
fn ends_operand(previous: Kind) -> bool {
    matches!(
        previous,
        IDENT | CONSTANT | NUMBER | STRING | SYMBOL | REGEX | INSTANCE_VAR | CLASS_VAR | GLOBAL_VAR
            | RPAREN | RBRACKET | RBRACE | KW_SELF | KW_NIL | KW_TRUE | KW_FALSE
            | KW_END | KW_SUPER | KW_YIELD
    )
}

/// `@name`, `@@name` or `$name`: one variable token.
fn variable(c: &mut Cursor) -> Kind {
    let sigil = c.peek();
    c.bump();
    if sigil == b'@' && c.peek() == b'@' {
        c.bump();
        c.eat_while(is_word_byte);
        return CLASS_VAR;
    }
    c.eat_while(is_word_byte);
    if sigil == b'@' { INSTANCE_VAR } else { GLOBAL_VAR }
}

/// An identifier, a method name ending in `?`/`!`, or a constant.
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    let suffix = matches!(c.peek(), b'?' | b'!') && c.peek_at(1) != b'=' && !is_word_byte(c.peek_at(1));
    if suffix {
        c.bump();
        return IDENT;
    }
    if text[0].is_ascii_uppercase() {
        return CONSTANT;
    }
    keyword(text)
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"abstract" => KW_ABSTRACT,
        b"alias" => KW_ALIAS,
        b"annotation" => KW_ANNOTATION,
        b"as" => KW_AS,
        b"asm" => KW_ASM,
        b"begin" => KW_BEGIN,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"class" => KW_CLASS,
        b"def" => KW_DEF,
        b"do" => KW_DO,
        b"else" => KW_ELSE,
        b"elsif" => KW_ELSIF,
        b"end" => KW_END,
        b"ensure" => KW_ENSURE,
        b"enum" => KW_ENUM,
        b"extend" => KW_EXTEND,
        b"false" => KW_FALSE,
        b"for" => KW_FOR,
        b"fun" => KW_FUN,
        b"if" => KW_IF,
        b"in" => KW_IN,
        b"include" => KW_INCLUDE,
        b"instance_sizeof" => KW_INSTANCE_SIZEOF,
        b"lib" => KW_LIB,
        b"module" => KW_MODULE,
        b"next" => KW_NEXT,
        b"nil" => KW_NIL,
        b"of" => KW_OF,
        b"offsetof" => KW_OFFSETOF,
        b"pointerof" => KW_POINTEROF,
        b"private" => KW_PRIVATE,
        b"protected" => KW_PROTECTED,
        b"require" => KW_REQUIRE,
        b"rescue" => KW_RESCUE,
        b"return" => KW_RETURN,
        b"select" => KW_SELECT,
        b"self" => KW_SELF,
        b"sizeof" => KW_SIZEOF,
        b"struct" => KW_STRUCT,
        b"super" => KW_SUPER,
        b"then" => KW_THEN,
        b"true" => KW_TRUE,
        b"type" => KW_TYPE,
        b"typeof" => KW_TYPEOF,
        b"uninitialized" => KW_UNINITIALIZED,
        b"union" => KW_UNION,
        b"unless" => KW_UNLESS,
        b"until" => KW_UNTIL,
        b"when" => KW_WHEN,
        b"while" => KW_WHILE,
        b"with" => KW_WITH,
        b"yield" => KW_YIELD,
        _ => IDENT,
    }
}

/// A double-quoted string with `#{}` interpolation: the field's braces and any
/// string inside them are balanced and stay opaque.
fn string(c: &mut Cursor) -> bool {
    c.bump();
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'\\' => c.bump_n(2),
            b'"' => {
                c.bump();
                return true;
            }
            b'#' if c.peek_at(1) == b'{' => {
                c.bump_n(2);
                interpolation(c);
            }
            b'\n' | b'\r' => return false,
            _ => c.bump_char(),
        }
    }
}

/// The body of a `#{...}` after its brace, through the closing brace.
fn interpolation(c: &mut Cursor) {
    let mut depth = 1u32;
    while !c.at_end() {
        match c.peek() {
            b'{' => {
                depth += 1;
                c.bump();
            }
            b'}' => {
                c.bump();
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            b'"' | b'\'' => {
                let quote = c.peek();
                c.bump();
                lex::quoted(c, quote);
            }
            b'\n' | b'\r' => return,
            _ => c.bump_char(),
        }
    }
}

/// `:name`, `:+` or `:"quoted"`.
fn symbol(c: &mut Cursor) -> Kind {
    c.bump();
    if c.peek() == b'"' {
        return if lex::quoted(c, b'"') { SYMBOL } else { BAD_LITERAL };
    }
    if is_word_start(c.peek()) {
        c.eat_while(is_word_byte);
        if matches!(c.peek(), b'?' | b'!' | b'=') && !is_word_byte(c.peek_at(1)) {
            c.bump();
        }
        return SYMBOL;
    }
    c.bump();
    SYMBOL
}

/// `%w[...]`, `%i(...)`, `%q{...}` and friends: one token, delimiter-balanced.
fn percent_literal(c: &mut Cursor) -> Kind {
    c.bump();
    c.eat(b'w') || c.eat(b'i') || c.eat(b'q') || c.eat(b'Q') || c.eat(b'r') || c.eat(b'x');
    let open = c.peek();
    let close = match open {
        b'[' => b']',
        b'(' => b')',
        b'{' => b'}',
        b'<' => b'>',
        b'|' => b'|',
        b'/' => b'/',
        _ => return BAD_LITERAL,
    };
    c.bump();
    let mut depth = 1u32;
    while !c.at_end() {
        match c.peek() {
            b'\\' => c.bump_n(2),
            byte if byte == open && close != open => {
                depth += 1;
                c.bump();
            }
            byte if byte == close => {
                depth -= 1;
                c.bump();
                if depth == 0 {
                    return STRING;
                }
            }
            _ => c.bump_char(),
        }
    }
    BAD_LITERAL
}

#[inline]
fn percent_delimiter(byte: u8) -> bool {
    matches!(byte, b'w' | b'i' | b'q' | b'Q' | b'r' | b'x' | b'[' | b'(' | b'{' | b'<' | b'|' | b'/')
}

/// `<<-EOS`, `<<~EOS` or `<<"EOS"` at the start of a heredoc.
fn heredoc_start(c: &mut Cursor) -> bool {
    let mut at = 2;
    if matches!(c.peek_at(at), b'-' | b'~') {
        at += 1;
    }
    is_word_start(c.peek_at(at)) || c.peek_at(at) == b'"'
}

/// A heredoc from its `<<` through the terminator line.
fn heredoc(c: &mut Cursor) -> Kind {
    c.bump_n(2);
    c.eat(b'-');
    c.eat(b'~');
    let quoted = c.peek() == b'"';
    if quoted {
        c.bump();
    }
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let terminator = start[..length].to_vec();
    if quoted {
        c.eat(b'"');
    }
    loop {
        if c.at_end() {
            return BAD_LITERAL;
        }
        c.eat_while(|byte| byte != b'\n');
        c.eat(b'\n');
        // A terminator is the whole line, indented or not.
        c.eat_while(|byte| byte == b' ' || byte == b'\t');
        if c.starts_with(&terminator) && matches!(c.peek_at(terminator.len()), b'\n' | b'\r' | 0) {
            c.bump_n(terminator.len());
            return STRING;
        }
    }
}

/// A `/.../` regex from its opening slash. False when the line ends first.
fn regex(c: &mut Cursor) -> bool {
    c.bump();
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'\\' => c.bump_n(2),
            b'/' => {
                c.bump();
                // Flags.
                c.eat_while(|byte| byte.is_ascii_alphabetic());
                return true;
            }
            b'\n' | b'\r' => return false,
            _ => c.bump_char(),
        }
    }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'*', b'*', b'=') => (STARSTAR_EQ, 3),
        (b'&', b'&', b'=') => (ANDAND_EQ, 3),
        (b'|', b'|', b'=') => (OROR_EQ, 3),
        (b'=', b'=', b'=') => (TEQ, 3),
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'<', b'<', _) => (SHL, 2),
        (b'>', b'>', _) => (SHR, 2),
        (b'<', b'=', b'>') => (CMP, 3),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'=', b'~', _) => (MATCH, 2),
        (b'!', b'~', _) => (NMATCH, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'*', b'*', _) => (STARSTAR, 2),
        (b'.', b'.', _) => (DOTDOT, 2),
        (b'=', b'>', _) => (FATARROW, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b':', b':', _) => (COLONCOLON, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'(', ..) => (LPAREN, 1),
        (b')', ..) => (RPAREN, 1),
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
        (b'&', ..) => (AMP, 1),
        (b'|', ..) => (PIPE, 1),
        (b'^', ..) => (CARET, 1),
        (b'~', ..) => (TILDE, 1),
        (b'!', ..) => (BANG, 1),
        (b'<', ..) => (LT, 1),
        (b'>', ..) => (GT, 1),
        (b'@', ..) => (AT, 1),
        (b'$', ..) => (DOLLAR, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    c.bump_n(width);
    kind
}
