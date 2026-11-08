//! JSON on the runtime, written by hand: the path for a caller with no tables.
//!
//! [`deep_rt::json::JSON`] is a complete grammar — lexer, kinds and a recursive
//! descent parser — built directly on this crate. Nothing is generated and nothing
//! is a stub, so a caller that has no table generator, no `grammar!` and no build
//! step can still parse a file through the same [`Workspace::parse`] everything
//! else uses.
//!
//! The S-expressions pinned here are the ones `src/deep/tests.rs` pins for the
//! same inputs, so the hand-written path is checked against the trees the old
//! runtime produced rather than against a fresh run of the same code.

use deep_rt::json::{JSON, ARRAY, BAD_STRING, DOCUMENT, OBJECT, PAIR};
use deep_rt::syntax::{Class, Language, ERROR};
use deep_rt::tree::{HAS_ERROR, MISSING, TOKEN, UNCLOSED};
use deep_rt::{Budget, Outcome, Workspace};

fn parse(src: &str) -> (Workspace, Outcome) {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src.as_bytes(), &JSON, &Budget::default()).unwrap();
    check(&workspace, src.as_bytes(), &JSON);
    (workspace, outcome)
}

fn sexp(src: &str) -> String {
    parse(src).0.tree.sexp(&JSON, true)
}

/// The invariants every parse must keep, whatever the input: lossless tokens, a
/// root that spans the file, nesting ranges, and flags that say exactly what is
/// below them.
fn check(workspace: &Workspace, src: &[u8], language: &Language) {
    let tokens = &workspace.tokens;
    let nodes = &workspace.tree.nodes;
    let mut joined = Vec::with_capacity(src.len());
    for at in 0..tokens.len() {
        assert!(tokens.start(at) < tokens.end[at], "empty token {at}");
        joined.extend_from_slice(tokens.text(src, at));
    }
    assert_eq!(joined, src, "tokens are not lossless");
    assert_eq!(nodes[0].kind, language.root);
    assert_eq!((nodes[0].start, nodes[0].end), (0, src.len() as u32));
    assert_eq!(nodes[0].subtree_end as usize, nodes.len());
    let mut open: Vec<usize> = Vec::new();
    let mut previous_end: Vec<u32> = vec![0];
    for (index, node) in nodes.iter().enumerate() {
        while open.last().is_some_and(|&at| nodes[at].subtree_end as usize <= index) {
            open.pop();
            previous_end.pop();
        }
        assert!(node.start <= node.end, "node {index} runs backwards");
        assert!(node.subtree_end as usize > index && node.subtree_end as usize <= nodes.len());
        if let Some(&parent) = open.last() {
            let parent = &nodes[parent];
            assert!(parent.start <= node.start && node.end <= parent.end, "node {index} escapes its parent");
            let previous = previous_end.last_mut().unwrap();
            assert!(*previous <= node.start, "node {index} overlaps its previous sibling");
            *previous = node.end;
        }
        open.push(index);
        previous_end.push(node.start);
    }
    for index in (0..nodes.len()).rev() {
        let node = &nodes[index];
        let mut want = 0;
        if node.kind == ERROR || (node.flags & TOKEN != 0 && language.class(node.kind) == Class::BadToken) {
            want |= HAS_ERROR;
        }
        let mut child = index + 1;
        while child < node.subtree_end as usize {
            want |= nodes[child].flags & HAS_ERROR;
            child = nodes[child].subtree_end as usize;
        }
        assert_eq!(node.flags & HAS_ERROR, want, "flags of node {index}");
    }
}

#[test]
fn the_reference_grammar_needs_no_tables() {
    // The hand-written path: a lexer, a kind table and a function. Nothing to
    // generate and nothing to link.
    assert!(JSON.tables.is_none());
    assert_eq!((JSON.name, JSON.name(JSON.root)), ("json", "document"));
    assert_eq!(JSON.kinds.len(), 17);
    assert_eq!(JSON.name(OBJECT), "object");
    assert_eq!(JSON.name(ARRAY), "array");
    assert_eq!(JSON.name(PAIR), "pair");
    assert_eq!(JSON.class(BAD_STRING), Class::BadToken);
}

