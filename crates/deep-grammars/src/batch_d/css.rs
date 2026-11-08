//! CSS: rule sets, selectors, declarations, at-rules and a modern value set.
//!
//! Selectors are modelled structurally: type, class, id, universal, attribute
//! and pseudo selectors, with the combinators between them, in a selector list.
//! Values are an operator table over numbers with units, colours, strings,
//! functions (`calc`, `var`, `rgb`, ...), and comma-separated lists.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Nesting and `@layer` blocks** are modelled as an at-rule with a block of
//!   declarations or rules; a nested rule inside a declaration block parses
//!   only through `at_rule` or a plain rule whose prelude is a selector.
//! - **Custom properties.** `--x: value` takes one `property` token; the value
//!   is an ordinary value, so `--x: {a: b}` (a component value that is not an
//!   expression) is an error.
//! - **Grid template strings** (`"a a" "b c"`) are strings in a value list,
//!   not a grid template node.
//! - **A missing semicolon between declarations is accepted.** A declaration's
//!   `;` is optional here, so `color: #fff` on one line and `border: 1px` on the
//!   next parses as two declarations; CSS itself wants the semicolon.
//! - **`@media` prelude feature syntax** is a value list, not a media-feature
//!   node; the parenthesis is not required.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { CSS {
    name = "css";
    lex = lex_css;

    tokens {
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        COMMA = ",",
        COLON = ":",
        COLON2 = "::",
        AMP = "&",
        SEMI = ";",
        DOT = ".",
        HASH = "#",
        STAR = "*",
        GT = ">",
        PLUS = "+",
        TILDE = "~",
        EQ = "=",
        TILDE_EQ = "~=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        DOLLAR_EQ = "$=",
        STAR_EQ = "*=",
        SLASH = "/",
        MINUS = "-",
        PERCENT = "%",
        BANG_IMPORTANT = "!important",
        NUMBER = "number",
        DIMENSION = "dimension",
        STRING = "string",
        BAD_STRING = "bad_string" bad,
        /// `#fff`, `#1a2b3c`.
        COLOR = "color",
        PROPERTY = "property",
        /// `--custom-property`.
        CUSTOM_PROPERTY = "custom_property",
        FUNCTION = "function_name",
        IDENT = "identifier",
        AT_KEYWORD = "at_keyword",
    }

    stylesheet: _top*,

    _top: rule_set | at_rule | declaration,

    rule_set: selector_list block,
    selector_list: (selector % ","),
    selector: _compound_selector _combinator*,
    _combinator: [">" | "+" | "~"] _compound_selector,
    _compound_selector: simple_selector_run | "&",
    simple_selector_run: _simple_selector+,
    _simple_selector:
        type_selector
        | class_selector
        | id_selector
        | universal_selector
        | attribute_selector
        | pseudo_class_selector
        | pseudo_element_selector,
    type_selector: IDENT,
    class_selector: "." IDENT,
    id_selector: HASH IDENT,
    universal_selector: "*",
    attribute_selector: "[" IDENT [attribute_operator] [STRING | IDENT] "]",
    attribute_operator: "=" | "~=" | "|=" | "^=" | "$=" | "*=",
    pseudo_class_selector: ":" IDENT [pseudo_arguments],
    pseudo_element_selector: "::" IDENT,
    pseudo_arguments: "(" pseudo_argument ("," pseudo_argument)* ")",
    pseudo_argument: selector | NUMBER,

    block: "{" _block_item* "}",
    _block_item: declaration | rule_set | at_rule,

    at_rule: AT_KEYWORD _at_prelude [at_body | ";"],
    _at_prelude: _at_prelude_item*,
    _at_prelude_item: IDENT | PROPERTY | CUSTOM_PROPERTY | DIMENSION | NUMBER | STRING | COLOR | ":" | at_group | "," | ">" | "/" | "*" | "=",
    at_group: "(" _at_prelude ")",
    at_body: "{" _block_item* "}",

    declaration: property_name ":" value_list ["!important"] [";"],
    property_name: PROPERTY | CUSTOM_PROPERTY,

    value_list: (value % ","),
    value: value_atom (value_part)*,
    value_part: value_atom | value_operator,
    value_operator: "+" | "-" | "*" | "/",
    value_atom:
        NUMBER
        | DIMENSION
        | PERCENT
        | STRING
        | COLOR
        | IDENT
        | CUSTOM_PROPERTY
        | function_call
        | paren_value
        | "#" IDENT,

    paren_value: "(" value_list ")",
    function_call: FUNCTION function_arguments,
    function_arguments: "(" [value_list] ")",
} }

