//! A call whose receiver is itself a call: `TaintAnalyzer(...).run()`.
//!
//! The CST records two facts for `A().run()`: the constructor call `A`, and the
//! method call `.run`, whose receiver is that constructor's result. The lexer
//! instead pastes the two into the single path `A().run`, which the resolver
//! ties to `A.run`; the CST's pair is truth, so the resolver ties it the same
//! way without a glued path. The link between the two facts is not the order of
//! `calls` (in `TaintAnalyzer(a, b(), c).run()` an argument sits between them)
//! but the receiver slot of the method call's flow event, which the walk fills.
//!
//! Run with `cargo test --features "deep parser" --test resolved_receivers`.

#![cfg(all(feature = "deep", feature = "parser"))]

#[path = "../src/facts.rs"]
mod facts;
#[path = "../src/json.rs"]
mod json;
#[path = "../src/langs.rs"]
mod langs;
#[path = "../src/lexer.rs"]
mod lexer;
#[path = "../src/expr.rs"]
mod expr;
#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/index.rs"]
mod index;
#[path = "../src/resolve.rs"]
mod resolve;
#[path = "../src/deep/mod.rs"]
mod deep;
#[path = "../src/deep_integration.rs"]
mod deep_integration;
#[path = "../src/deep_facts.rs"]
mod deep_facts;

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_rust-graph");

/// Two direct receiver calls and one chain: `A().run()`,
/// `ApplicationService(x).handle()`, `A().chain().run()`.
const FIXTURE: &str = "\
class A:
    def run(self):
        return 1

    def chain(self):
        return A()


class ApplicationService:
    def __init__(self, client):
        self.client = client

    def handle(self):
        return 1


def use():
    A().run()
    ApplicationService(None).handle()
    A().chain().run()
";

/// Every shape a receiver call can take: a constructor with arguments, a
/// chain, a bare function whose return names a class, a method on `self`, a
/// method on a variable, an import, and the three shapes that stay undecided.
const SHAPES: &str = "\
class A:
    def run(self):
        return 1

    def chain(self):
        return A()

    def make(self):
        return A()

    def make_b(self):
        return B()


class B:
    def go(self):
        return 2


class ApplicationService:
    def __init__(self, client):
        self.client = client

    def handle(self):
        return 1

    def parser(self):
        return Parser()


class Parser:
    def parse(self):
        return 3


class Holder:
    def wrap(self):
        return A()

    def use_self(self):
        return self.wrap().run()


def make_a():
    return A()


def make_union(flag):
    if flag:
        return A()
    return B()


def use():
    A().run()
    ApplicationService(None).handle()
    A().chain().run()
    make_a().run()
    A().make().go()
    A().make_b().go()
    ApplicationService(None).parser().parse()
    made = A()
    made.run()
    svc = ApplicationService(None)
    svc.parser().parse()
    make_union(True).run()
    A().missing().run()
";

const HELPERS: &str = "\
class K:
    def m(self):
        return 1


def make_k():
    return K()
";

const MAIN: &str = "\
from helpers import K, make_k


def use():
    return K().m()


def use_make():
    return make_k().m()
";

/// A return that is itself a method call: `build` returns `self.inner()`, and
/// `inner` names the class. Both hops are decided; the chain closes.
const RETURN_CHAIN: &str = "\
class Q:
    def build(self):
        return self.inner()

    def inner(self):
        return Q()

    def go(self):
        return 4


def use():
    return Q().build().go()
";

fn deep_record(source: &str) -> facts::Facts {
    let lang = deep_integration::Lang::Python;
    let cst = deep_integration::parse_file(source.as_bytes(), lang).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, lang)
}

fn lexer_record(source: &str) -> facts::Facts {
    let language = langs::by_content("py", source.as_bytes()).expect("a lexer row");
    parser::parse(source.as_bytes(), language)
}

