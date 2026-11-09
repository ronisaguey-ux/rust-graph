//! HTML: elements, attributes, text, comments, the doctype and void elements.
//!
//! The lexer decides what a context-free rule cannot:
//!
//! - a name directly after `<` that is a void element (`br`, `img`, `meta`,
//!   ...) gets its own token, so `<br>` need not wait for a `</br>`;
//! - a run of characters that does not start with `<` is one `text` token,
//!   whitespace included, which is what makes mixed content parse without a
//!   separate text mode;
//! - `<script>` and `<style>` switch the lexer into a raw-text mode, so a `<`
//!   or a `</div>` inside a script is text, not markup. The closing `</` pops
//!   the mode and the name right after it is lexed as a tag name again;
//! - the elements whose end tag HTML lets the author omit (`p`, `li`, `dt`,
//!   `dd`, `td`, `th`, `tr`, `option`, `optgroup`, `thead`, `tbody`, `tfoot`,
//!   `colgroup`) get their own open and close tokens, so `</div>` closes a
//!   `p` that was never closed while `</p>` still closes the `p` and nothing
//!   else. A closing tag of one of those names is one token;
//! - an attribute value without quotes is one `attribute_value` token.
//!
//! A mismatched tag is still an error inside the element it breaks: an
//! unclosed `<span>` costs the span's subtree, never the file.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Optional *start* tags** (a `tbody` implied by `<tr>`) and the implicit
//!   close of one optional element by the *start* of another (`<li>a<li>b`)
//!   are not implied: the second `li` nests inside the first. The end tags
//!   themselves are optional, so the file still parses.
//! - **Entities stay text.** `&amp;` is part of the text token, not decoded and
//!   not a node.
//! - **`<textarea>` and `<title>` are not raw text.** Only `script` and `style`
//!   switch the lexer's mode; a `<` inside a `<title>` is markup.

use std::cell::RefCell;

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { HTML {
    name = "html";
    lex = lex_html;

    /// After `<script>` or `<style>` the body is text until its own closing
    /// tag; `RAW_GT` pushes the mode and `RAW_END` pops it.
    modes {
        RAW = lex_raw,
    }
    switch RAW_GT => push RAW;
    switch RAW_END => pop;

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
        /// An attribute value with no quotes: `width=100`, `href=/a/b`.
        UNQUOTED = "unquoted_attribute_value",
        BAD_STRING = "bad_string" bad,
        TEXT = "text",
        COMMENT = "comment",
        DOCTYPE = "doctype",
        /// The `>` that opens a `script` or `style` body.
        RAW_GT = "raw_start",
        /// The body of a `script` or `style`, up to its closing tag.
        RAW_BODY = "raw_text",
        /// The `</` that ends a `script` or `style` body.
        RAW_END = "raw_end_tag",

        P_NAME = "p_tag_name",
        P_CLOSE = "</p>",
        LI_NAME = "li_tag_name",
        LI_CLOSE = "</li>",
        DT_NAME = "dt_tag_name",
        DT_CLOSE = "</dt>",
        DD_NAME = "dd_tag_name",
        DD_CLOSE = "</dd>",
        TD_NAME = "td_tag_name",
        TD_CLOSE = "</td>",
        TH_NAME = "th_tag_name",
        TH_CLOSE = "</th>",
        TR_NAME = "tr_tag_name",
        TR_CLOSE = "</tr>",
        OPTION_NAME = "option_tag_name",
        OPTION_CLOSE = "</option>",
        OPTGROUP_NAME = "optgroup_tag_name",
        OPTGROUP_CLOSE = "</optgroup>",
        THEAD_NAME = "thead_tag_name",
        THEAD_CLOSE = "</thead>",
        TBODY_NAME = "tbody_tag_name",
        TBODY_CLOSE = "</tbody>",
        TFOOT_NAME = "tfoot_tag_name",
        TFOOT_CLOSE = "</tfoot>",
        COLGROUP_NAME = "colgroup_tag_name",
        COLGROUP_CLOSE = "</colgroup>",
    }

    document: _node*,

    _node:
        element
        | void_element
        | optional_element
        | text_node
        | comment_node
        | doctype_node,

    element: "<" tag _attribute* (empty_tag | start_tag | raw_element_body),
    empty_tag: "/>",
    start_tag: ">" _node* closing_tag,
    closing_tag: ("</" | RAW_END) tag ">",

    /// `<script>` and `<style>`: the body is raw text up to the closing tag,
    /// so a `<` inside the script is not markup.
    raw_element_body: RAW_GT raw_text* closing_tag,
    raw_text: RAW_BODY,

    void_element: "<" VOID_NAME _attribute* [">" | "/>"],

    _attribute: tag ["=" attribute_value],
    attribute_value: STRING | UNQUOTED | tag,

    /// The thirteen elements whose end tag may be left out. `</div>` then ends
    /// a `p` that has no closer, while `</p>` is the p token itself.
    optional_element:
        p_element
        | li_element
        | dt_element
        | dd_element
        | td_element
        | th_element
        | tr_element
        | option_element
        | optgroup_element
        | thead_element
        | tbody_element
        | tfoot_element
        | colgroup_element,

    p_element: "<" P_NAME _attribute* ">" _node* [P_CLOSE],
    li_element: "<" LI_NAME _attribute* ">" _node* [LI_CLOSE],
    dt_element: "<" DT_NAME _attribute* ">" _node* [DT_CLOSE],
    dd_element: "<" DD_NAME _attribute* ">" _node* [DD_CLOSE],
    td_element: "<" TD_NAME _attribute* ">" _node* [TD_CLOSE],
    th_element: "<" TH_NAME _attribute* ">" _node* [TH_CLOSE],
    tr_element: "<" TR_NAME _attribute* ">" _node* [TR_CLOSE],
    option_element: "<" OPTION_NAME _attribute* ">" _node* [OPTION_CLOSE],
    optgroup_element: "<" OPTGROUP_NAME _attribute* ">" _node* [OPTGROUP_CLOSE],
    thead_element: "<" THEAD_NAME _attribute* ">" _node* [THEAD_CLOSE],
    tbody_element: "<" TBODY_NAME _attribute* ">" _node* [TBODY_CLOSE],
    tfoot_element: "<" TFOOT_NAME _attribute* ">" _node* [TFOOT_CLOSE],
    colgroup_element: "<" COLGROUP_NAME _attribute* ">" _node* [COLGROUP_CLOSE],

    tag: NAME,
    text_node: TEXT,
    comment_node: COMMENT,
    doctype_node: DOCTYPE,
} }