pub fn lex_css(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'*' => {
            lex::block_comment(c, b"/*", b"*/", false);
            COMMENT
        }
        b'!' if c.rest().len() >= 10 && c.rest()[..10].eq_ignore_ascii_case(b"!important") => {
            c.bump_n(10);
            BANG_IMPORTANT
        }
        b'"' | b'\'' => {
            if lex::quoted(c, byte) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'@' => {
            c.bump();
            at_keyword(c);
            AT_KEYWORD
        }
        b'#' => {
            c.bump();
            let start = c.rest();
            let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
            let text = &start[..length];
            if text.is_empty() {
                HASH
            } else if text.len() >= 3 && text.len() <= 8 && text.iter().all(|byte| byte.is_ascii_hexdigit()) {
                COLOR
            } else {
                HASH
            }
        }
        b'0'..=b'9' => number_or_dimension(c, true),
        b'.' if c.peek_at(1).is_ascii_digit() => {
            c.bump();
            number_or_dimension(c, false)
        }
        b'-' if c.peek_at(1) == b'-' => {
            c.bump_n(2);
            c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
            CUSTOM_PROPERTY
        }
        b'-' if c.peek_at(1).is_ascii_alphabetic() || c.peek_at(1) == b'-' => {
            c.bump();
            ident(c);
            // `-webkit-transform` is one identifier.
            IDENT
        }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(css_name_byte);
            let text = &start[..length];
            if c.peek() == b'(' {
                return FUNCTION;
            }
            let _ = text;
            // A name directly followed by `: ` is a declaration's property;
            // `a:hover` (no space) is a selector, so the space is the signal.
            if c.peek() == b':' && matches!(c.peek_at(1), b' ' | b'\t' | b'\n' | b'\r') {
                return PROPERTY;
            }
            IDENT
        }
        b'(' => {
            c.bump();
            LPAREN
        }
        b')' => {
            c.bump();
            RPAREN
        }
        b'{' => {
            c.bump();
            LBRACE
        }
        b'}' => {
            c.bump();
            RBRACE
        }
        b'[' => {
            c.bump();
            LBRACKET
        }
        b']' => {
            c.bump();
            RBRACKET
        }
        b',' => {
            c.bump();
            COMMA
        }
        b':' if c.peek_at(1) == b':' => {
            c.bump_n(2);
            COLON2
        }
        b'&' => {
            c.bump();
            AMP
        }
        b':' => {
            c.bump();
            COLON
        }
        b';' => {
            c.bump();
            SEMI
        }
        b'.' => {
            c.bump();
            DOT
        }
        b'*' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            STAR_EQ
        }
        b'*' => {
            c.bump();
            STAR
        }
        b'>' => {
            c.bump();
            GT
        }
        b'+' => {
            c.bump();
            PLUS
        }
        b'~' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            TILDE_EQ
        }
        b'~' => {
            c.bump();
            TILDE
        }
        b'=' => {
            c.bump();
            EQ
        }
        b'|' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            PIPE_EQ
        }
        b'^' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            CARET_EQ
        }
        b'$' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            DOLLAR_EQ
        }
        b'/' => {
            c.bump();
            SLASH
        }
        b'-' => {
            c.bump();
            MINUS
        }
        b'%' => {
            c.bump();
            PERCENT
        }
        _ => UNKNOWN,
    }
}

fn css_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80
}

fn ident(c: &mut Cursor) {
    c.eat_while(css_name_byte);
}

fn at_keyword(c: &mut Cursor) {
    c.eat_while(css_name_byte);
}

fn number_or_dimension(c: &mut Cursor, started: bool) -> Kind {
    if started {
        c.eat_while(|byte| byte.is_ascii_digit());
    }
    if c.peek() == b'.' && c.peek_at(1).is_ascii_digit() {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit());
    }
    if matches!(c.peek(), b'e' | b'E') {
        let sign = usize::from(matches!(c.peek_at(1), b'+' | b'-'));
        if c.peek_at(1 + sign).is_ascii_digit() {
            c.bump_n(1 + sign);
            c.eat_while(|byte| byte.is_ascii_digit());
        }
    }
    if c.peek() == b'%' {
        c.bump();
        return PERCENT;
    }
    if c.peek().is_ascii_alphabetic() {
        ident(c);
        return DIMENSION;
    }
    NUMBER
}
