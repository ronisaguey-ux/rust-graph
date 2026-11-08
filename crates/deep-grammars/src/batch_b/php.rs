//! PHP through `grammar!`: one of Field 6's four languages (Ruby, PHP, Swift,
//! Scala).
//!
//! PHP is brace- and semicolon-delimited, so no layout rule is needed and every
//! statement ends at its `;`. Node names follow Tree-sitter's PHP where that
//! costs nothing; the tree is not the same tree.
//!
//! What the DSL covers here:
//!
//! - **The opening tag is trivia.** `<?php` and `?>` are lexed as trivia, so
//!   the root is the file's statements and nothing has to consume the tag.
//! - **Variables are one token.** `$name` is `VARIABLE`, which is what makes
//!   `$x = 1`, `$this->y` and `$a[$i]` ordinary expression forms.
//! - **A member and a static member are postfix rows.** `$this->name(...)`,
//!   `Foo::bar`, `Foo::CONST` and `$obj?->name` are `member_access`,
//!   `scoped_call` and `nullsafe_member_access`, taken after their operands.
//! - **Strings are single tokens.** A double-quoted string's `{$...}` and
//!   `$name` interpolations, a heredoc's whole body and a nowdoc's whole body
//!   are scanned as one `string`; the code inside a string is not parsed.
//! - **Types are shallow.** A parameter or return type is `?` plus a qualified
//!   name plus `|` alternatives (`?int`, `string|int`, `\App\Model`). Generic
//!   phpdoc types are comments, not syntax.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **A heredoc or nowdoc is a lexer state.** `<<<EOT` opens a body that ends
//!   at a line holding `EOT` (optionally indented, optionally followed by `;`).
//!   The DSL has no external scanner, so the step lexer scans it whole and
//!   returns one `string` token; the terminator line is part of the token.
//! - **`->` is a token, not an operator.** There is no row for it: a member
//!   access is `postfix "->" _member_name`, so `$a->b->c` is two postfix
//!   takes of the same row, which is what keeps the table LL(3).
//! - **Alternative syntax is not modelled.** `if (...): ... endif;`,
//!   `foreach (...): ... endforeach;` and friends are valid PHP but are not
//!   rules here; the fixtures use braces. The colon form would need a second
//!   statement grammar per construct for no tree-shape gain.
//! - **Attributes (`#[...]`) collide with `#` comments.** A `#` starts a
//!   comment in this lexer; PHP 8 attributes are therefore not lexed, which is
//!   the one place a valid file can lose tokens it should keep.
//! - **A decision sees three tokens.** `foreach ($xs as $k => $v)` and a named
//!   argument `f(name: 1)` share their first token with an expression, so they
//!   are entered through `&(...)` predicates.
//! - **One kind per rule**, so `member_access` and `nullsafe_member_access`
//!   are two rules rather than one labelled alternative.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { PHP {
    name = "php";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "name",
        VARIABLE = "variable_name",
        NUMBER = "integer",
        FLOAT = "float",
        STRING = "string",
        /// A string literal or heredoc with no end.
        BAD_LITERAL = "bad_literal" bad,
        /// `<?php` and `<?=`: the opening tag is not syntax here.
        OPEN_TAG = "<?php" trivia,
        CLOSE_TAG = "?>" trivia,

        KW_ABSTRACT = "abstract",
        KW_AND = "and",
        KW_ARRAY = "array",
        KW_AS = "as",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CLASS = "class",
        KW_CLONE = "clone",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DEFAULT = "default",
        KW_DO = "do",
        KW_ECHO = "echo",
        KW_ELSE = "else",
        KW_ELSEIF = "elseif",
        KW_EMPTY = "empty",
        KW_ENUM = "enum",
        KW_EXTENDS = "extends",
        KW_FALSE = "false",
        KW_FINAL = "final",
        KW_FINALLY = "finally",
        KW_FN = "fn",
        KW_FOR = "for",
        KW_FOREACH = "foreach",
        KW_FUNCTION = "function",
        KW_GLOBAL = "global",
        KW_IF = "if",
        KW_IMPLEMENTS = "implements",
        KW_INCLUDE = "include",
        KW_INCLUDE_ONCE = "include_once",
        KW_INSTANCEOF = "instanceof",
        KW_INTERFACE = "interface",
        KW_ISSET = "isset",
        KW_LIST = "list",
        KW_MATCH = "match",
        KW_NAMESPACE = "namespace",
        KW_NEW = "new",
        KW_NULL = "null",
        KW_OR = "or",
        KW_PARENT = "parent",
        KW_PRINT = "print",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_PUBLIC = "public",
        KW_READONLY = "readonly",
        KW_REQUIRE = "require",
        KW_REQUIRE_ONCE = "require_once",
        KW_RETURN = "return",
        KW_SELF = "self",
        KW_STATIC = "static",
        KW_SWITCH = "switch",
        KW_THROW = "throw",
        KW_TRAIT = "trait",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_UNSET = "unset",
        KW_USE = "use",
        KW_VAR = "var",
        KW_WHILE = "while",
        KW_XOR = "xor",
        KW_YIELD = "yield",

        SEMI = ";",
        COMMA = ",",
        COLON = ":",
        DOT = ".",
        ELLIPSIS = "...",
        ARROW = "->",
        NULLSAFE = "?->",
        COLONCOLON = "::",
        FATARROW = "=>",
        BACKSLASH = "\\",
        QUESTION = "?",
        AT = "@",
        ASSIGN = "=",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        DOT_EQ = ".=",
        PERCENT_EQ = "%=",
        STARSTAR_EQ = "**=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        NULLISH_EQ = "??=",
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
        SPACESHIP = "<=>",
        EQ = "==",
        IDENTICAL = "===",
        NE = "!=",
        ANGLE_NE = "<>",
        NOT_IDENTICAL = "!==",
        ANDAND = "&&",
        OROR = "||",
        NULLISH = "??",
        SHL = "<<",
        SHR = ">>",
        INCREMENT = "++",
        DECREMENT = "--",
    }

    program: _statement*,

    _statement:
        &([top_modifiers] "class") class_declaration
        | &([top_modifiers] "interface") interface_declaration
        | &([top_modifiers] "trait") trait_declaration
        | &([top_modifiers] "enum") enum_declaration
        | namespace_declaration
        | use_declaration
        | require_statement
        | function_declaration
        | if_statement
        | while_statement
        | do_statement
        | for_statement
        | foreach_statement
        | switch_statement
        | try_statement
        | return_statement
        | break_statement
        | continue_statement
        | echo_statement
        | global_statement
        | throw_statement
        | unset_statement
        | block
        | ";"
        | expression_statement,

    // ---- declarations ----

    namespace_declaration: "namespace" _qualified_name (";" | namespace_block),
    namespace_block: "{" _statement* "}",
    use_declaration: "use" [("function" | "const")] (_use_item % ",") ";",
    _use_item: _qualified_name ["as" _name],
    require_statement: ("require" | "require_once" | "include" | "include_once") _expression ";",

    modifiers: _modifier+,
    _modifier: "public" | "protected" | "private" | "static" | "abstract" | "final" | "readonly" | "var",
    /// What a top-level declaration may have: never `static`, which at the top
    /// level starts a `static::` expression instead.
    top_modifiers: ("abstract" | "final" | "readonly")+,

    class_declaration: [top_modifiers] "class" _name ["extends" _qualified_name] ["implements" _qualified_name ("," _qualified_name)*] class_body,
    interface_declaration: [top_modifiers] "interface" _name ["extends" _qualified_name ("," _qualified_name)*] class_body,
    trait_declaration: [top_modifiers] "trait" _name class_body,
    enum_declaration: [top_modifiers] "enum" _name [":" _type] ["implements" _qualified_name ("," _qualified_name)*] enum_body,
    enum_body: "{" enum_case* "}",
    enum_case: "case" _name ["=" _expression] ";",
    class_body: "{" _member* "}",
    _member:
        &([modifiers] "function") method_declaration
        | &([modifiers] "const") const_declaration
        | &("use" _qualified_name) trait_use_declaration
        | &([modifiers] _type VARIABLE) property_declaration
        | ";",

    method_declaration: [modifiers] "function" ["&"] _name formal_parameters [":" _type] (block | ";"),
    function_declaration: "function" ["&"] _name formal_parameters [":" _type] (block | ";"),
    formal_parameters: "(" (formal_parameter %? ",") ")",
    formal_parameter: [modifiers] [_type] ["&"] ["..."] VARIABLE ["=" _expression],
    property_declaration: [modifiers] _type (_property_item % ",") ";",
    _property_item: VARIABLE ["=" _expression],
    const_declaration: [modifiers] "const" _name "=" _expression ";",
    trait_use_declaration: "use" _qualified_name ("," _qualified_name)* ";",

    // ---- statements ----

    block: "{" _statement* "}",
    expression_statement: _expression ";",
    if_statement: "if" parenthesized_expression _statement elseif_clause* [else_clause],
    elseif_clause: "elseif" parenthesized_expression _statement,
    else_clause: "else" _statement,
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression ";",
    for_statement: "for" for_header _statement,
    for_header: "(" [_expression_list] ";" [_expression_list] ";" [_expression_list] ")",
    _expression_list: _expression ("," _expression)*,
    foreach_statement: "foreach" foreach_header _statement,
    foreach_header: "(" _expression "as" _foreach_target ")",
    _foreach_target: &(_expression "=>") key_value_target | _expression,
    key_value_target: _expression "=>" _expression,
    switch_statement: "switch" parenthesized_expression switch_body,
    switch_body: "{" (case_clause | default_clause)* "}",
    case_clause: "case" _expression (":" | ";") _statement*,
    default_clause: "default" (":" | ";") _statement*,
    try_statement: "try" block catch_clause* [finally_clause],
    catch_clause: "catch" catch_header block,
    catch_header: "(" _catch_types VARIABLE ")",
    _catch_types: _qualified_name ("|" _qualified_name)*,
    finally_clause: "finally" block,
    return_statement: "return" [_expression] ";",
    break_statement: "break" [NUMBER] ";",
    continue_statement: "continue" [NUMBER] ";",
    echo_statement: "echo" _expression ("," _expression)* ";",
    global_statement: "global" (VARIABLE % ",") ";",
    throw_statement: "throw" _expression ";",
    unset_statement: "unset" unset_arguments ";",
    unset_arguments: "(" (_expression %? ",") ")",
    parenthesized_expression: "(" _expression ")",

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | ".=" | "%=" | "**=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "??=" => assignment_expression,
        word_or: left "or" => binary_expression,
        word_xor: left "xor" => binary_expression,
        word_and: left "and" => binary_expression,
        cond: postfix "?" _expression ":" _expression@assign => conditional_expression,
        nullish: left "??" => binary_expression,
        or: left "||" => binary_expression,
        and: left "&&" => binary_expression,
        bor: left "|" => binary_expression,
        xor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        eq: left "==" | "!=" | "===" | "!==" | "<>" | "<=>" => binary_expression,
        cmp: left "<" | ">" | "<=" | ">=" => binary_expression,
        instance: left "instanceof" => instance_of_expression,
        shift: left "<<" | ">>" => binary_expression,
        add: left "+" | "-" | "." => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        exp: right "**" => binary_expression,
        clone: prefix "clone" => clone_expression,
        print: prefix "print" => print_expression,
        unary: prefix "!" | "~" | "-" | "+" | "@" => unary_expression,
        update: prefix "++" | "--" => update_expression,
        postfix "++" | "--" => update_expression,
        call: postfix arguments => function_call_expression,
        index: postfix subscript => subscript_expression,
        member: postfix "->" _member_name => member_access_expression,
        nullsafe: postfix "?->" _member_name => nullsafe_member_access_expression,
        scope: postfix "::" (_member_name | VARIABLE) => scoped_property_access_expression,
    },

    _primary:
        VARIABLE
        | NUMBER
        | FLOAT
        | STRING
        | "true"
        | "false"
        | "null"
        | "self"
        | "parent"
        | "static"
        | parenthesized_expression
        | array_literal
        | array_expression
        | new_expression
        | isset_expression
        | empty_expression
        | list_expression
        | _qualified_name,

    _qualified_name: [BACKSLASH] _name (BACKSLASH _name)*,
    new_expression: "new" _new_class [arguments],
    _new_class: "self" | "parent" | "static" | _qualified_name,
    _name: IDENT,
    /// After `->` and `::` every word is a member name.
    _member_name: IDENT | "class",
    arguments: "(" (_argument %? ",") ")",
    _argument: &(_name ":") named_argument | spread_element | _expression,
    named_argument: _name ":" _expression,
    spread_element: "..." _expression,
    subscript: "[" [_expression] "]",

    array_literal: "[" (_array_element %? ",") "]",
    array_expression: "array" array_arguments,
    array_arguments: "(" (_array_element %? ",") ")",
    _array_element: &(_expression "=>") pair_element | spread_element | _expression,
    pair_element: _expression "=>" _expression,

    isset_expression: "isset" isset_arguments,
    isset_arguments: "(" (_expression % ",") ")",
    empty_expression: "empty" empty_arguments,
    empty_arguments: "(" _expression ")",
    list_expression: "list" list_arguments,
    list_arguments: "(" (VARIABLE %? ",") ")",

    // ---- types ----

    _type: ["?"] _type_name ("|" _type_name)*,
    _type_name: [BACKSLASH] (_name | "array" | "self" | "parent" | "static") (BACKSLASH _name)*,
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
        b'<' if c.starts_with(b"<?php") => {
            c.bump_n(5);
            OPEN_TAG
        }
        b'<' if c.starts_with(b"<?=") => {
            c.bump_n(3);
            OPEN_TAG
        }
        b'?' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            CLOSE_TAG
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        b'#' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            block_comment(c);
            COMMENT
        }
        b'\'' | b'"' | b'`' => string(c),
        b'<' if c.starts_with(b"<<<") => heredoc(c),
        b'$' if is_word_start(c.peek_at(1)) => {
            c.bump();
            c.eat_while(is_word_byte);
            VARIABLE
        }
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() => number(c),
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
        b"abstract" => KW_ABSTRACT,
        b"and" => KW_AND,
        b"array" => KW_ARRAY,
        b"as" => KW_AS,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"catch" => KW_CATCH,
        b"class" => KW_CLASS,
        b"clone" => KW_CLONE,
        b"const" => KW_CONST,
        b"continue" => KW_CONTINUE,
        b"default" => KW_DEFAULT,
        b"do" => KW_DO,
        b"echo" => KW_ECHO,
        b"else" => KW_ELSE,
        b"elseif" => KW_ELSEIF,
        b"empty" => KW_EMPTY,
        b"enum" => KW_ENUM,
        b"extends" => KW_EXTENDS,
        b"false" => KW_FALSE,
        b"final" => KW_FINAL,
        b"finally" => KW_FINALLY,
        b"fn" => KW_FN,
        b"for" => KW_FOR,
        b"foreach" => KW_FOREACH,
        b"function" => KW_FUNCTION,
        b"global" => KW_GLOBAL,
        b"if" => KW_IF,
        b"implements" => KW_IMPLEMENTS,
        b"include" => KW_INCLUDE,
        b"include_once" => KW_INCLUDE_ONCE,
        b"instanceof" => KW_INSTANCEOF,
        b"interface" => KW_INTERFACE,
        b"isset" => KW_ISSET,
        b"list" => KW_LIST,
        b"match" => KW_MATCH,
        b"namespace" => KW_NAMESPACE,
        b"new" => KW_NEW,
        b"null" => KW_NULL,
        b"or" => KW_OR,
        b"parent" => KW_PARENT,
        b"print" => KW_PRINT,
        b"private" => KW_PRIVATE,
        b"protected" => KW_PROTECTED,
        b"public" => KW_PUBLIC,
        b"readonly" => KW_READONLY,
        b"require" => KW_REQUIRE,
        b"require_once" => KW_REQUIRE_ONCE,
        b"return" => KW_RETURN,
        b"self" => KW_SELF,
        b"static" => KW_STATIC,
        b"switch" => KW_SWITCH,
        b"throw" => KW_THROW,
        b"trait" => KW_TRAIT,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"unset" => KW_UNSET,
        b"use" => KW_USE,
        b"var" => KW_VAR,
        b"while" => KW_WHILE,
        b"xor" => KW_XOR,
        b"yield" => KW_YIELD,
        _ => IDENT,
    }
}