fn calls_of<'a>(facts: &'a facts::Facts, name: &str) -> Vec<&'a str> {
    facts
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .expect("the symbol")
        .calls
        .iter()
        .map(|call| call.path.as_str())
        .collect()
}

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str, files: &[(&str, &str)]) -> Tree {
        // A test per shape at a time: two tests building the same shape in
        // parallel must not share, or clear, one directory.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rust-graph-receivers-{}-{name}-{serial}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (file, source) in files {
            fs::write(dir.join(file), source).unwrap();
        }
        Tree(dir)
    }

    fn chain() -> Tree {
        Tree::new("chain", &[("chain.py", FIXTURE)])
    }

    fn shapes() -> Tree {
        Tree::new("shapes", &[("shapes.py", SHAPES), ("helpers.py", HELPERS), ("main.py", MAIN)])
    }

    fn deep_return() -> Tree {
        Tree::new("deep-return", &[("deepreturn.py", RETURN_CHAIN)])
    }

    fn json(&self, extra: &str) -> String {
        let out: Output = Command::new(BIN).arg("build").arg(&self.0).arg(extra).arg("--stdout").output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }
}

fn edges(json: &str) -> usize {
    json.matches("\"kind\":\"call\"").count()
}

fn has_edge(json: &str, caller: &str, callee: &str, line: u32) -> bool {
    let edge = format!(r#""caller":"{caller}","callee":"{callee}","line":{line}"#);
    json.contains(&edge)
}

/// The facts the CST emits, before resolution: `A` and `.run`, never `A().run`.
#[test]
fn the_cst_keeps_the_constructor_and_the_method_apart() {
    let record = deep_record(FIXTURE);
    assert_eq!(
        calls_of(&record, "use"),
        ["A", ".run", "ApplicationService", ".handle", "A", ".chain", ".run"]
    );
    for path in calls_of(&record, "use") {
        assert!(!path.contains('('), "the CST glued a receiver into {path}");
    }
    // The lexer's record is the glued one: this file must not change it.
    assert_eq!(
        calls_of(&lexer_record(FIXTURE), "use"),
        ["A", "A().run", "ApplicationService", "ApplicationService().handle", "A", "A().chain", ".run"]
    );
}

/// The walk says which call produced each receiver: `.run` is `A()`'s result,
/// `.chain` is `A()`'s, and the chained `.run` is `.chain`'s.
#[test]
fn the_walk_records_which_call_is_the_receiver() {
    let record = deep_record(FIXTURE);
    let symbol = record.symbols.iter().find(|symbol| symbol.name == "use").expect("the symbol");
    let receivers = resolve::receiver_calls(&symbol.events);
    assert_eq!(receivers, [None, Some(0), None, Some(2), None, Some(4), Some(5)], "events {}", symbol.events);
}

/// The resolver ties the pair to the class's method, as the lexer's glued path
/// does, and decides one link the lexer cannot by name alone: `A().chain().run()`.
#[test]
fn the_deep_graph_ties_the_method_to_the_class() {
    let json = Tree::chain().json("--deep");
    for (caller, callee, line) in [
        ("chain.py:use", "chain.py:A.run", 18),
        ("chain.py:use", "chain.py:ApplicationService.handle", 19),
        ("chain.py:use", "chain.py:A.chain", 20),
        // The chained call: `A().chain()` is proven to return `A`.
        ("chain.py:use", "chain.py:A.run", 20),
    ] {
        assert!(has_edge(&json, caller, callee, line), "missing {caller} -> {callee}:{line}\n{json}");
    }
    assert!(json.contains("\"tag\":\"INFERRED\""), "{json}");
    assert!(json.contains("\"strategy\":\"receiver-call\""), "{json}");
}

/// The edge count on this shape must not fall short of the lexer's graph, and
/// the lexer's own record must still tie the glued path it records.
#[test]
fn the_deep_graph_is_not_short_of_the_lexer() {
    let tree = Tree::chain();
    let deep = tree.json("--deep");
    let lexer = tree.json("--lexer");
    assert!(edges(&deep) >= edges(&lexer), "deep {} < lexer {}\n{deep}\n{lexer}", edges(&deep), edges(&lexer));
    // Seven calls, and the chained `.run` is decided for both paths: the
    // receiver the lexer records is a call too, and its owner is the class.
    assert_eq!(edges(&lexer), 8, "{lexer}");
    assert!(has_edge(&lexer, "chain.py:use", "chain.py:A.run", 18), "{lexer}");
}

/// Every decidable shape ties to the same method symbol the one-pass graph
/// names, with the strategy that says how it was read; every undecidable shape
/// gets no edge at all.
#[test]
fn every_decidable_receiver_ties_and_the_rest_stay_open() {
    let json = Tree::shapes().json("--deep");
    for (caller, callee, line, strategy) in [
        // A constructor with an argument, and a chain.
        ("shapes.py:use", "shapes.py:A.run", 55, "receiver-call"),
        ("shapes.py:use", "shapes.py:ApplicationService.handle", 56, "receiver-call"),
        ("shapes.py:use", "shapes.py:A.chain", 57, "receiver-call"),
        ("shapes.py:use", "shapes.py:A.run", 57, "receiver-return"),
        // A bare function whose every return names the same class.
        ("shapes.py:use", "shapes.py:A.run", 58, "receiver-return"),
        // A method whose return names a class, called on a class instance.
        ("shapes.py:use", "shapes.py:A.make", 59, "receiver-call"),
        ("shapes.py:use", "shapes.py:B.go", 60, "receiver-return"),
        // Two hops: `ApplicationService.parser()` returns `Parser`.
        ("shapes.py:use", "shapes.py:ApplicationService.parser", 61, "receiver-call"),
        ("shapes.py:use", "shapes.py:Parser.parse", 61, "receiver-return"),
        // A method on a variable, then a call on its result.
        ("shapes.py:use", "shapes.py:ApplicationService.parser", 65, "typed-variable"),
        ("shapes.py:use", "shapes.py:Parser.parse", 65, "receiver-return"),
        // The owner comes from the method's own row, not from the caller.
        ("shapes.py:Holder.use_self", "shapes.py:Holder.wrap", 41, "same-class"),
        ("shapes.py:Holder.use_self", "shapes.py:A.run", 41, "receiver-return"),
        // Across an import: `K` is `helpers.py`'s class.
        ("main.py:use", "helpers.py:K", 5, "import"),
        ("main.py:use", "helpers.py:K.m", 5, "receiver-call"),
    ] {
        let edge = format!(r#""caller":"{caller}","callee":"{callee}","line":{line}"#);
        let found = json.lines().find(|row| row.contains(&edge)).unwrap_or_else(|| panic!("missing {caller} -> {callee}:{line}\n{json}"));
        assert!(
            found.contains(&format!("\"strategy\":\"{strategy}\"")),
            "{caller} -> {callee}:{line} is not {strategy}: {found}"
        );
    }
    // `make_union(True)` returns `A` on one branch and `B` on the other: the
    // tree does not name one class, so `.run` is not decided. The receiver
    // call itself is still a fact.
    assert!(has_edge(&json, "shapes.py:use", "shapes.py:make_union", 66), "{json}");
    // `A()` has no `missing`, so neither `.missing` nor the call on its result
    // is decided.
    assert!(has_edge(&json, "shapes.py:use", "shapes.py:A", 67), "{json}");
    for (callee, line) in [("shapes.py:A.run", 66), ("shapes.py:A.missing", 67), ("shapes.py:A.run", 67)] {
        let edge = format!(r#""caller":"shapes.py:use","callee":"{callee}","line":{line}"#);
        assert!(!json.contains(&edge), "{callee}:{line} must stay open\n{json}");
    }
    // The documented undecidable-in-principle shape: a receiver whose own
    // class the tree never states (`obj` is an untyped parameter).
    assert!(!json.contains("\"callee\":\"shapes.py:B.go\",\"line\":59"), "{json}");
    // A function imported from another file is bound, but its body lives in
    // that file and this pass only holds the facts of the file it resolves:
    // `make_k`'s return class is not readable here, so `.m` stays open.
    assert!(has_edge(&json, "main.py:use_make", "helpers.py:make_k", 9), "{json}");
    assert!(!json.contains("\"callee\":\"helpers.py:K.m\",\"line\":9"), "{json}");
}

/// A function whose return is itself a method call is followed when that
/// method's return names a class, so `Q().build().go()` closes.
#[test]
fn a_return_that_is_a_method_call_is_followed() {
    let json = Tree::deep_return().json("--deep");
    for (caller, callee, line, strategy) in [
        ("deepreturn.py:use", "deepreturn.py:Q.build", 13, "receiver-call"),
        ("deepreturn.py:use", "deepreturn.py:Q.go", 13, "receiver-return"),
    ] {
        assert!(has_edge(&json, caller, callee, line), "missing {caller} -> {callee}:{line}\n{json}");
        let edge = format!(r#""caller":"{caller}","callee":"{callee}","line":{line}"#);
        let found = json.lines().find(|row| row.contains(&edge)).unwrap();
        assert!(found.contains(&format!("\"strategy\":\"{strategy}\"")), "{found}");
    }
}

/// Both paths now decide the same links on the same tree; the deep one does it
/// from the two facts it records, the lexer one from the path it glues.
#[test]
fn both_paths_agree_on_the_shapes_they_can_decide() {
    let tree = Tree::shapes();
    let deep = tree.json("--deep");
    let lexer = tree.json("--lexer");
    let pairs = |json: &str| -> Vec<(String, String, String)> {
        json.lines()
            .filter(|row| row.contains("\"kind\":\"call\""))
            .map(|row| {
                let text = |name: &str| -> String {
                    let at = row.find(&format!("\"{name}\":\"")).unwrap() + name.len() + 4;
                    let rest = &row[at..];
                    rest[..rest.find('"').unwrap()].to_string()
                };
                let at = row.find("\"line\":").unwrap() + 7;
                let rest = &row[at..];
                let end = rest.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(rest.len());
                (text("caller"), text("callee"), rest[..end].to_string())
            })
            .collect()
    };
    let mut deep_pairs = pairs(&deep);
    let mut lexer_pairs = pairs(&lexer);
    deep_pairs.sort();
    lexer_pairs.sort();
    assert_eq!(edges(&deep), edges(&lexer), "deep\n{deep}\nlexer\n{lexer}");
    assert_eq!(deep_pairs, lexer_pairs, "deep\n{deep}\nlexer\n{lexer}");
}

/// The resolver, called on the record itself: each method call is tied to the
/// class its receiver constructs, and the chain is followed through `A.chain`.
#[test]
fn the_resolver_ties_the_receiver_call() {
    struct NoImports;
    impl resolve::ImportSource for NoImports {
        fn imports_of(&mut self, _file: u32) -> std::rc::Rc<Vec<facts::Import>> {
            std::rc::Rc::new(Vec::new())
        }
        fn bases_of(&mut self, _file: u32, _class: &str) -> Vec<String> {
            Vec::new()
        }
    }

    let record = deep_record(FIXTURE);
    let mut index = index::Index::default();
    index.add_file("chain.py", &record);
    index.seal();
    let symbol = record.symbols.iter().find(|symbol| symbol.name == "use").expect("the symbol");
    let mut source = NoImports;
    let mut resolver = resolve::Resolver::new(&index, &mut source);
    let mut tied = Vec::new();
    for (position, call) in symbol.calls.iter().enumerate() {
        let outcome = resolver.call_at(&call.path, symbol, position, 0, &record, resolve::Scope::default());
        if let resolve::Outcome::Bound(bound) = outcome {
            tied.push((call.path.clone(), index.id(&bound.row), bound.strategy));
        }
    }
    assert_eq!(
        tied,
        [
            ("A".to_string(), "chain.py:A".to_string(), "same-file"),
            (".run".to_string(), "chain.py:A.run".to_string(), "receiver-call"),
            ("ApplicationService".to_string(), "chain.py:ApplicationService.__init__".to_string(), "same-file"),
            (".handle".to_string(), "chain.py:ApplicationService.handle".to_string(), "receiver-call"),
            ("A".to_string(), "chain.py:A".to_string(), "same-file"),
            (".chain".to_string(), "chain.py:A.chain".to_string(), "receiver-call"),
            (".run".to_string(), "chain.py:A.run".to_string(), "receiver-return"),
        ]
    );
}
