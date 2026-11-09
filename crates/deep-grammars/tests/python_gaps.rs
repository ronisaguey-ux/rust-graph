//! The four Python grammar gaps the differential fixtures recorded, one test
//! each: the CST must name what the one-pass lexer named.
//!
//! - a star import is a real node, not a comment-shaped omission;
//! - `a, b = f(x), 1` keeps its targets as list nodes a reader can name;
//! - `f"{x}"` keeps its fields, with spans over the expressions inside;
//! - `three = lambda z: z` names the binding `three`.
//!
//! Each test reads the shipped fixture, so the fixture and the assertion
//! cannot drift apart, and asserts node kinds and byte spans rather than
//! "the file parsed".

use std::fs;
use std::path::{Path, PathBuf};


use deep_grammars::deep::tree::{Node, TOKEN};
use deep_grammars::deep::{Budget, Workspace};
use deep_grammars::python::PYTHON;

struct Parsed {
    src: Vec<u8>,
    workspace: Workspace,
}

impl Parsed {
    fn node(&self, index: usize) -> Node {
        self.workspace.tree.nodes[index]
    }

    fn kind(&self, index: usize) -> &'static str {
        PYTHON.name(self.node(index).kind)
    }

    fn text(&self, index: usize) -> &str {
        let node = self.node(index);
        std::str::from_utf8(&self.src[node.start as usize..node.end as usize]).expect("utf8 fixture")
    }

    fn is_token(&self, index: usize) -> bool {
        self.node(index).flags & TOKEN != 0
    }

    /// Every node of `kind` below the root, as indices.
    fn of_kind(&self, kind: &str) -> Vec<usize> {
        (1..self.workspace.tree.nodes.len())
            .filter(|&index| !self.is_token(index) && self.kind(index) == kind)
            .collect()
    }

    /// Direct child nodes of `index`, tokens included.
    fn children(&self, index: usize) -> Vec<usize> {
        self.workspace.tree.children(index as u32).map(|child| child as usize).collect()
    }

    fn error_nodes(&self) -> usize {
        use deep_grammars::deep::syntax::ERROR;
        (0..self.workspace.tree.nodes.len())
            .filter(|&index| self.node(index).kind == ERROR || (index > 0 && self.is_token(index) && self.node(index).flags & deep_grammars::deep::tree::HAS_ERROR != 0))
            .count()
    }
}

fn parse(name: &str) -> Parsed {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/python").join(name);
    parse_path(&path)
}

fn parse_path(path: &Path) -> Parsed {
    let src = fs::read(path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    let mut workspace = Workspace::default();
    workspace.parse(&src, &PYTHON, &Budget::default()).unwrap_or_else(|_| panic!("{path:?}: too large"));
    Parsed { src, workspace }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/python").join(name)
}

#[test]
fn a_star_import_is_a_real_node_with_a_span_over_the_star() {
    let parsed = parse("star_import.py");
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&PYTHON, true));
    let imports = parsed.of_kind("import_from_statement");
    assert_eq!(imports.len(), 2, "{}", parsed.workspace.tree.sexp(&PYTHON, true));
    let star = imports[0];
    assert_eq!((parsed.node(star).start, parsed.node(star).end), (0, 15), "the whole clause");
    // The star is a leaf the extraction can read: a direct child of the clause,
    // by text `*`, spanning the one byte at offset 14.
    let star_leaf = parsed
        .children(star)
        .into_iter()
        .find(|&child| parsed.is_token(child) && parsed.kind(child) == "*")
        .expect("a `*` child of `from m import *`");
    assert_eq!((parsed.node(star_leaf).start, parsed.node(star_leaf).end), (14, 15), "the star byte");
    // Nothing wraps it: `wildcard_import` was the node that hid the star from a
    // reader that looks at the clause's children.
    assert!(parsed.of_kind("wildcard_import").is_empty(), "the star is not one opaque wrapper");
}

#[test]
fn tuple_and_list_targets_are_named_by_their_list_nodes() {
    let parsed = parse("tuple_assignment.py");
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&PYTHON, true));
    let assignments = parsed.of_kind("assignment");
    assert_eq!(assignments.len(), 5, "three statements and `p = q = 1`'s two nodes:\n{}", parsed.workspace.tree.sexp(&PYTHON, true));

    // `a, b = f(x), 1`: the left side is a pattern list spanning `a, b`.
    let first = assignments[0];
    assert_eq!(parsed.text(first), "a, b = f(x), 1");
    let target = parsed.children(first)[0];
    assert_eq!(parsed.kind(target), "pattern_list", "{}", parsed.workspace.tree.sexp(&PYTHON, true));
    assert_eq!((parsed.node(target).start, parsed.node(target).end), (0, 4), "`a, b`");
    let names: Vec<&str> =
        parsed.children(target).into_iter().filter(|&child| parsed.is_token(child) && parsed.kind(child) == "identifier").map(|child| parsed.text(child)).collect();
    assert_eq!(names, ["a", "b"]);

    // `[c, d] = pair()`: a list target keeps the list node and both names.
    let second = assignments[1];
    let target = parsed.children(second)[0];
    assert_eq!(parsed.kind(target), "list");
    assert_eq!((parsed.node(target).start, parsed.node(target).end), (15, 21), "`[c, d]`");
    let names: Vec<&str> =
        parsed.children(target).into_iter().filter(|&child| parsed.is_token(child) && parsed.kind(child) == "identifier").map(|child| parsed.text(child)).collect();
    assert_eq!(names, ["c", "d"]);

    // `obj.attr = a` is still one named target, not a pattern list.
    let third = assignments[2];
    let target = parsed.children(third)[0];
    assert_eq!(parsed.kind(target), "attribute");
    assert_eq!(parsed.text(target), "obj.attr");

    // `p = q = 1` nests a second assignment, so both bindings are named.
    let chain = assignments.iter().copied().find(|&node| parsed.text(node) == "p = q = 1").expect("the chain");
    let inner = parsed.children(chain).into_iter().rev().find(|&child| !parsed.is_token(child)).expect("the nested assignment");
    assert_eq!(parsed.kind(inner), "assignment");
    assert_eq!(parsed.text(inner), "q = 1");
}

