//! The template-literal gap in the JavaScript and TypeScript grammars, one
//! test per construct the one-pass lexer could name and the CST could not:
//!
//! - `` `abc ${f(x)} def` ``: the substitution is a node, and the call in it is
//!   a `call_expression` with its callee and argument;
//! - `` `${`inner ${g(y)}`}` ``: a template inside a substitution is a template
//!   of its own, and both expressions are in the tree;
//! - ``tag`x ${h(z)}` ``: the literal is the tagged call's argument, as the
//!   grammar already shapes a call, and the substitution inside it is real;
//! - `` `${(x as Foo).bar()}` `` in TypeScript: a type assertion in a
//!   substitution is an ordinary expression over an ordinary call;
//! - `` `plain` ``: a literal with no substitution stays one template node;
//! - a template that cannot close is error nodes inside its statement, never a
//!   panic or a lost file.
//!
//! Each test reads the shipped fixture, so the fixture and the assertion cannot
//! drift apart, and asserts node kinds and byte spans rather than "the file
//! parsed".

use std::fs;
use std::path::{Path, PathBuf};

use deep_grammars::deep::syntax::{Language, ERROR};
use deep_grammars::deep::tree::{Node, HAS_ERROR, TOKEN};
use deep_grammars::deep::{Budget, Workspace};
use deep_grammars::javascript::{JAVASCRIPT, TYPESCRIPT};

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

    /// Every node of `kind` below the root, as indices.
    fn of_kind(&self, kind: &str) -> Vec<usize> {
        (1..self.workspace.tree.nodes.len()).filter(|&index| !self.is_token(index) && self.kind(index) == kind).collect()
    }

    /// The one node of `kind` whose text is `text`.
    fn find(&self, kind: &str, text: &str) -> usize {
        self.of_kind(kind)
            .into_iter()
            .find(|&index| self.text(index) == text)
            .unwrap_or_else(|| panic!("no {kind} node with text {text:?}:\n{}", self.workspace.tree.sexp(self.language, true)))
    }

    /// The offset of `needle` in the fixture, as the span's start.
    fn at(&self, needle: &str) -> u32 {
        let at = self.src.windows(needle.len()).position(|window| window == needle.as_bytes());
        at.unwrap_or_else(|| panic!("{needle:?} is in the fixture")) as u32
    }

    /// The span `needle` occupies: its offset and its length.
    fn bytes_of(&self, needle: &str) -> (u32, u32) {
        let start = self.at(needle);
        (start, start + needle.len() as u32)
    }

    /// Direct child nodes of `index`, tokens included.
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

fn parse(folder: &str, name: &str, language: &'static Language) -> Parsed {
    let path = fixture(folder, name);
    let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    let mut workspace = Workspace::default();
    workspace.parse(&src, language, &Budget::default()).unwrap_or_else(|_| panic!("{path:?}: too large"));
    Parsed { src, workspace, language }
}

#[test]
fn a_substitution_is_a_node_and_the_call_in_it_is_a_call() {
    let parsed = parse("javascript", "template_substitutions.js", &JAVASCRIPT);
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&JAVASCRIPT, true));

    // `const message = `hi ${format(user)} and ${user.name}`;`: the literal is
    // one `template_string` node spanning both substitutions, and each
    // substitution is a node whose span is exactly its own `${...}`.
    let literal = "`hi ${format(user)} and ${user.name}`";
    let message = parsed.find("template_string", literal);
    assert_eq!(parsed.span(message), parsed.bytes_of(literal), "the literal's bytes");

    let field = "${format(user)}";
    let substitution = parsed.find("template_substitution", field);
    assert_eq!(parsed.span(substitution), parsed.bytes_of(field), "the substitution's bytes");

    // The braces keep their own tokens: the group's first and last children.
    let kids = parsed.children(substitution);
    assert_eq!(parsed.kind(kids[0]), "template_substitution_open", "the substitution's opening brace");
    assert_eq!(parsed.kind(kids[kids.len() - 1]), "template_substitution_close", "the brace that matches it");

    // The expression between them is ordinary code, so the call is a
    // `call_expression`, not part of a span.
    let call = parsed.find("call_expression", "format(user)");
    assert!((parsed.span(substitution).0..parsed.span(substitution).1).contains(&parsed.span(call).0));
    let parts = parsed.children(call);
    let callee = parts.iter().copied().find(|&child| parsed.kind(child) == "identifier").expect("the callee");
    assert_eq!(parsed.text(callee), "format");
    assert!(parts.iter().any(|&child| parsed.kind(child) == "arguments" && parsed.text(child) == "(user)"), "the argument list");

    // The second substitution is a member expression, named as usual.
    let second = parsed.find("template_substitution", "${user.name}");
    assert!(parsed.children(second).iter().any(|&child| parsed.kind(child) == "member_expression" && parsed.text(child) == "user.name"));

    // `${ { a: 1 }.a }`: the object's brace is not the substitution's, so the
    // substitution closes at its own brace and the object keeps its span.
    let braced = parsed.find("template_substitution", "${ { a: 1 }.a }");
    let object = parsed.of_kind("object").into_iter().find(|&node| parsed.text(node) == "{ a: 1 }").expect("the object literal");
    assert!((parsed.span(braced).0..parsed.span(braced).1).contains(&parsed.span(object).0), "the object is inside the substitution");
    assert!(parsed.span(object).1 < parsed.span(braced).1, "and the substitution closes after it");
}

