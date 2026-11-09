//! JSX elements, one test per construct: the tags are tokens and nodes, and
//! every `{...}` part is ordinary code again.
//!
//! Each test reads the shipped fixture, so the fixture and the assertion cannot
//! drift apart, and asserts node kinds and byte spans rather than "the file
//! parsed".

use std::fs;
use std::path::{Path, PathBuf};

use deep_grammars::deep::syntax::{Language, ERROR};
use deep_grammars::deep::tree::{Node, HAS_ERROR, TOKEN};
use deep_grammars::deep::{Budget, Workspace};
use deep_grammars::javascript::{JAVASCRIPT, TSX};

struct Parsed {
    src: Vec<u8>,
    workspace: Workspace,
    language: &'static Language,
}

impl Parsed {
    fn node(&self, index: usize) -> Node {
        self.workspace.tree.nodes[index]
    }

    fn kind(&self, index: usize) -> &'static str {
        self.language.name(self.node(index).kind)
    }

    fn text(&self, index: usize) -> &str {
        let node = self.node(index);
        std::str::from_utf8(&self.src[node.start as usize..node.end as usize]).expect("utf8 fixture")
    }

    fn span(&self, index: usize) -> (u32, u32) {
        let node = self.node(index);
        (node.start, node.end)
    }

    fn is_token(&self, index: usize) -> bool {
        self.node(index).flags & TOKEN != 0
    }

    /// A leaf token of `kind`, which `of_kind` leaves out on purpose.
    fn find_token(&self, kind: &str, text: &str) -> usize {
        (1..self.workspace.tree.nodes.len())
            .find(|&index| self.is_token(index) && self.kind(index) == kind && self.text(index) == text)
            .unwrap_or_else(|| panic!("no {kind} token with text {text:?}:\n{}", self.workspace.tree.sexp(self.language, true)))
    }

    fn of_kind(&self, kind: &str) -> Vec<usize> {
        (1..self.workspace.tree.nodes.len()).filter(|&index| !self.is_token(index) && self.kind(index) == kind).collect()
    }

    fn find(&self, kind: &str, text: &str) -> usize {
        self.of_kind(kind)
            .into_iter()
            .find(|&index| self.text(index) == text)
            .unwrap_or_else(|| panic!("no {kind} node with text {text:?}:\n{}", self.workspace.tree.sexp(self.language, true)))
    }

    fn at(&self, needle: &str) -> u32 {
        let at = self.src.windows(needle.len()).position(|window| window == needle.as_bytes());
        at.unwrap_or_else(|| panic!("{needle:?} is in the fixture")) as u32
    }

    fn bytes_of(&self, needle: &str) -> (u32, u32) {
        let start = self.at(needle);
        (start, start + needle.len() as u32)
    }

    fn children(&self, index: usize) -> Vec<usize> {
        self.workspace.tree.children(index as u32).map(|child| child as usize).collect()
    }

    fn error_nodes(&self) -> usize {
        (0..self.workspace.tree.nodes.len())
            .filter(|&index| self.node(index).kind == ERROR || (index > 0 && self.is_token(index) && self.node(index).flags & HAS_ERROR != 0))
            .count()
    }
}

fn fixture(folder: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(folder).join(name)
}

fn parse_path(path: &Path, language: &'static Language) -> Parsed {
    let src = fs::read(path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    let mut workspace = Workspace::default();
    workspace.parse(&src, language, &Budget::default()).unwrap_or_else(|_| panic!("{path:?}: too large"));
    Parsed { src, workspace, language }
}

fn parse(folder: &str, name: &str, language: &'static Language) -> Parsed {
    parse_path(&fixture(folder, name), language)
}

#[test]
fn an_attribute_value_in_braces_is_code_and_its_call_is_a_call() {
    let parsed = parse("javascript", "jsx_attributes.jsx", &JAVASCRIPT);
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&JAVASCRIPT, true));

    let opening = "<button onClick={() => handle(x)} title=\"press &amp; go\" data-id={id(x)} {...props}>press</button>";
    let element = parsed.find("jsx_element", opening);
    assert_eq!(parsed.span(element), parsed.bytes_of(opening));

    // The event handler's arrow is the attribute value, and the call in it is
    // the callee `handle` with the argument `x`, at its own span.
    let expression = parsed.find("jsx_expression", "{() => handle(x)}");
    assert_eq!(parsed.span(expression), parsed.bytes_of("{() => handle(x)}"));
    let call = parsed.find("call_expression", "handle(x)");
    assert_eq!(parsed.span(call), parsed.bytes_of("handle(x)"));
    assert!((parsed.span(expression).0..parsed.span(expression).1).contains(&parsed.span(call).0));
    let parts = parsed.children(call);
    assert_eq!(parsed.kind(parts[0]), "identifier");
    assert_eq!(parsed.text(parts[0]), "handle");
    assert_eq!(parsed.text(parts[1]), "(x)");

    // `data-id={id(x)}`: the first call after the handler.
    let value = parsed.find("jsx_expression", "{id(x)}");
    assert_eq!(parsed.span(value), parsed.bytes_of("{id(x)}"));
    assert_eq!(parsed.span(parsed.find("call_expression", "id(x)")), parsed.bytes_of("id(x)"));

    // The quoted value is one string token and its entity stays undecoded.
    let text = parsed.find_token("jsx_string", "\"press &amp; go\"");
    assert_eq!(parsed.text(text), "\"press &amp; go\"", "the entity is not decoded");

    // `{...props}` is a spread attribute, not a value: the expression inside
    // is the identifier `props`.
    let spread = parsed.find("jsx_spread_attribute", "{...props}");
    assert_eq!(parsed.span(spread), parsed.bytes_of("{...props}"));
    assert!(parsed.children(spread).iter().any(|&child| parsed.kind(child) == "identifier" && parsed.text(child) == "props"));

    // The second function's attribute call, too.
    assert_eq!(parsed.span(parsed.find("call_expression", "label(user)")), parsed.bytes_of("label(user)"));
}