#[test]
fn an_assigned_lambda_exposes_the_binding_name() {
    let parsed = parse("assigned_lambda.py");
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&PYTHON, true));
    let definitions = parsed.of_kind("function_definition");
    assert_eq!(definitions.len(), 3, "two bindings and `def use`:\n{}", parsed.workspace.tree.sexp(&PYTHON, true));

    // `three = lambda z: z` is a definition whose name is the binding, and the
    // lambda keeps its parameter and body.
    let three = definitions[0];
    assert_eq!((parsed.node(three).start, parsed.node(three).end), (0, 19));
    let name = parsed
        .children(three)
        .into_iter()
        .find(|&child| parsed.is_token(child) && parsed.kind(child) == "identifier")
        .expect("the binding name");
    assert_eq!(parsed.text(name), "three");
    let lambda = parsed.of_kind("lambda").into_iter().find(|&node| node != three).or_else(|| parsed.children(three).into_iter().find(|&child| parsed.kind(child) == "lambda")).expect("a lambda body");
    assert!(parsed.text(lambda).starts_with("lambda z: z"), "{}", parsed.text(lambda));
    assert_eq!(parsed.kind(parsed.children(lambda)[0]), "lambda", "the lambda node keeps its keyword child");

    // `scale = lambda x, k=2: x * k`: the second binding is named too.
    let scale = definitions[1];
    let name = parsed
        .children(scale)
        .into_iter()
        .find(|&child| parsed.is_token(child) && parsed.kind(child) == "identifier")
        .expect("the binding name");
    assert_eq!(parsed.text(name), "scale");
}

#[test]
fn an_f_string_keeps_each_field_with_its_own_span() {
    let parsed = parse("fstring_fields.py");
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&PYTHON, true));
    let strings = parsed.of_kind("f_string");
    assert_eq!(strings.len(), 4, "`message`, `padded`, `braces`, `document`:\n{}", parsed.workspace.tree.sexp(&PYTHON, true));

    let find = |needle: &str| parsed.src.windows(needle.len()).position(|window| window == needle.as_bytes()).expect("the construct is in the fixture");

    // `message = f"hi {name}"`: the whole f-string is one node and the field is
    // a node of its own whose span is exactly `{name}`.
    let message = strings[0];
    let start = find("f\"hi {name}\"");
    assert_eq!((parsed.node(message).start, parsed.node(message).end), (start as u32, (start + 12) as u32), "the f-string's bytes");
    assert_eq!(parsed.kind(parsed.children(message)[0]), "fstring_start");
    let field = parsed.of_kind("interpolation").into_iter().find(|&node| parsed.text(node) == "{name}").expect("the `{name}` field");
    let field_start = find("{name}");
    assert_eq!((parsed.node(field).start, parsed.node(field).end), (field_start as u32, (field_start + 6) as u32), "the field's bytes");
    assert_eq!(parsed.text(field), "{name}");
    assert_eq!(parsed.kind(parsed.children(field)[0]), "fstring_field", "the field is a token, so its bytes can be sliced");

    // `padded = f"{name!r:>10}"`: conversion and format spec are inside the
    // field's one span, so the expression `name` is still addressable.
    let padded = parsed.of_kind("interpolation").into_iter().find(|&node| parsed.text(node).contains("!r:>10")).expect("the conversion field");
    assert_eq!(parsed.text(padded), "{name!r:>10}");

    // `braces = f"{{literal}} {name}"`: the escaped braces stay text and only
    // the real field is an interpolation.
    let braces = strings[2];
    let fields: Vec<&str> = parsed.of_kind("interpolation").into_iter().filter(|&node| parsed.node(node).start >= parsed.node(braces).start && parsed.node(node).end <= parsed.node(braces).end).map(|node| parsed.text(node)).collect();
    assert_eq!(fields, ["{name}"], "escaped braces are text");

    // A triple-quoted f-string keeps its text across the line break and still
    // ends at its closing quotes.
    let document = strings[3];
    let span = (parsed.node(document).start, parsed.node(document).end);
    let start = find("f\"\"\"multi");
    assert_eq!(span, (start as u32, (start + 24) as u32), "the triple string's bytes");
}

#[test]
fn the_gap_fixtures_are_the_shipped_files() {
    for name in ["star_import.py", "tuple_assignment.py", "fstring_fields.py", "assigned_lambda.py"] {
        assert!(fixture(name).is_file(), "{name} is a fixture, not an inline string");
    }
}

