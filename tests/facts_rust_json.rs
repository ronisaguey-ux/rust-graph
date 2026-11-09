//! Rust and JSON facts, pinned to the records the walk writes.
//!
//! The differential gate (`tests/deep_facts.rs`) covers the languages whose
//! fixtures ship with a lexer row. Rust has neither, so its table is pinned
//! here against a source shaped like real code — a struct, a trait, an inherent
//! method, a trait impl, a call with a receiver, and a `use` with an alias —
//! and every record is asserted rather than counted: a later arm that reorders
//! a call, drops an owner or renames a key fails here.
//!
//! JSON is the other half of the decision: a data file has no definitions, no
//! calls and no imports, so the record is the module symbol and nothing else.
//! That is why `lang_for` still answers `None` for `JSON` — the CST would add
//! nothing the lexer path does not already have — and the test says so.

#![cfg(all(feature = "deep", feature = "parser"))]

#[path = "../src/facts.rs"]
mod facts;
#[path = "../src/json.rs"]
mod json;
#[path = "../src/deep/mod.rs"]
mod deep;
#[path = "../src/deep_integration.rs"]
mod deep_integration;
#[path = "../src/deep_facts.rs"]
mod deep_facts;

use deep_integration::Lang;
use facts::{Facts, Symbol, IMPORT_MODULE};

const RUST: &str = r#"use std::collections::HashMap as Map;
use std::fmt;

pub struct Widget {
    pub name: String,
}

pub trait Draw {
    fn draw(&self, ctx: &mut Context);
}

impl Draw for Widget {
    fn draw(&self, ctx: &mut Context) {
        ctx.begin(self.name.as_str());
        render(ctx, self);
    }
}

impl Widget {
    pub fn new(name: &str) -> Self {
        Widget { name: name.to_string() }
    }
}
"#;

const JSON: &str = r#"{
  "name": "example",
  "version": "1.0.0",
  "dependencies": {
    "serde": "1.0"
  },
  "features": ["deep", "parser"]
}
"#;

fn extract(src: &str, lang: Lang) -> Facts {
    let bytes = src.as_bytes();
    let cst = deep_integration::parse_file(bytes, lang).expect("a deep parse");
    assert_eq!(cst.errors, 0, "the pinned source must parse without recovery");
    deep_facts::extract(bytes, &cst, lang)
}

/// `(key, name, owner, kind, line, end_line)` for every symbol, in walk order.
fn symbols(facts: &Facts) -> Vec<(String, String, String, u8, u32, u32)> {
    facts
        .symbols
        .iter()
        .map(|symbol| {
            (
                symbol.key.clone(),
                symbol.name.clone(),
                symbol.owner.clone(),
                symbol.kind,
                symbol.line,
                symbol.end_line,
            )
        })
        .collect()
}

fn calls(symbol: &Symbol) -> Vec<(String, u32)> {
    symbol.calls.iter().map(|call| (call.path.clone(), call.line)).collect()
}

#[test]
fn rust_a_struct_a_trait_an_impl_and_a_use_are_every_record_they_should_be() {
    let facts = extract(RUST, Lang::Rust);
    assert_eq!(facts.tag, "RS");
    assert_eq!(facts.lines, 24);

    // The module, the struct, the trait and its signature, the trait impl, the
    // inherent impl and the method each own the key a reader resolves on. An
    // `impl` is named after the type it implements, so the trait impl and the
    // inherent impl share `Widget` and the second one is disambiguated by line.
    assert_eq!(
        symbols(&facts),
        vec![
            ("<module>".into(), "".into(), "".into(), facts::KIND_MODULE, 1, 24),
            ("Widget".into(), "Widget".into(), "".into(), facts::KIND_CLASS, 4, 6),
            ("Draw".into(), "Draw".into(), "".into(), facts::KIND_CLASS, 8, 10),
            ("Draw.draw".into(), "draw".into(), "Draw".into(), facts::KIND_FUNCTION, 9, 9),
            ("Widget@12".into(), "Widget".into(), "".into(), facts::KIND_CLASS, 12, 17),
            ("Widget.draw".into(), "draw".into(), "Widget".into(), facts::KIND_FUNCTION, 13, 16),
            ("Widget@19".into(), "Widget".into(), "".into(), facts::KIND_CLASS, 19, 23),
            ("Widget.new".into(), "new".into(), "Widget".into(), facts::KIND_FUNCTION, 20, 22),
        ]
    );

    // `impl Draw for Widget` records the trait as a base.
    assert_eq!(facts.symbols[4].bases, vec!["Draw".to_string()]);

    // `self` is not a parameter and the receiver type is not a param either;
    // the names are the binders.
    assert_eq!(facts.symbols[3].params, vec!["ctx".to_string()]);
    assert_eq!(facts.symbols[5].params, vec!["ctx".to_string()]);
    assert_eq!(facts.symbols[7].params, vec!["name".to_string()]);

    // Calls carry receiver, name and the line the name is written on.
    assert_eq!(
        calls(&facts.symbols[5]),
        vec![
            ("ctx.begin".to_string(), 14),
            ("self.name.as_str".to_string(), 14),
            ("render".to_string(), 15),
        ]
    );
    assert_eq!(calls(&facts.symbols[7]), vec![("name.to_string".to_string(), 21)]);

    // The flow record: a statement that is a call is an `x` event, and the
    // receiver a call is written on is part of the form.
    assert_eq!(
        facts.symbols[5].events,
        r#"["x",14,["c",0,["n","ctx"],[["c",1,["n","self.name"],[]]]]],["x",15,["c",2,0,[["n","ctx"]]]]"#
    );
    assert_eq!(
        facts.symbols[7].events,
        r#"["x",21,["m",[["n","Widget"],["m",[["m",[["n","name"],["c",0,["n","name"],[]]]]]]]]]"#
    );

    // `use ... as ...` is one import with the local name; a plain `use` binds
    // its last segment.
    assert_eq!(
        facts
            .imports
            .iter()
            .map(|import| (import.line, import.module.clone(), import.name.clone(), import.alias.clone(), import.kind))
            .collect::<Vec<_>>(),
        vec![
            (1, "std.collections.HashMap".to_string(), String::new(), "Map".to_string(), IMPORT_MODULE),
            (2, "std.fmt".to_string(), String::new(), "fmt".to_string(), IMPORT_MODULE),
        ]
    );

    // No typed variables: `Widget { .. }` is a struct literal, not a `new`.
    assert!(facts.vars.is_empty());
}

#[test]
fn rust_is_eligible_and_json_is_not() {
    assert_eq!(deep_facts::lang_for("RS"), Some(Lang::Rust));
    // A data file has no symbols, calls or imports: the record would be the
    // module symbol and nothing more, so the lexer path stays the choice.
    assert_eq!(deep_facts::lang_for("JSON"), None);
}

#[test]
fn json_a_data_file_is_only_the_module_symbol() {
    let facts = extract(JSON, Lang::Json);
    assert_eq!(facts.tag, "JSON");
    assert_eq!(facts.lines, 9);

    // The only symbol is the module the walk always opens; no key, no owner,
    // no parameters, no events. A config has no definitions to name.
    assert_eq!(symbols(&facts), vec![("<module>".into(), "".into(), "".into(), facts::KIND_MODULE, 1, 9)]);
    let module = &facts.symbols[0];
    assert!(module.events.is_empty());
    assert!(module.calls.is_empty());
    assert!(module.params.is_empty());
    assert!(module.bases.is_empty());

    assert!(facts.imports.is_empty());
    assert!(facts.vars.is_empty());
}