/// `/* ... */`. An unclosed one runs to the end of the file.
fn block_comment(c: &mut Cursor) {
    c.bump_n(2);
    while !c.at_end() && !c.starts_with(b"*/") {
        c.bump();
    }
    c.bump_n(2);
}

/// A single- or double-quoted string, or a backtick shell string, from its
/// quote. The interpolations of a double-quoted one stay opaque.
fn string(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    c.bump();
    loop {
        match c.peek() {
            0 if c.at_end() => return BAD_LITERAL,
            b'\\' => c.bump_n(2),
            b'\n' | b'\r' if quote != b'"' && quote != b'`' => return BAD_LITERAL,
            b'{' if quote == b'"' && c.peek_at(1) == b'$' => {
                c.bump_n(2);
                braces(c);
            }
            byte if byte == quote => {
                c.bump();
                return STRING;
            }
            _ => c.bump(),
        }
    }
}

/// The code of a `{$...}` interpolation, up to its matching `}`.
fn braces(c: &mut Cursor) {
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
                    return;
                }
            }
            b'"' | b'\'' => {
                string(c);
            }
            _ => c.bump(),
        }
    }
}

/// A heredoc or nowdoc from `<<<` to the line holding its terminator. The
/// terminator may be indented (PHP 7.3), and the `;` that ends the statement
/// stays outside the token: scanning stops at the marker's last byte.
fn heredoc(c: &mut Cursor) -> Kind {
    let rest = c.rest();
    let mut at = 3;
    while at < rest.len() && matches!(rest[at], b' ' | b'\t') {
        at += 1;
    }
    let quote = rest.get(at).copied().unwrap_or(0);
    let quoted = matches!(quote, b'\'' | b'"');
    if quoted {
        at += 1;
    }
    let mark_start = at;
    while at < rest.len() && is_word_byte(rest[at]) {
        at += 1;
    }
    let marker = &rest[mark_start..at];
    if marker.is_empty() {
        c.bump_n(3);
        return BAD_LITERAL;
    }
    let marker_len = marker.len();
    if quoted && rest.get(at) == Some(&quote) {
        at += 1;
    }
    while at < rest.len() && rest[at] != b'\n' {
        at += 1;
    }
    if at >= rest.len() {
        c.bump_n(rest.len());
        return BAD_LITERAL;
    }
    at += 1;
    loop {
        if at >= rest.len() {
            c.bump_n(rest.len());
            return BAD_LITERAL;
        }
        let line_start = at;
        let mut line_end = at;
        while line_end < rest.len() && rest[line_end] != b'\n' {
            line_end += 1;
        }
        let line = &rest[line_start..line_end];
        let lead = line.iter().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
        let body = &line[lead..];
        let body = body.strip_suffix(b";").unwrap_or(body);
        if trim_end(body) == marker {
            c.bump_n(line_start + lead + marker_len);
            return STRING;
        }
        if line_end >= rest.len() {
            c.bump_n(rest.len());
            return BAD_LITERAL;
        }
        at = line_end + 1;
    }
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = bytes {
        if matches!(first, b' ' | b'\t') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while let [rest @ .., last] = bytes {
        if matches!(last, b' ' | b'\t' | b'\r') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn number(c: &mut Cursor) -> Kind {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
        let radix = c.peek_at(1).to_ascii_lowercase();
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        let _ = radix;
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

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'*', b'*', b'=') => (STARSTAR_EQ, 3),
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'=', b'=', b'=') => (IDENTICAL, 3),
        (b'!', b'=', b'=') => (NOT_IDENTICAL, 3),
        (b'<', b'=', b'>') => (SPACESHIP, 3),
        (b'?', b'-', b'>') => (NULLSAFE, 3),
        (b'?', b'?', b'=') => (NULLISH_EQ, 3),
        (b'>', b'>', _) => (SHR, 2),
        (b'<', b'<', _) => (SHL, 2),
        (b'*', b'*', _) => (STARSTAR, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b'=', b'>', _) => (FATARROW, 2),
        (b':', b':', _) => (COLONCOLON, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'<', b'>', _) => (ANGLE_NE, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'?', b'?', _) => (NULLISH, 2),
        (b'+', b'+', _) => (INCREMENT, 2),
        (b'-', b'-', _) => (DECREMENT, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'.', b'=', _) => (DOT_EQ, 2),
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
        (b':', ..) => (COLON, 1),
        (b'.', ..) => (DOT, 1),
        (b'\\', ..) => (BACKSLASH, 1),
        (b'?', ..) => (QUESTION, 1),
        (b'@', ..) => (AT, 1),
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
