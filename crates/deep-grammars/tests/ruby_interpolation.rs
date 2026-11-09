//! Interpolation is a field of code, not text.
//!
//! This test pins the shape a `#{...}` field has in the tree, with the byte
//! span of every node it names: a field is an `interpolation` node over its
//! `#{` token, the expression's own nodes, and its `}` token, and a call
//! written in a field is an ordinary `call` node whose span is the call and
//! nothing else. Nested strings and braces belong to their own nodes, so the
//! field that holds `#{ {a: 1}[:a] }` closes at its own brace.
//!
//! A field that cannot close must still produce a tree: the parse finishes,
//! the root survives, and the error lives inside it. That is the malformed
//! rule the fixtures in `fixtures/ruby/malformed/` state in the abstract, here
//! applied to the one construct whose lexer is a mode.

use std::fs;
use std::path::Path;

use deep_grammars::batch_b::ruby::RUBY;
use deep_grammars::deep::syntax::ERROR;
use deep_grammars::deep::tree::{HAS_ERROR, TOKEN};
use deep_grammars::deep::{Budget, Workspace};

/// One node of the parse, kept after the workspace is reused.
#[derive(Clone)]
struct Node {
    kind: &'static str,
    start: u32,
    end: u32,
    children: Vec<Node>,
}

fn parse(src: &[u8]) -> (bool, Vec<Node>) {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src, &RUBY, &Budget::default()).expect("the input fits");
    let tree = &workspace.tree;
    fn build(tree: &deep_grammars::deep::tree::Tree, index: u32) -> Node {
        let node = &tree.nodes[index as usize];
        let children = tree.children(index).map(|child| build(tree, child)).collect();
        Node { kind: RUBY.name(node.kind), start: node.start, end: node.end, children }
    }
    let nodes = tree.children(0).map(|child| build(tree, child)).collect();
    (outcome.complete(), nodes)
}

fn errors(nodes: &[Node]) -> usize {
    nodes.iter().map(|node| (node.kind == "ERROR") as usize + errors(&node.children)).sum()
}

/// Every node of one kind, depth first.
fn find<'a>(nodes: &'a [Node], kind: &str, out: &mut Vec<&'a Node>) {
    for node in nodes {
        if node.kind == kind {
            out.push(node);
        }
        find(&node.children, kind, out);
    }
}

fn text<'a>(src: &'a [u8], node: &Node) -> &'a str {
    std::str::from_utf8(&src[node.start as usize..node.end as usize]).expect("the fixture is utf-8")
}

fn fixture() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join("ruby_interpolation.rb");
    fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
}

#[test]
fn the_fixture_parses_without_an_error_node() {
    let src = fixture();
    let (complete, tree) = parse(&src);
    assert!(complete, "the parse did not finish");
    assert_eq!(errors(&tree), 0, "an error node in a valid file");
}

#[test]
fn a_field_is_a_node_over_its_braces() {
    let src = fixture();
    let (_, tree) = parse(&src);
    let mut fields = Vec::new();
    find(&tree, "interpolation", &mut fields);
    assert_eq!(fields.len(), 9, "one node per field:\ngot {}", fields.iter().map(|node| text(&src, node)).collect::<Vec<_>>().join(" | "));
    for field in fields {
        let span = text(&src, field);
        assert!(span.starts_with("#{"), "{span:?} does not open with the field's `#{{`");
        assert!(span.ends_with('}'), "{span:?} does not close with the field's `}}`");
        assert!(field.end - field.start >= 4, "{span:?} is too short to hold a field");
    }
}

#[test]
fn a_call_in_a_field_is_a_call_node_with_its_own_span() {
    let src = fixture();
    let (_, tree) = parse(&src);
    let mut strings = Vec::new();
    find(&tree, "interpolated_string", &mut strings);
    let line = strings
        .iter()
        .find(|node| text(&src, node).starts_with("\"result:"))
        .expect("the `call = \"result: ...\"` line");

    let mut calls = Vec::new();
    find(&line.children, "call", &mut calls);
    assert_eq!(calls.len(), 1, "one call in the field");
    assert_eq!(text(&src, calls[0]), "fetch(name)");

    // The node's span is the call itself: its `fetch`, its name argument, its
    // parentheses, and not one byte of the string around it.
    let call = calls[0];
    assert_eq!(call.start as usize, src.windows(11).position(|w| w == b"fetch(name)").unwrap());
    assert_eq!(call.end - call.start, 11);
}

