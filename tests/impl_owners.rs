//! P5-12: an impl block records the owner the way the lexer does.
//!
//! A method's edges hang off the class frame it sits in, so the frame has to be
//! the same type in both readers: an `impl Trait for Type` block is named
//! `Type`, never the trait and never the crate the type came from. The corpus
//! check is `tools/owner_report.py`, which compares the two symbol tables file
//! by file; these rows are the shapes it must agree on, small enough to name the
//! form when one breaks.
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
#[path = "../src/sha256.rs"]
mod sha256;
#[path = "../src/deep/mod.rs"]
mod deep;
#[path = "../src/deep_facts/mod.rs"]
mod deep_facts;
#[path = "../src/pipeline.rs"]
mod pipeline;

use deep_integration::Lang;
use pipeline::gate::*;
use facts::Facts;

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// `(what the row covers, source)`.
const ROWS: &[(&str, &str)] = &[
    (
        "an inherent impl owns its methods",
        "struct Counter { n: u32 }\nimpl Counter {\n    fn bump(&mut self) {\n        self.n += 1;\n    }\n}\n",
    ),
    (
        "a trait impl is named after the type, not the trait",
        "impl Display for Counter {\n    fn fmt(&self) {\n        write()\n    }\n}\n",
    ),
    (
        "a generic trait impl keeps the last path segment as the owner",
        "impl<T> RequestExt for tonic::Request<T> {\n    fn set_default_timeout(&mut self) {\n        work()\n    }\n}\n",
    ),
    (
        "an impl with a where clause owns its methods",
        "impl<'a, T: Clone> Store<'a> for Cache<T>\nwhere\n    T: Send,\n{\n    fn get(&self) {\n        read()\n    }\n}\n",
    ),
    (
        "an unsafe impl names the type",
        "unsafe impl<T: Send> Send for Cell2<T> {\n    fn send(&self) {\n        go()\n    }\n}\n",
    ),
    (
        "an impl for a reference type is named after the type",
        "impl<'a> Reader for &'a Buffer {\n    fn read(&self) {\n        fill()\n    }\n}\n",
    ),
    (
        "an impl inside a module keeps its own frame",
        "mod inner {\n    impl Worker {\n        fn run(&self) {\n            poll()\n        }\n    }\n}\n",
    ),
];

/// `(owner, symbol name, kind)` for every symbol that has an owner.
fn owned(facts: &Facts) -> Vec<(String, String, u8)> {
    let mut out: Vec<(String, String, u8)> = facts
        .symbols
        .iter()
        .filter(|symbol| !symbol.owner.is_empty())
        .map(|symbol| (symbol.owner.clone(), symbol.name.clone(), symbol.kind))
        .collect();
    out.sort();
    out
}

#[test]
fn every_impl_shape_agrees_with_the_lexer_about_its_owner() {
    let mut problems = Vec::new();
    for (what, source) in ROWS {
        let lexer = owned(&lexer_facts(source));
        let deep = owned(&deep_facts(source));
        if lexer != deep {
            problems.push(format!("{what}: lexer {lexer:?} cst {deep:?}"));
        }
        if lexer.is_empty() {
            problems.push(format!("{what}: neither reader owned the symbol"));
        }
        let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
        if verdict.lost() {
            problems.push(format!("{what}: {verdict:?}"));
        }
    }
    assert!(problems.is_empty(), "the two readers disagree about who owns a method:\n{}", problems.join("\n"));
}

#[test]
fn a_trait_impl_is_named_after_the_type_and_carries_the_trait_as_a_base() {
    // `impl Display for Counter` opens the class `Counter`, and `Display` is the
    // base. Read the other way round every method edge hung off a trait that has
    // no body in the file.
    let source = "impl Display for Counter {\n    fn fmt(&self) {\n        write()\n    }\n}\n";
    for (reader, facts) in [("lexer", lexer_facts(source)), ("cst", deep_facts(source))] {
        let class = facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == facts::KIND_CLASS)
            .unwrap_or_else(|| panic!("{reader}: no class symbol"));
        assert_eq!(class.name, "Counter", "{reader}: the impl is named after its trait");
        assert_eq!(class.bases, vec!["Display".to_string()], "{reader}: the trait is not the base");
    }
}
