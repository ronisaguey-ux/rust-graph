//! HTML: elements, attributes, text, comments, the doctype and void elements.
//!
//! The lexer decides two things a context-free rule cannot: a name directly
//! after `<` that is a void element (`br`, `img`, `meta`, ...) gets its own
//! token, so `<br>` need not wait for a `</br>`; and a run of characters that
//! does not start with `<` is one `text` token, whitespace included, which is
//! what makes mixed content parse without a separate text mode.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Raw-text elements.** `script` and `style` bodies may contain `<`, which
//!   the text scanner stops at, so a script with a less-than in it is an error.
//!   Every tag in the body still parses; the body is not re-lexed as raw text.
//! - **Optional end tags.** `<p>` closed implicitly by the next `<p>`, `<li>`,
//!   `<td>` and friends are not implied: the grammar wants the closing tag.
//!   HTML's tag-omission rules are not structural in this grammar.
//! - **Entities stay text.** `&amp;` is part of the text token, not decoded and
//!   not a node.
//! - **Mismatched tags are errors inside the element**, never silently paired.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { HTML {
    name = "html";
    lex = lex_html;

    tokens {
        LT = "<",
        GT = ">",
        CLOSE_START = "</",
        SLASH_GT = "/>",
        EQ = "=",
        NAME = "tag_name",
        /// A void element's name, straight after `<`.
        VOID_NAME = "void_tag_name",
        STRING = "attribute_value",
        BAD_STRING = "bad_string" bad,
        TEXT = "text",
        COMMENT = "comment",
        DOCTYPE = "doctype",
    }

    document: _node*,

    _node: element | void_element | text_node | comment_node | doctype_node,

    element: "<" tag _attribute* (empty_tag | start_tag),
    empty_tag: "/>",
    start_tag: ">" _node* closing_tag,
    closing_tag: "</" tag ">",

    void_element: "<" VOID_NAME _attribute* [">" | "/>"],

    _attribute: tag ["=" attribute_value],
    attribute_value: STRING | tag,

    tag: NAME,
    text_node: TEXT,
    comment_node: COMMENT,
    doctype_node: DOCTYPE,
} }

/// Names that never take a closing tag, in the HTML standard's spelling.
const VOID: &[&[u8]] = &[
    b"area", b"base", b"br", b"col", b"embed", b"hr", b"img", b"input", b"link", b"meta", b"param", b"source", b"track", b"wbr",
];

pub fn lex_html(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT as TRIVIA_COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'<' if c.starts_with(b"<!--") => {
            c.bump_n(4);
            while !c.at_end() {
                if c.starts_with(b"-->") {
                    c.bump_n(3);
                    return COMMENT;
                }
                c.bump_char();
            }
            COMMENT
        }
        b'<' if c.starts_with(b"<!") => {
            c.bump_n(2);
            while !c.at_end() && c.peek() != b'>' {
                c.bump_char();
            }
            c.eat(b'>');
            DOCTYPE
        }
        b'<' if c.starts_with(b"<?") => {
            c.bump_n(2);
            while !c.at_end() {
                if c.starts_with(b"?>") {
                    c.bump_n(2);
                    return COMMENT;
                }
                c.bump_char();
            }
            TRIVIA_COMMENT
        }
        b'<' if c.starts_with(b"</") => {
            c.bump_n(2);
            CLOSE_START
        }
        b'<' => {
            c.bump();
            LT
        }
        b'>' => {
            c.bump();
            GT
        }
        b'/' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            SLASH_GT
        }
        b'=' => {
            c.bump();
            EQ
        }
        b'"' | b'\'' => {
            if lex::quoted(c, byte) {
                STRING
            } else {
                BAD_STRING
            }
        }
        _ => {
            // A name is a tag name or an attribute name only where one can
            // stand: after `<`, `</`, another name, an attribute value or `=`.
            // Anywhere else a letter starts text, which is what makes a body
            // such as `<title>Scanner dashboard</title>` parse.
            let previous = c.previous();
            let in_tag = matches!(previous, crate::deep::syntax::UNKNOWN | LT | CLOSE_START | NAME | VOID_NAME | STRING | EQ);
            if in_tag && matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'_') {
                let start = c.rest();
                let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'));
                let text = &start[..length];
                if previous == LT && VOID.iter().any(|void| void.eq_ignore_ascii_case(text)) {
                    return VOID_NAME;
                }
                return NAME;
            }
            if byte == 0 {
                return UNKNOWN;
            }
            // Text runs to the next `<` and keeps its whitespace.
            let length = c.eat_while(|byte| byte != b'<');
            if length == 0 {
                c.bump_char();
                return UNKNOWN;
            }
            TEXT
        }
    }
}