#[test]
fn a_document_is_a_sequence_of_values() {
    assert_eq!(sexp(""), "(document)");
    assert_eq!(sexp("1"), "(document number)");
    assert_eq!(sexp(r#"{"a":1}"#), "(document (object (pair string number)))");
    assert_eq!(sexp(r#"[1, 2, 3]"#), "(document (array number number number))");
    assert_eq!(sexp(r#"{"a":[1,{"b":null},true]}"#), "(document (object (pair string (array number (object (pair string null)) true))))");
    assert_eq!(sexp("1 2 3"), "(document number number number)");
    // Comments are trivia, as in tree-sitter-json.
    assert_eq!(sexp("// a note\n[1] /* tail */"), "(document (array number))");
    // Deep nesting is a tree, not a recursion of the parser over the process stack.
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let parse = parse(&deep);
    assert!(parse.1.complete());
    assert_eq!(parse.0.tree.depth(), 201);
}

/// The trees Tree-sitter v0.26.12 made for the five #5910 inputs, which are what
/// keep a broken member from taking its siblings or the root with it.
const FIXTURES_5910: &[(&str, &str)] = &[
    (r#"{"a":{"b" "c"}}"#, "(document (object (pair string (object (ERROR string string)))))"),
    (r#"{"one":{"bar" "baz"},"two":"bar"}"#, "(document (object (pair string (object (ERROR string string))) (pair string string)))"),
    (r#"{"b" "c"}"#, "(document (object (ERROR string string)))"),
    ("[1, 2 3, 4]", "(document (array number number (ERROR number) number))"),
    (
        r#"{"k":[{"a":1},{"b" "c"},{"d":2}],"z":{"ok":true}}"#,
        "(document (object (pair string (array (object (pair string number)) (object (ERROR string string)) (object (pair string number)))) (pair string (object (pair string true)))))",
    ),
];

#[test]
fn every_5910_fixture_keeps_its_root_and_confines_the_error() {
    for (src, want) in FIXTURES_5910 {
        let (workspace, outcome) = parse(src);
        assert_eq!(workspace.tree.root().kind, DOCUMENT, "{src}");
        assert_eq!(workspace.tree.sexp(&JSON, true), *want, "{src}");
        assert!(outcome.complete(), "{src}: {outcome:?}");
    }
}

#[test]
fn an_error_stays_inside_the_node_it_broke() {
    // An unterminated string is a bad token, so the leaf carries the error.
    let (workspace, _) = parse("[\"open\n]");
    assert_eq!(workspace.tree.nodes[0].kind, DOCUMENT);
    assert!(workspace.tree.nodes[0].flags & HAS_ERROR != 0, "the root's flag must say an error is below");
    // A bare word is one too.
    let (workspace, _) = parse("{\"a\": nope}");
    assert!(workspace.tree.nodes.iter().any(|node| node.flags & TOKEN != 0 && node.flags & HAS_ERROR != 0));
}

#[test]
fn a_missing_closer_is_flagged_and_the_root_survives() {
    let (workspace, outcome) = parse(r#"{"a":[1,2}"#);
    assert!(outcome.complete());
    assert_eq!(workspace.tree.root().kind, DOCUMENT);
    assert!(workspace.pairs.unclosed > 0, "the unclosed group is counted");
    assert!(workspace.tree.nodes.iter().any(|node| node.flags & UNCLOSED != 0));
}

#[test]
fn a_missing_value_is_an_empty_error_flagged_missing() {
    let (workspace, _) = parse(r#"{"a":}"#);
    assert!(
        workspace.tree.nodes.iter().any(|node| node.kind == ERROR && node.flags & MISSING != 0),
        "a missing value becomes an empty MISSING error"
    );
    assert_eq!(workspace.tree.root().kind, DOCUMENT);
}

#[test]
fn the_step_budget_stops_the_parse_and_still_leaves_a_tree() {
    let budget = Budget { steps_per_token: 0, base_steps: 0, ..Budget::default() };
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(b"[[[1,2,3],4],5]", &JSON, &budget).unwrap();
    assert!(!outcome.complete(), "an exhausted budget is never complete");
    assert!(outcome.exhausted.is_some());
    assert_eq!(workspace.tree.root().kind, DOCUMENT);
    assert_eq!((workspace.tree.root().start, workspace.tree.root().end), (0, 15));
}

#[test]
fn a_zero_clock_budget_degrades_instead_of_panicking() {
    let budget = Budget { time: Some(std::time::Duration::ZERO), ..Budget::default() };
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(b"[1]", &JSON, &budget).unwrap();
    assert!(outcome.exhausted.is_some());
    assert_eq!(workspace.tree.root().kind, DOCUMENT);
}
