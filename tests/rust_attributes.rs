//! P5-13: an attribute is layout, not a fact.
//!
//! `#[derive(Debug, Clone)]`, `#[serde(rename = "value")]` and `#[tokio::main]`
//! decorate the item under them. The item still records its name, its
//! parameters and its owner, and neither the attribute's path nor its arguments
//! become a symbol, a call or an event — the CST writes none of them, so a lexer
//! that read them would be inventing syntax.
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
use facts::Facts;
use pipeline::gate::*;

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// `(what the attribute decorates, source, the symbol it must leave behind)`.
const ROWS: &[(&str, &str, &str, &[&str])] = &[
    (
        "a plain attribute over a function",
        "#[inline]\npub fn simple(a: u32, b: &str) -> u32 {\n    a\n}\n",
        "simple",
        &["a", "b"],
    ),
    (
        "a derive over a struct",
        "#[derive(Debug, Clone)]\npub struct Point {\n    pub x: i32,\n}\n",
        "Point",
        &[],
    ),
    (
        "an attribute with arguments over a function",
        "#[serde(rename = \"value\")]\npub fn renamed(v: u64) -> u64 {\n    v\n}\n",
        "renamed",
        &["v"],
    ),
    (
        "a path attribute over an async function",
        "#[tokio::main]\nasync fn main2() {\n    start();\n}\n",
        "main2",
        &[],
    ),
    (
        "stacked attributes, one of them a doc",
        "#[allow(dead_code)]\n#[doc = \"a doc\"]\nfn two(n: usize, m: u8) -> usize {\n    n\n}\n",
        "two",
        &["n", "m"],
    ),
    (
        "an attribute inside a decorated module",
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn check(x: i32) {\n        run(x);\n    }\n}\n",
        "check",
        &["x"],
    ),
    (
        "an attribute over a method in an impl",
        "impl Counter {\n    #[inline]\n    fn bump(&mut self, by: u32) {\n        self.n += by;\n    }\n}\n",
        "bump",
        &["by"],
    ),
];

/// The attribute words that must never appear as a fact of their own.
const ATTRIBUTE_WORDS: &[&str] = &[
    "inline", "derive", "Debug", "Clone", "serde", "rename", "value", "tokio", "main", "allow", "doc",
    "dead_code", "cfg", "test", "unused", "rustfmt", "skip",
];

fn decorated(facts: &Facts, name: &str) -> Option<(String, u8, Vec<String>)> {
    facts
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .map(|symbol| (symbol.owner.clone(), symbol.kind, symbol.params.clone()))
}

#[test]
fn a_decorated_item_still_records_its_name_and_parameters() {
    let mut problems = Vec::new();
    for (what, source, name, params) in ROWS {
        for (reader, facts) in [("lexer", lexer_facts(source)), ("cst", deep_facts(source))] {
            match decorated(&facts, name) {
                None => problems.push(format!("{what} [{reader}]: no symbol `{name}`")),
                Some((_owner, _kind, found)) => {
                    if found != *params {
                        problems.push(format!("{what} [{reader}]: `{name}` params {found:?}, want {params:?}"));
                    }
                }
            }
            for symbol in &facts.symbols {
                if ATTRIBUTE_WORDS.contains(&symbol.name.as_str()) {
                    problems.push(format!("{what} [{reader}]: the attribute became a symbol `{}`", symbol.name));
                }
                for call in &symbol.calls {
                    let word = call.path.rsplit('.').next().unwrap_or(&call.path);
                    if ATTRIBUTE_WORDS.contains(&word) {
                        problems.push(format!("{what} [{reader}]: the attribute became a call `{}`", call.path));
                    }
                }
            }
            let verdict = pipeline::loses(&facts, &deep_facts(source));
            if reader == "lexer" && verdict.lost() {
                problems.push(format!("{what}: {verdict:?}"));
            }
        }
    }
    assert!(problems.is_empty(), "an attribute was read as syntax:\n{}", problems.join("\n"));
}

#[test]
fn an_attribute_decorates_without_moving_the_owner() {
    // The method's owner is the type, not the attribute in front of it.
    let source = "impl Counter {\n    #[inline]\n    fn bump(&mut self, by: u32) {\n        self.n += by;\n    }\n}\n";
    for (reader, facts) in [("lexer", lexer_facts(source)), ("cst", deep_facts(source))] {
        let (owner, kind, params) = decorated(&facts, "bump").unwrap_or_else(|| panic!("{reader}: no `bump`"));
        assert_eq!(owner, "Counter", "{reader}: the decorated method lost its owner");
        assert_eq!(kind, facts::KIND_FUNCTION, "{reader}: the method is not a function");
        assert_eq!(params, vec!["by".to_string()], "{reader}: the parameters are wrong");
    }
}