/// Names that never take a closing tag, in the HTML standard's spelling.
const VOID: &[&[u8]] = &[
    b"area", b"base", b"br", b"col", b"embed", b"hr", b"img", b"input", b"link", b"meta", b"param", b"source", b"track", b"wbr",
];

/// Elements whose end tag may be omitted, with the token each one's name and
/// closing tag lex to.
const OPTIONAL: &[(&[u8], Kind, Kind)] = &[
    (b"p", P_NAME, P_CLOSE),
    (b"li", LI_NAME, LI_CLOSE),
    (b"dt", DT_NAME, DT_CLOSE),
    (b"dd", DD_NAME, DD_CLOSE),
    (b"td", TD_NAME, TD_CLOSE),
    (b"th", TH_NAME, TH_CLOSE),
    (b"tr", TR_NAME, TR_CLOSE),
    (b"option", OPTION_NAME, OPTION_CLOSE),
    (b"optgroup", OPTGROUP_NAME, OPTGROUP_CLOSE),
    (b"thead", THEAD_NAME, THEAD_CLOSE),
    (b"tbody", TBODY_NAME, TBODY_CLOSE),
    (b"tfoot", TFOOT_NAME, TFOOT_CLOSE),
    (b"colgroup", COLGROUP_NAME, COLGROUP_CLOSE),
];