#[test]
fn a_member_tag_and_a_namespaced_tag_keep_their_shape() {
    let parsed = parse("javascript", "jsx_elements.jsx", &JAVASCRIPT);
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&JAVASCRIPT, true));

    // `<A.B/>`: a self-closing element whose name is a nested identifier.
    let member = parsed.find("jsx_element", "<A.B/>");
    assert_eq!(parsed.span(member), parsed.bytes_of("<A.B/>"));
    assert!(parsed.children(member).iter().any(|&child| parsed.kind(child) == "jsx_self_closing"));
    let nested = parsed.find("nested_identifier", "A.B");
    assert_eq!(parsed.span(nested), parsed.bytes_of("A.B"));
    assert_eq!(parsed.children(nested).iter().filter(|&&child| parsed.kind(child) == "jsx_identifier").count(), 2, "two segments");

    // `<A.B.C>` names three segments.
    let deep = parsed.find("nested_identifier", "A.B.C");
    assert_eq!(parsed.span(deep), parsed.bytes_of("A.B.C"));
    assert_eq!(parsed.children(deep).iter().filter(|&&child| parsed.kind(child) == "jsx_identifier").count(), 3);

    // `<a:b>` is a namespace name, with the colon between the two segments.
    let namespaced = parsed.find("jsx_namespace_name", "a:b");
    assert_eq!(parsed.span(namespaced), parsed.bytes_of("a:b"));
    assert_eq!(parsed.text(parsed.find("jsx_element", "<a:b>text</a:b>")), "<a:b>text</a:b>");
}

#[test]
fn a_paired_element_has_text_children_a_self_closing_child_and_expression_children() {
    let parsed = parse("javascript", "jsx_elements.jsx", &JAVASCRIPT);

    // `<div><span/>{f(y)}</div>`: the children are the self-closing element,
    // the call's expression, and the text between them when there is one.
    let pair = parsed.find("jsx_element", "<div><span/>{f(y)}</div>");
    assert_eq!(parsed.span(pair), parsed.bytes_of("<div><span/>{f(y)}</div>"));
    assert_eq!(parsed.text(parsed.find("jsx_element", "<span/>")), "<span/>");
    assert_eq!(parsed.span(parsed.find("call_expression", "f(y)")), parsed.bytes_of("f(y)"));

    // Text stays a text token: `text` inside the namespaced pair.
    let text = parsed.find_token("jsx_text", "text");
    let namespaced = parsed.span(parsed.find("jsx_element", "<a:b>text</a:b>"));
    assert!((namespaced.0..namespaced.1).contains(&parsed.span(text).0), "the text is the namespaced pair's child");

    // `items.map(item => <li key={item.id}>{item.name}</li>)`: the element is
    // the arrow's body, both of its `{...}` parts are member expressions.
    let list = parsed.find("jsx_element", "<ul>{items.map(item => <li key={item.id}>{item.name}</li>)}</ul>");
    assert!(parsed.children(list).iter().any(|&child| parsed.kind(child) == "jsx_expression" && parsed.text(child) == "{items.map(item => <li key={item.id}>{item.name}</li>)}"));
    assert!(parsed.of_kind("call_expression").iter().any(|&node| parsed.text(node) == "items.map(item => <li key={item.id}>{item.name}</li>)"));
    assert!(parsed.of_kind("jsx_attribute").iter().any(|&node| parsed.text(node) == "key={item.id}"));
}