#[test]
fn a_nested_string_keeps_its_own_fields() {
    let src = fixture();
    let (_, tree) = parse(&src);
    let mut strings = Vec::new();
    find(&tree, "interpolated_string", &mut strings);
    let outer = strings
        .iter()
        .find(|node| text(&src, node).starts_with("\"a #{"))
        .expect("the nested line");
    let fields: Vec<&Node> = outer.children.iter().filter(|node| node.kind == "interpolation").collect();
    assert_eq!(fields.len(), 1, "the outer field is one");
    let inner: Vec<&Node> = fields[0].children.iter().filter(|node| node.kind == "interpolated_string").collect();
    assert_eq!(inner.len(), 1, "the nested string is one node");
    let inner_fields: Vec<&Node> = inner[0].children.iter().filter(|node| node.kind == "interpolation").collect();
    assert_eq!(inner_fields.len(), 1, "the nested field belongs to the nested string");
    assert_eq!(text(&src, inner_fields[0]), "#{name}");
}

#[test]
fn a_brace_in_a_field_closes_at_its_own_depth() {
    let src = fixture();
    let (_, tree) = parse(&src);
    let mut strings = Vec::new();
    find(&tree, "interpolated_string", &mut strings);
    let line = strings
        .iter()
        .find(|node| text(&src, node).starts_with("\"#{ {"))
        .expect("the hash line");
    let mut fields = Vec::new();
    find(&line.children, "interpolation", &mut fields);
    assert_eq!(text(&src, fields[0]), "#{ {a: 1}[:a] }");
    let mut hashes = Vec::new();
    find(&fields[0].children, "hash", &mut hashes);
    assert_eq!(hashes.len(), 1, "the field holds one hash");
}

#[test]
fn a_field_that_cannot_close_reports_an_error_inside_the_tree() {
    let broken: [&[u8]; 3] = [b"x = \"a #{", b"x = \"a #{f(\"\n", b"x = \"#{'unterminated\n"];
    for broken in broken {
        let (complete, tree) = parse(broken);
        assert!(complete, "{:?}: the parse did not finish", String::from_utf8_lossy(broken));
        assert!(!tree.is_empty(), "the root survives");
        assert!(errors(&tree) > 0, "{:?}: no error in the tree", String::from_utf8_lossy(broken));
    }
}

#[test]
fn an_unterminated_string_costs_its_line_not_the_file() {
    // A quote with no closing quote anywhere ends at its line, so the `end`s
    // after it are still keywords and the method still closes.
    let src = b"def value\n  text = \"unterminated\nend\n";
    let (complete, tree) = parse(src);
    assert!(complete);
    assert!(errors(&tree) > 0, "the bad string is an error");
    let mut definitions = Vec::new();
    find(&tree, "method_definition", &mut definitions);
    assert_eq!(definitions.len(), 1, "the method is still a method");
    let kinds: Vec<&str> = tree.iter().map(|node| node.kind).collect();
    assert_eq!(kinds, ["method_definition", "ERROR"], "the method closed and the bad string is its own node");
    let body = text(src, &definitions[0]);
    assert!(body.starts_with("def value"), "the def survived: {body:?}");
    // The bad literal is the last thing the method's span holds: the two `end`s
    // after it were lexed as keywords, so the method closed normally.
    assert!(body.ends_with("text ="), "the bad literal ends the method's span: {body:?}");
}

/// The field's span is checked against the source byte offsets, so a span that
/// is off by the two bytes of `#{` fails even if the text looks right.
#[test]
fn every_field_span_covers_the_bytes_from_its_hash_brace_to_its_close() {
    let src = fixture();
    let (_, tree) = parse(&src);
    let mut fields = Vec::new();
    find(&tree, "interpolation", &mut fields);
    for field in fields {
        let span = text(&src, field);
        assert_eq!(span.as_bytes()[0..2], *b"#{", "{span:?}");
        assert_eq!(span.as_bytes()[span.len() - 1], b'}', "{span:?}");
        let before = &src[field.start as usize - 1];
        assert!(matches!(before, b'"' | b' ' | b'(' | b',' | b'['), "the field starts right after a string byte: {span:?}");
    }
}

/// A token flag no test leaves behind: every visible leaf either belongs to a
/// named kind or is the file's own trivia.
#[test]
fn no_node_is_a_bad_token() {
    let src = fixture();
    let mut workspace = Workspace::default();
    workspace.parse(&src, &RUBY, &Budget::default()).unwrap();
    let bad = workspace
        .tree
        .nodes
        .iter()
        .filter(|node| node.kind == ERROR || node.flags & HAS_ERROR != 0 && node.flags & TOKEN != 0)
        .count();
    assert_eq!(bad, 0);
}
