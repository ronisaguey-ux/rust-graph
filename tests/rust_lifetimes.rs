//! P5-09: lifetime and generics syntax never produces a phantom symbol.
//!
//! `'a` is not a name, and neither is a bound: `<T: Clone + 'a>` says nothing
//! about a symbol called `T`, `Clone` or `'a`, and a reader that writes one has
//! invented a node the source does not have. Every row below is a shape the
//! reader must get through without one, asserted on both readers — the lexer is
//! what the corpus goes through, and the CST is what the lexer is checked against.
//!
//! The rule is stated the way the plan states it: no symbol carries a lifetime,
//! and no symbol is named for a generic parameter or a bound in its own header.
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

use pipeline::gate::*;

use deep_integration::Lang;
use facts::Facts;

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("rs", source.as_bytes()).expect("a Rust row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Rust).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Rust)
}

/// The names a header may mention but must never turn into a symbol: the
/// generic parameters it declares and the bounds it states.
const BOUND_NAMES: &[&str] = &[
    "T", "K", "V", "F", "N", "U", "Clone", "Debug", "Send", "Sync", "Default", "Iterator", "Eq",
    "Sized", "Fn", "Item", "Self",
];

/// `(what the row covers, source, names the header declares that are not symbols)`.
const ROWS: &[(&str, &str)] = &[
    (
        "struct with a lifetime and two bounds",
        "pub struct Handle<'a, T: Clone + Debug> where T: Send {\n    inner: &'a T,\n}\n",
    ),
    (
        "impl header repeating the lifetime and the bounds",
        "impl<'a, T: Clone + Debug> Handle<'a, T>\nwhere\n    T: Send + Sync,\n{\n    pub fn get(&self) -> &'a T {\n        self.inner\n    }\n}\n",
    ),
    (
        "a trait with an associated type bounded by a lifetime",
        "trait Store<'a, K: Eq, V: Default> {\n    type Item<'b>: Iterator<Item = &'b V> where Self: 'b;\n    fn get(&'a self, key: &K) -> Option<&'a V>;\n}\n",
    ),
    (
        "a const generic parameter",
        "struct Buffer<const N: usize> {\n    buf: [u8; N],\n}\nfn take<const N: usize>(arr: [u8; N]) -> usize {\n    N\n}\n",
    ),
    (
        "a higher-ranked bound",
        "fn h<F>(f: F) -> usize\nwhere\n    for<'a> F: Fn(&'a str) -> usize,\n{\n    f(\"x\")\n}\n",
    ),
    (
        "an impl with a ?Sized parameter and an outlives bound",
        "unsafe impl<T: Send + ?Sized> Send for Cell2<T> {}\nfn bounded<T: ?Sized>(x: &T) {}\n",
    ),
    (
        "a trait object with a lifetime bound",
        "fn boxes(v: Box<dyn Fn(&str) -> usize + Send + 'static>) -> usize {\n    0\n}\n",
    ),
    (
        "lifetime labels are not symbols",
        "fn labeled() {\n    'outer: loop {\n        'inner: for i in 0..3 {\n            if i > 1 { break 'outer; }\n            continue 'inner;\n        }\n        break 'outer;\n    }\n}\n",
    ),
    (
        "an enum with a lifetime and a bound",
        "enum E<'a, T: 'a> {\n    A(&'a T),\n    B { inner: &'a T },\n}\n",
    ),
    (
        "a type alias with a where clause",
        "type Alias<'a, T> = &'a T where T: 'a;\n",
    ),
];

/// A symbol name is an identifier, the empty module name, or the reader's own
/// `<anonymous>@N` for a closure — anything else is a fragment of syntax that
/// should have been consumed by the header.
fn valid_name(name: &str) -> bool {
    if name.is_empty() || name == "<module>" {
        return true;
    }
    if let Some(rest) = name.strip_prefix("<anonymous>@") {
        return rest.parse::<u32>().is_ok();
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn phantoms(facts: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    for symbol in &facts.symbols {
        if symbol.name.contains('\'') {
            out.push(format!("lifetime as a {}-name: `{}`", symbol.kind, symbol.name));
        } else if !valid_name(&symbol.name) {
            out.push(format!("not an identifier: `{}`", symbol.name));
        } else if BOUND_NAMES.contains(&symbol.name.as_str()) {
            out.push(format!("a generic parameter or bound as a symbol: `{}`", symbol.name));
        }
        for param in &symbol.params {
            if param.contains('\'') || BOUND_NAMES.contains(&param.as_str()) {
                out.push(format!("a lifetime or bound as a parameter of `{}`: `{param}`", symbol.name));
            }
        }
        // A base may legitimately be a trait — `impl Send for Cell2` — so only a
        // lifetime there is a fragment of syntax rather than a name.
        for base in &symbol.bases {
            if base.contains('\'') || !valid_name(base) {
                out.push(format!("a lifetime as a base of `{}`: `{base}`", symbol.name));
            }
        }
    }
    out
}

#[test]
fn no_reader_names_a_symbol_from_a_lifetime_or_a_bound() {
    let mut problems = Vec::new();
    for (what, source) in ROWS {
        for (reader, facts) in [("lexer", lexer_facts(source)), ("cst", deep_facts(source))] {
            for problem in phantoms(&facts) {
                problems.push(format!("{what} [{reader}]: {problem}"));
            }
        }
    }
    assert!(problems.is_empty(), "a lifetime or a bound became a symbol:\n{}", problems.join("\n"));
}

#[test]
fn the_rows_are_read_and_their_real_names_survive() {
    // A test that asserts "no phantom" passes trivially on no symbols at all.
    // Each row must also yield the name it declares.
    let expected: &[(&str, &str)] = &[
        ("struct with a lifetime and two bounds", "Handle"),
        ("impl header repeating the lifetime and the bounds", "get"),
        ("a trait with an associated type bounded by a lifetime", "Store"),
        ("a const generic parameter", "Buffer"),
        ("a higher-ranked bound", "h"),
        ("an impl with a ?Sized parameter and an outlives bound", "Cell2"),
        ("a trait object with a lifetime bound", "boxes"),
        ("lifetime labels are not symbols", "labeled"),
        ("an enum with a lifetime and a bound", "E"),
    ];
    // `type Alias<'a, T> = ..` declares no symbol in either reader today — the row
    // is here for its header, not for its name.
    let mut problems = Vec::new();
    for (what, name) in expected {
        let row = ROWS.iter().find(|(row, _)| row == what).unwrap_or_else(|| panic!("no row {what}"));
        for (reader, facts) in [("lexer", lexer_facts(row.1)), ("cst", deep_facts(row.1))] {
            if !facts.symbols.iter().any(|symbol| symbol.name == *name) {
                let names: Vec<&str> = facts.symbols.iter().map(|symbol| symbol.name.as_str()).collect();
                problems.push(format!("{what} [{reader}]: no symbol `{name}` in {names:?}"));
            }
        }
    }
    assert!(problems.is_empty(), "a declared name did not survive:\n{}", problems.join("\n"));
}