#[test]
fn a_fragment_holds_children_and_its_expressions() {
    let parsed = parse("javascript", "jsx_elements.jsx", &JAVASCRIPT);
    let fragment = parsed.find("jsx_element", "<>before{g(x)}after</>");
    assert_eq!(parsed.span(fragment), parsed.bytes_of("<>before{g(x)}after</>"));
    let inner = parsed.find("jsx_fragment", "<>before{g(x)}after</>");
    assert_eq!(parsed.children(inner).len(), 5, "open, text, expression, text, close");
    assert_eq!(parsed.kind(parsed.children(inner)[0]), "jsx_fragment_open");
    assert_eq!(parsed.kind(parsed.children(inner)[4]), "jsx_fragment_close");
    assert_eq!(parsed.span(parsed.find("call_expression", "g(x)")), parsed.bytes_of("g(x)"));
    assert_eq!(parsed.text(parsed.find_token("jsx_text", "before")), "before");
}

#[test]
fn a_template_a_jsx_element_and_a_template_all_open_at_once() {
    let parsed = parse("javascript", "jsx_nested.jsx", &JAVASCRIPT);
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&JAVASCRIPT, true));

    // `x ${<div a={`inner ${f(y)}`}>{<span/>}</div>} z`: the outer template's
    // substitution holds the element, whose attribute value is a template
    // whose substitution holds the call.
    let literal = "`x ${<div a={`inner ${f(y)}`}>{<span/>}</div>} z`";
    let outer = parsed.find("template_string", literal);
    assert_eq!(parsed.span(outer), parsed.bytes_of(literal));

    let element = "<div a={`inner ${f(y)}`}>{<span/>}</div>";
    let jsx = parsed.find("jsx_element", element);
    assert_eq!(parsed.span(jsx), parsed.bytes_of(element));

    let attribute = "{`inner ${f(y)}`}";
    assert_eq!(parsed.span(parsed.find("jsx_expression", attribute)), parsed.bytes_of(attribute));
    let inner = "`inner ${f(y)}`";
    assert_eq!(parsed.span(parsed.find("template_string", inner)), parsed.bytes_of(inner));
    assert_eq!(parsed.span(parsed.find("template_substitution", "${f(y)}")), parsed.bytes_of("${f(y)}"));
    assert_eq!(parsed.span(parsed.find("call_expression", "f(y)")), parsed.bytes_of("f(y)"));

    // The child `{<span/>}` is an expression holding an element.
    let child = "{<span/>}";
    let expression = parsed.find("jsx_expression", child);
    assert_eq!(parsed.span(expression), parsed.bytes_of(child));
    assert!(parsed.children(expression).iter().any(|&node| parsed.kind(node) == "jsx_element" && parsed.text(node) == "<span/>"));
}

#[test]
fn tsx_parses_the_same_elements_with_types_inside_them() {
    let parsed = parse("javascript", "jsx_types.tsx", &TSX);
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&TSX, true));

    // `format<User>(user)` is a call with type arguments in a child expression.
    let child = "{format<User>(user)}";
    assert_eq!(parsed.span(parsed.find("jsx_expression", child)), parsed.bytes_of(child));
    let call = parsed.find("call_expression", "format<User>(user)");
    assert_eq!(parsed.span(call), parsed.bytes_of("format<User>(user)"));
    assert!(parsed.children(call).iter().any(|&node| parsed.kind(node) == "type_arguments" && parsed.text(node) == "<User>"));

    // The attribute template's substitution is a member expression.
    assert_eq!(parsed.span(parsed.find("template_substitution", "${user.name}")), parsed.bytes_of("${user.name}"));

    // A tag's type arguments are not its end: `<Select<Option> ... />` is one
    // self-closing element with its value an expression.
    let element = "<Select<Option> value={pick(selected)} />";
    assert_eq!(parsed.span(parsed.find("jsx_element", element)), parsed.bytes_of(element));
    assert_eq!(parsed.span(parsed.find("call_expression", "pick(selected)")), parsed.bytes_of("pick(selected)"));
}

#[test]
fn malformed_elements_are_errors_inside_a_surviving_tree() {
    for name in ["jsx_unclosed_tag.js", "jsx_unclosed_pair.js", "jsx_broken_attr.js", "jsx_unclosed_eof.js"] {
        let parsed = parse("malformed", name, &JAVASCRIPT);
        assert_eq!(parsed.kind(0), "program", "{name}: the root survived");
        assert!(parsed.error_nodes() > 0, "{name}: the broken element is reported:\n{}", parsed.workspace.tree.sexp(&JAVASCRIPT, true));
        assert!(parsed.workspace.tree.nodes[0].end as usize == parsed.src.len(), "{name}: the tree spans the file");
    }

    // A broken attribute recovers: the declaration after it still parses.
    let recovered = parse("malformed", "jsx_broken_attr.js", &JAVASCRIPT);
    assert_eq!(recovered.of_kind("lexical_declaration").len(), 2, "the next statement is not swallowed");
}
