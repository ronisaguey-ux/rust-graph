//! CSS: rule sets, selectors, declarations, at-rules and a modern value set.
//!
//! Selectors are modelled structurally: type, class, id, universal, attribute,
//! pseudo and keyframe selectors, with the combinators between them, in a
//! selector list. `&` is a selector of its own, so a nested rule (CSS Nesting)
//! is an ordinary rule whose prelude starts with the parent reference. Values
//! are an operator table over numbers with units, colours, strings, functions
//! (`calc`, `var`, `rgb`, ...), `url()` as one token, and comma-separated
//! lists.
//!
//! A name directly before `:` is a declaration's property when the declaration
//! ends (`;` or `}`) before a block opens, which is what tells `color:red;`
//! from the selector `a:hover { }`; the lexer looks ahead for that brace, so a
//! compact `color:red` is a property just like `color: red`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Custom properties.** `--x: value` takes one `custom_property` token; the
//!   value is an ordinary value, so `--x: {a: b}` (a component value that is
//!   not an expression) is an error.
//! - **Grid template strings** (`"a a" "b c"`) are strings in a value list,
//!   not a grid template node.
//! - **A missing semicolon between declarations is accepted.** A declaration's
//!   `;` is optional here, so `color: #fff` on one line and `border: 1px` on
//!   the next parses as two declarations; CSS itself wants the semicolon.
//! - **`@media` prelude feature syntax** is a prelude item list, not a
//!   media-feature node; the parenthesis is not required.
//! - **`url(...)` is one token.** Its contents are not a subtree, exactly like
//!   a string: `url(#grad)`, `url(a b.png)` and an escaped URL all stay inside
//!   the token.

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
        LT = "<",
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
        BANG = "!",
        /// A whole `url(...)`, quoted or not.
        URL = "url",
    }

    stylesheet: _top*,

    _top: rule_set | at_rule | declaration | empty_declaration,

    rule_set: selector_list block,
    selector_list: (selector % ","),
    selector: _compound_selector _combinator*,
    _combinator: [(">" | "+" | "~")] _compound_selector,
    _compound_selector: parent_selector | simple_selector_run | keyframe_selector,
    parent_selector: "&" _simple_selector*,
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
    /// `0%`, `50%`, `100%` in a `@keyframes` block.
    keyframe_selector: PERCENT,
    attribute_selector: "[" IDENT [attribute_operator] [STRING | IDENT] "]",
    attribute_operator: "=" | "~=" | "|=" | "^=" | "$=" | "*=",
    pseudo_class_selector: ":" (IDENT | FUNCTION) [pseudo_arguments],
    pseudo_element_selector: "::" IDENT [pseudo_arguments],
    /// `:not(.a)`, `:is(.a, .b)`, `:nth-child(2n+1)`, `:lang(ar)`.
    pseudo_arguments: "(" [_pseudo_argument ("," _pseudo_argument)*] ")",
    _pseudo_argument: selector | nth_expression | NUMBER,
    /// The `an+b` micro-syntax: `2n`, `2n+1`, `-n+3`, `n`, `odd`, `even`.
    nth_expression:
        _number_sign IDENT [(PLUS | MINUS) NUMBER]
        | [_number_sign] DIMENSION [(PLUS | MINUS) (NUMBER | DIMENSION)]
        | [_number_sign] NUMBER IDENT [(PLUS | MINUS) NUMBER],
    _number_sign: PLUS | MINUS,

    block: "{" _block_item* "}",
    _block_item: declaration | rule_set | at_rule | empty_declaration,

    at_rule: AT_KEYWORD _at_prelude [at_body | ";"],
    _at_prelude: _at_prelude_item*,
    _at_prelude_item:
        IDENT
        | PROPERTY
        | CUSTOM_PROPERTY
        | DIMENSION
        | PERCENT
        | NUMBER
        | STRING
        | COLOR
        | URL
        | AT_KEYWORD
        | function_call
        | at_group
        | ":"
        | ","
        | ">"
        | "<"
        | "="
        | "/"
        | "*"
        | "+"
        | "."
        | "!",
    at_group: "(" _at_prelude ")",
    at_body: "{" _block_item* "}",

    declaration: property_name ":" value_list ["!important"] [";"],
    property_name: PROPERTY | CUSTOM_PROPERTY,
    empty_declaration: ";",

    value_list: (value %? ","),
    value: [("+" | "-")] value_atom (value_part)*,
    value_part: value_atom | value_operator,
    value_operator: "+" | "-" | "*" | "/" | "=" | ":",
    value_atom:
        NUMBER
        | DIMENSION
        | PERCENT
        | STRING
        | COLOR
        | IDENT
        | PROPERTY
        | CUSTOM_PROPERTY
        | URL
        | AT_KEYWORD
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
        b'!' => {
            c.bump();
            BANG
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
            // A hash that is a colour is one token; any other hash is the `#`
            // of an id selector and the name after it is its own identifier.
            let rest = &c.rest()[1..];
            let mut length = 0;
            while length < rest.len() && (rest[length].is_ascii_alphanumeric() || matches!(rest[length], b'-' | b'_')) {
                length += 1;
            }
            let text = &rest[..length];
            if (3..=8).contains(&text.len()) && text.iter().all(|byte| byte.is_ascii_hexdigit()) {
                c.bump_n(1 + length);
                COLOR
            } else {
                c.bump();
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
            // `-webkit-transform` is one identifier, unless it is a property.
            name_kind(c)
        }
        b'u' | b'U' if starts_url(c) => {
            url_token(c);
            URL
        }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            ident(c);
            name_kind(c)
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
        b'<' => {
            c.bump();
            LT
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

/// A name that has just been consumed: a function call if `(` follows, a
/// declaration's property if `:` does and the declaration ends before a block
/// opens (`color:red;` versus the selector `a:hover { }`), an identifier
/// otherwise.
fn name_kind(c: &Cursor) -> Kind {
    if c.peek() == b'(' {
        return FUNCTION;
    }
    let rest = c.rest();
    let mut index = 0;
    while index < rest.len() && matches!(rest[index], b' ' | b'\t' | b'\n' | b'\r') {
        index += 1;
    }
    if rest.get(index) == Some(&b':') && rest.get(index + 1) != Some(&b':') && declaration_ahead(&rest[index..]) {
        return PROPERTY;
    }
    IDENT
}

fn css_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80
}

/// A name: name bytes, plus backslash escapes so that `.text-align\:left` and
/// `.-left-\[25px\]` are one identifier each.
fn ident(c: &mut Cursor) {
    loop {
        if c.peek() == b'\\' && !matches!(c.peek_at(1), 0 | b'\n' | b'\r') {
            c.bump_n(2);
            continue;
        }
        if c.eat_while(css_name_byte) == 0 {
            return;
        }
    }
}

fn at_keyword(c: &mut Cursor) {
    c.eat_while(css_name_byte);
}

/// After `name:`, whether the declaration ends (`;` or `}`) before a block
/// opens (`{`): that is the difference between `color:red;` and `a:hover { }`,
/// which are otherwise the same `name ":" name` prefix. Strings and comments
/// are stepped over so their punctuation cannot decide it.
fn declaration_ahead(rest: &[u8]) -> bool {
    let mut index = 1;
    while index < rest.len() {
        match rest[index] {
            b';' | b'}' => return true,
            b'{' => return false,
            b'"' | b'\'' => {
                let quote = rest[index];
                index += 1;
                while index < rest.len() && rest[index] != quote {
                    if rest[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
            }
            b'/' if rest.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < rest.len() && !(rest[index] == b'*' && rest[index + 1] == b'/') {
                    index += 1;
                }
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    // No block opened: the end of the input closes the declaration.
    true
}

/// Whether the cursor is on `url(` — the one function whose contents are not
/// CSS tokens.
fn starts_url(c: &Cursor) -> bool {
    let rest = c.rest();
    rest.len() >= 4 && rest[..4].eq_ignore_ascii_case(b"url(")
}

/// A whole `url(...)`: up to the matching `)`, quotes and nested parentheses
/// included.
fn url_token(c: &mut Cursor) {
    c.bump_n(4);
    let mut depth = 1u32;
    let mut quote = 0u8;
    while !c.at_end() {
        let byte = c.peek();
        if quote != 0 {
            if byte == b'\\' {
                c.bump_n(2);
                continue;
            }
            if byte == quote {
                quote = 0;
            }
            c.bump_char();
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = byte,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                c.bump();
                if depth == 0 {
                    return;
                }
                continue;
            }
            _ => {}
        }
        c.bump_char();
    }
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::deep::tree::{Node, TOKEN};
    use crate::deep::{Budget, Workspace};

    struct Parsed {
        src: Vec<u8>,
        workspace: Workspace,
    }

    impl Parsed {
        fn fixture(name: &str) -> Parsed {
            let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/longtail/css").join(name);
            let src = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            let mut workspace = Workspace::default();
            workspace.parse(&src, &super::CSS, &Budget::default()).unwrap();
            Parsed { src, workspace }
        }

        fn node(&self, index: usize) -> Node {
            self.workspace.tree.nodes[index]
        }

        fn kind(&self, index: usize) -> &'static str {
            super::CSS.name(self.node(index).kind)
        }

        fn is_token(&self, index: usize) -> bool {
            self.node(index).flags & TOKEN != 0
        }

        fn text(&self, index: usize) -> &str {
            let node = self.node(index);
            std::str::from_utf8(&self.src[node.start as usize..node.end as usize]).expect("utf8 fixture")
        }

        fn of_kind(&self, kind: &str) -> Vec<usize> {
            (1..self.workspace.tree.nodes.len())
                .filter(|&index| !self.is_token(index) && self.kind(index) == kind)
                .collect()
        }

        fn leaves_of_kind(&self, kind: &str) -> Vec<usize> {
            (1..self.workspace.tree.nodes.len())
                .filter(|&index| self.is_token(index) && self.kind(index) == kind)
                .collect()
        }

        fn errors(&self) -> usize {
            use crate::deep::syntax::ERROR;
            use crate::deep::tree::HAS_ERROR;
            (0..self.workspace.tree.nodes.len())
                .filter(|&index| self.node(index).kind == ERROR || (self.is_token(index) && self.node(index).flags & HAS_ERROR != 0))
                .count()
        }
    }

    #[test]
    fn an_id_selector_is_a_hash_and_a_name() {
        let parsed = Parsed::fixture("css_nesting.css");
        assert_eq!(parsed.errors(), 0, "the fixture must parse cleanly");
        let ids = parsed.of_kind("id_selector");
        assert_eq!(ids.len(), 1, "#sidebar");
        let node = parsed.workspace.tree.nodes[ids[0]];
        assert_eq!(&parsed.src[node.start as usize..node.end as usize], b"#sidebar");
        // `#1a73e8` in a value stays one colour token, not a selector.
        assert!(parsed.leaves_of_kind("color").iter().any(|&index| parsed.text(index) == "#1a73e8"));
    }

    #[test]
    fn a_compact_declaration_is_a_declaration() {
        let parsed = Parsed::fixture("css_nesting.css");
        let declarations = parsed.of_kind("declaration");
        let compact = declarations
            .iter()
            .map(|&index| parsed.text(index))
            .find(|text| text.starts_with("width:"))
            .expect("the #sidebar rule's width");
        assert_eq!(compact, "width: 240px;");
    }

    #[test]
    fn nesting_keyframes_and_functions_are_structural() {
        let parsed = Parsed::fixture("css_nesting.css");
        assert_eq!(parsed.of_kind("parent_selector").len(), 3, "`& > .title`, `&:hover` and `&:focus-visible`");
        let frames: Vec<&str> = parsed.of_kind("keyframe_selector").iter().map(|&index| parsed.text(index)).collect();
        assert_eq!(frames, ["0%", "50%", "100%"]);
        assert!(parsed.leaves_of_kind("url").iter().any(|&index| parsed.text(index) == "url(../img/card.png)"));
        let nth = parsed.of_kind("nth_expression");
        assert_eq!(nth.len(), 1);
        let node = parsed.workspace.tree.nodes[nth[0]];
        assert_eq!(&parsed.src[node.start as usize..node.end as usize], b"2n + 1");
        assert!(!parsed.of_kind("at_rule").is_empty());
    }

    #[test]
    fn every_prefix_of_the_fixture_parses_or_errors_without_panicking() {
        let parsed = Parsed::fixture("css_nesting.css");
        for end in (0..parsed.src.len()).step_by(9).chain(std::iter::once(parsed.src.len())) {
            let mut workspace = Workspace::default();
            workspace.parse(&parsed.src[..end], &super::CSS, &Budget::default()).unwrap();
        }
    }
}