thread_local! {
    /// The element name whose `>` opens a raw-text body, once its start tag's
    /// name has been seen.
    static RAW_PENDING: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// The element name a raw-text body ends at.
    static RAW_NAME: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Whether the previous token leaves the lexer inside a tag, where `=`, a
/// quoted string and a bare word are markup rather than text.
fn in_tag_context(previous: Kind) -> bool {
    matches!(
        previous,
        crate::deep::syntax::UNKNOWN
            | LT
            | CLOSE_START
            | NAME
            | VOID_NAME
            | STRING
            | UNQUOTED
            | EQ
            | RAW_END
            | P_NAME
            | LI_NAME
            | DT_NAME
            | DD_NAME
            | TD_NAME
            | TH_NAME
            | TR_NAME
            | OPTION_NAME
            | OPTGROUP_NAME
            | THEAD_NAME
            | TBODY_NAME
            | TFOOT_NAME
            | COLGROUP_NAME
    )
}

/// The lower-case form of a name, or `None` when it is longer than any name in
/// the tables above.
fn lowered(text: &[u8]) -> Option<Vec<u8>> {
    if text.len() > 8 {
        return None;
    }
    Some(text.iter().map(u8::to_ascii_lowercase).collect())
}

pub fn lex_html(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT as TRIVIA_COMMENT, UNKNOWN, WHITESPACE};
    if c.pos() == 0 {
        // A new file starts outside every mode and with no raw element open.
        RAW_PENDING.with(|pending| pending.borrow_mut().take());
        RAW_NAME.with(|name| name.borrow_mut().clear());
    }
    let byte = c.peek();
    let previous = c.previous();
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
            // A closing tag of an element whose end tag may be omitted is one
            // token, `</p>`; any other closing tag is `</`, a name and `>`.
            let name = &c.rest()[2..];
            let length = name.iter().take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':')).count();
            if let Some(lower) = lowered(&name[..length]) {
                if let Some((_, _, close)) = OPTIONAL.iter().find(|(element, _, _)| *element == lower.as_slice()) {
                    c.bump_n(2 + length);
                    while !c.at_end() && c.peek() != b'>' {
                        c.bump_char();
                    }
                    c.eat(b'>');
                    return *close;
                }
            }
            c.bump_n(2);
            CLOSE_START
        }
        b'<' => {
            c.bump();
            LT
        }
        b'>' if in_tag_context(previous) => {
            c.bump();
            let pending = RAW_PENDING.with(|pending| pending.borrow_mut().take());
            match pending {
                Some(name) => {
                    RAW_NAME.with(|raw| *raw.borrow_mut() = name);
                    RAW_GT
                }
                None => GT,
            }
        }
        b'/' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            SLASH_GT
        }
        b'=' if in_tag_context(previous) => {
            c.bump();
            EQ
        }
        b'"' | b'\'' if in_tag_context(previous) => {
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
            let in_tag = in_tag_context(previous);
            let name_start = matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'@');
            if previous == EQ && !matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'>') {
                // An unquoted attribute value runs to whitespace or the end of
                // the tag, but the `/>` that closes the tag is not part of it.
                loop {
                    if c.at_end() || matches!(c.peek(), b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'>') {
                        break;
                    }
                    if c.peek() == b'/' && c.peek_at(1) == b'>' {
                        break;
                    }
                    c.bump_char();
                }
                return UNQUOTED;
            }
            if in_tag && name_start {
                let start = c.rest();
                let length = c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'@'));
                let text = &start[..length];
                let lower = lowered(text);
                if previous == LT {
                    if let Some(lower) = &lower {
                        if VOID.iter().any(|void| void == lower) {
                            return VOID_NAME;
                        }
                        if let Some((_, open, _)) = OPTIONAL.iter().find(|(element, _, _)| element == lower) {
                            return *open;
                        }
                        if lower == b"script" || lower == b"style" {
                            RAW_PENDING.with(|pending| *pending.borrow_mut() = Some(lower.clone()));
                        }
                    }
                    return NAME;
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

/// The step for a `script` or `style` body: text until that element's own
/// closing tag. The `</` of the closing tag is the token that pops the mode, so
/// the name after it is lexed as a tag name again.
pub fn lex_raw(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::WHITESPACE;
    let _ = WHITESPACE;
    if c.starts_with(b"</") {
        let rest = &c.rest()[2..];
        let length = rest.iter().take_while(|byte| byte.is_ascii_alphanumeric()).count();
        let lower = lowered(&rest[..length]);
        let matches = RAW_NAME.with(|name| {
            let name = name.borrow();
            !name.is_empty() && lower.as_deref() == Some(name.as_slice())
        });
        if matches {
            c.bump_n(2);
            RAW_NAME.with(|name| name.borrow_mut().clear());
            return RAW_END;
        }
    }
    let name = RAW_NAME.with(|name| name.borrow().clone());
    let rest = c.rest();
    let mut length = 0;
    while length < rest.len() {
        if rest[length] == b'<' && rest.get(length + 1) == Some(&b'/') {
            let after = &rest[length + 2..];
            let size = after.iter().take_while(|byte| byte.is_ascii_alphanumeric()).count();
            if size == name.len() && after[..size].eq_ignore_ascii_case(&name) {
                break;
            }
        }
        length += 1;
    }
    if length == 0 {
        c.bump_char();
        return crate::deep::syntax::UNKNOWN;
    }
    c.bump_n(length);
    RAW_BODY
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
            let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/longtail/html").join(name);
            let src = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            let mut workspace = Workspace::default();
            workspace.parse(&src, &super::HTML, &Budget::default()).unwrap();
            Parsed { src, workspace }
        }

        fn node(&self, index: usize) -> Node {
            self.workspace.tree.nodes[index]
        }

        fn kind(&self, index: usize) -> &'static str {
            super::HTML.name(self.node(index).kind)
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
    fn a_script_body_is_raw_text_and_not_markup() {
        let parsed = Parsed::fixture("html_raw_text.html");
        assert_eq!(parsed.errors(), 0, "the fixture must parse cleanly");
        let bodies = parsed.of_kind("raw_text");
        assert_eq!(bodies.len(), 2, "one style body and one script body");
        let texts: Vec<&[u8]> = bodies
            .iter()
            .map(|&index| {
                let node = parsed.node(index);
                &parsed.src[node.start as usize..node.end as usize]
            })
            .collect();
        assert!(
            texts.iter().any(|text| text.windows(6).any(|window| window == b"</div>")),
            "the `</div>` inside the script is text: {texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.windows(3).any(|window| window == b"a>b")),
            "the `>` inside the style attribute selector is text: {texts:?}"
        );
        // No element starts inside either raw body: the `<` never began a tag.
        for index in 1..parsed.workspace.tree.nodes.len() {
            if parsed.kind(index) == "element" && !parsed.is_token(index) {
                let node = parsed.node(index);
                for &body in &bodies {
                    let span = parsed.node(body);
                    assert!(
                        node.start <= span.start || node.start >= span.end,
                        "an element at {} would be inside the raw body at {}..{}",
                        node.start,
                        span.start,
                        span.end
                    );
                }
            }
        }
    }

    #[test]
    fn the_doctype_and_an_unquoted_attribute_keep_their_spans() {
        let parsed = Parsed::fixture("html_raw_text.html");
        let doctype = &parsed.workspace.tree.nodes[parsed.of_kind("doctype_node")[0]];
        assert_eq!((doctype.start, doctype.end), (0, 15), "<!DOCTYPE html>");
        let unquoted: Vec<&str> = parsed.leaves_of_kind("unquoted_attribute_value").iter().map(|&index| parsed.text(index)).collect();
        assert!(unquoted.contains(&"utf-8"), "{unquoted:?}");
        assert!(unquoted.contains(&"page"), "{unquoted:?}");
    }

    #[test]
    fn an_omitted_end_tag_still_closes_its_element() {
        let parsed = Parsed::fixture("html_optional_tags.html");
        assert_eq!(parsed.errors(), 0);
        let items = parsed.of_kind("li_element");
        assert_eq!(items.len(), 3, "three list items, none with `</li>`");
        let cells = parsed.of_kind("td_element");
        assert_eq!(cells.len(), 4, "two rows of two cells, none with `</td>`");
        assert_eq!(parsed.of_kind("p_element").len(), 2);
        assert_eq!(parsed.of_kind("option_element").len(), 2);
        assert_eq!(parsed.of_kind("dt_element").len(), 2);
        assert_eq!(parsed.of_kind("dd_element").len(), 2);
        for &index in &items {
            assert!(
                !parsed.workspace.tree.children(index as u32).any(|child| parsed.kind(child as usize) == "closing_tag"),
                "an omitted end tag leaves no closing_tag child"
            );
        }
        // The third item's span ends where the next item begins, not at the
        // closing tag of the list.
        let third = &parsed.workspace.tree.nodes[items[2]];
        let text = &parsed.src[third.start as usize..third.end as usize];
        assert_eq!(text, b"<li>third\n");
    }

    #[test]
    fn a_mismatched_tag_costs_its_subtree_and_not_the_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/longtail/malformed/html_broken.html");
        let src = std::fs::read(&path).unwrap();
        let mut workspace = Workspace::default();
        workspace.parse(&src, &super::HTML, &Budget::default()).unwrap();
        assert!(workspace.tree.nodes.iter().skip(1).any(|node| node.kind == crate::deep::syntax::ERROR));
        assert_eq!(super::HTML.name(workspace.tree.root().kind), "document");
        assert_eq!((workspace.tree.root().start, workspace.tree.root().end), (0, src.len() as u32));
    }

    #[test]
    fn every_prefix_of_the_fixtures_parses_or_errors_without_panicking() {
        for name in ["html_raw_text.html", "html_optional_tags.html"] {
            let parsed = Parsed::fixture(name);
            for end in (0..parsed.src.len()).step_by(11).chain(std::iter::once(parsed.src.len())) {
                let mut workspace = Workspace::default();
                workspace.parse(&parsed.src[..end], &super::HTML, &Budget::default()).unwrap();
            }
        }
    }
}