#[test]
fn a_template_inside_a_substitution_is_a_template_of_its_own() {
    let parsed = parse("javascript", "template_substitutions.js", &JAVASCRIPT);
    let outer = parsed.find("template_string", "`${`inner ${g(y)}`}`");
    let inner = parsed.find("template_string", "`inner ${g(y)}`");
    assert!(parsed.span(outer) != parsed.span(inner), "two literals, two spans");
    // The inner literal is inside the outer substitution, and the outer
    // substitution closes after it.
    let substitution = parsed.children(outer).into_iter().find(|&child| parsed.kind(child) == "template_substitution").expect("the outer substitution");
    assert!((parsed.span(substitution).0..parsed.span(substitution).1).contains(&parsed.span(inner).0));
    assert!(parsed.span(substitution).1 > parsed.span(inner).1, "the outer closer comes after the inner backtick");
    // Both expressions are in the tree: the inner call is a call of its own.
    assert_eq!(parsed.text(parsed.find("call_expression", "g(y)")), "g(y)");
    assert_eq!(parsed.text(parsed.find("template_substitution", "${g(y)}")), "${g(y)}");
}

#[test]
fn a_tagged_template_keeps_the_literal_as_the_call_argument() {
    let parsed = parse("javascript", "template_substitutions.js", &JAVASCRIPT);
    let call = parsed.find("call_expression", "tag`x ${h(z)}`");
    let parts = parsed.children(call);
    // The tag is the callee-adjacent node, exactly as `tag(x)` shapes it.
    assert_eq!(parsed.kind(parts[0]), "identifier");
    assert_eq!(parsed.text(parts[0]), "tag");
    assert_eq!(parsed.kind(parts[1]), "template_string");
    assert_eq!(parsed.span(parts[1]), parsed.bytes_of("`x ${h(z)}`"));
    // And the substitution inside the literal is real code again.
    assert_eq!(parsed.text(parsed.find("call_expression", "h(z)")), "h(z)");
}

#[test]
fn a_template_without_a_substitution_is_one_literal() {
    let parsed = parse("javascript", "template_substitutions.js", &JAVASCRIPT);
    let plain = parsed.find("template_string", "`no substitutions`");
    assert_eq!(parsed.span(plain), parsed.bytes_of("`no substitutions`"), "the whole literal");
    assert!(parsed.of_kind("template_substitution").iter().all(|&node| !(parsed.span(plain).0..parsed.span(plain).1).contains(&parsed.span(node).0)), "no substitution inside it");
    let parts = parsed.children(plain);
    assert_eq!(parsed.kind(parts[0]), "template_start", "the opening backtick");
    assert_eq!(parsed.kind(parts[1]), "string_fragment", "the text between the backticks");
    assert_eq!(parsed.text(parts[1]), "no substitutions");
    assert_eq!(parsed.kind(parts[2]), "template_end", "the closing backtick");
}

#[test]
fn a_type_assertion_in_a_substitution_is_an_ordinary_expression() {
    let parsed = parse("typescript", "template_substitutions.ts", &TYPESCRIPT);
    assert_eq!(parsed.error_nodes(), 0, "{}", parsed.workspace.tree.sexp(&TYPESCRIPT, true));

    // `${(x as Foo).bar()}`: the assertion binds `(x as Foo)` and the call on
    // it is a call expression, all inside a substitution of its own.
    let substitution = parsed.find("template_substitution", "${(x as Foo).bar()}");
    let call = parsed.find("call_expression", "(x as Foo).bar()");
    assert!((parsed.span(substitution).0..parsed.span(substitution).1).contains(&parsed.span(call).0));
    let assertion = parsed.find("as_expression", "x as Foo");
    assert!((parsed.span(call).0..parsed.span(call).1).contains(&parsed.span(assertion).0), "the assertion is the call's receiver");

    // A template inside a substitution parses in TypeScript too.
    assert_eq!(parsed.text(parsed.find("template_string", "`${count as number}`")), "`${count as number}`");
    assert!(parsed.of_kind("as_expression").iter().any(|&node| parsed.text(node) == "count as number"));
}

#[test]
fn a_template_that_cannot_close_is_an_error_inside_its_statement() {
    let parsed = parse("malformed", "unclosed_template.js", &JAVASCRIPT);
    // The tree still covers the file and the error is inside the root, so the
    // statements around the broken one keep their place.
    assert_eq!(parsed.language.name(parsed.workspace.tree.root().kind), "program");
    assert!(parsed.error_nodes() > 0, "the broken template is reported:\n{}", parsed.workspace.tree.sexp(&JAVASCRIPT, true));
    assert_eq!(parsed.kind(0), "program", "the root survived");
}
