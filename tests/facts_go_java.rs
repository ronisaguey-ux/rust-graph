//! Go and Java facts: the record `src/deep_facts.rs` reads off the CST, pinned
//! per language.
//!
//! What is pinned is the part a resolver reads: the tag, the imports with their
//! aliases, the definitions with their owner and parameters, the calls with
//! their receiver paths, the bases a container declares, and the flow events on
//! the lines they are written on. `lang_for` is pinned too: without it a file
//! of either language never takes the CST path at all, which is the whole point
//! of the tables.
//!
//! The sources are shaped like the code they come from — a struct/class, a
//! method with parameters, a call with a receiver, an import with an alias —
//! and small enough that any change in the record is a change a reader sees.

#![cfg(feature = "deep")]

#[path = "../src/facts.rs"]
mod facts;
#[path = "../src/json.rs"]
mod json;
#[path = "../src/deep/mod.rs"]
mod deep;
#[path = "../src/deep_integration.rs"]
mod deep_integration;
#[path = "../src/deep_facts/mod.rs"]
mod deep_facts;

use deep_facts::{extract, lang_for, lang_for_forced};
use deep_integration::{parse_file, Lang};
use facts::{Facts, KIND_CLASS, KIND_FUNCTION, KIND_MODULE};

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn record(path: &str, lang: Lang) -> Facts {
    let src = read(path);
    let cst = parse_file(&src, lang).unwrap_or_else(|| panic!("{path} does not parse"));
    extract(&src, &cst, lang)
}

/// One symbol, by the key the walk builds for it.
fn symbol<'a>(facts: &'a Facts, key: &str) -> &'a facts::Symbol {
    facts
        .symbols
        .iter()
        .find(|symbol| symbol.key == key)
        .unwrap_or_else(|| panic!("no symbol {key} in {:?}", facts.symbols.iter().map(|s| &s.key).collect::<Vec<_>>()))
}

fn calls(facts: &Facts, key: &str) -> Vec<(String, u32)> {
    symbol(facts, key).calls.iter().map(|call| (call.path.clone(), call.line)).collect()
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

#[test]
fn the_tags_that_reach_the_cst_path_include_go_and_java() {
    // Go and Java have tables, but their records still lose the lexer's
    // artefacts often enough that `auto` keeps the lexer on most real files —
    // measured on 112 files of Go's net/http: 91 fallbacks, and auto costs 2.6x
    // the lexer for nothing. They are therefore force-only: `--deep` parses
    // them, `auto` does not spend a second parse on them.
    assert_eq!(lang_for("GO"), None);
    assert_eq!(lang_for("JAVA"), None);
    assert_eq!(lang_for_forced("GO"), Some(Lang::Go));
    assert_eq!(lang_for_forced("JAVA"), Some(Lang::Java));
    // The tags auto does pick.
    assert_eq!(lang_for("RS"), Some(Lang::Rust));
    assert_eq!(lang_for("PY"), Some(Lang::Python));
    // A language with no table answers neither.
    assert_eq!(lang_for("C"), None);
    assert_eq!(lang_for_forced("C"), None);
}

#[test]
fn go_facts_are_read_off_the_tree() {
    let facts = record("tests/fixtures/facts_go.go", Lang::Go);

    assert_eq!(facts.tag, "GO");
    assert_eq!(facts.lines, 21);

    // `import ("fmt"; str "strings")`: the module is the string as written and
    // the alias is what the spec binds — the explicit `str`, the path's last
    // segment otherwise. Both carry the import clause's line.
    let imports: Vec<(String, String, u8)> =
        facts.imports.iter().map(|row| (row.module.clone(), row.alias.clone(), row.kind)).collect();
    assert_eq!(
        imports,
        vec![
            ("fmt".to_string(), "fmt".to_string(), facts::IMPORT_MODULE),
            ("strings".to_string(), "str".to_string(), facts::IMPORT_MODULE),
        ]
    );

    // Go has no class declaration: a struct is a statement, so the only symbols
    // are the module and the two functions. A method takes its owner from the
    // receiver's type and its parameters from the second list.
    assert_eq!(facts.symbols.len(), 3);
    let module = symbol(&facts, "<module>");
    assert_eq!(module.kind, KIND_MODULE);
    assert_eq!(module.line, 1);

    let new = symbol(&facts, "NewRepo");
    assert_eq!((new.name.as_str(), new.owner.as_str(), new.kind, new.line), ("NewRepo", "", KIND_FUNCTION, 12));
    assert_eq!(new.params, owned(&["name"]));

    let find = symbol(&facts, "Repo.Find");
    assert_eq!((find.name.as_str(), find.owner.as_str(), find.kind, find.line), ("Find", "Repo", KIND_FUNCTION, 16));
    assert_eq!(find.params, owned(&["id"]));

    // A call with a receiver keeps the path the source spells, and the
    // receiver's own name is `self` exactly as the lexer writes it.
    assert_eq!(
        calls(&facts, "Repo.Find"),
        vec![("str.TrimSpace".to_string(), 17), ("fmt.Println".to_string(), 18)]
    );

    // The events: `trimmed := str.TrimSpace(r.name)` is an assignment to one
    // name, `return trimmed, nil` a return of two values.
    assert_eq!(find.events.matches("[\"a\",17").count(), 1);
    assert!(find.events.contains("\"a\",17,[\"trimmed\"]"));
    assert!(find.events.contains("\"r\",19,"));
    assert!(find.events.contains("[\"n\",\"trimmed\"]"));

    // `package shop` and `type Repo struct` are statements to the one-pass
    // lexer, so they are events here too, on their own lines.
    assert!(module.events.contains("\"x\",1,"), "{}", module.events);
    assert!(module.events.contains("\"x\",8,"), "{}", module.events);
    assert!(module.events.contains("\"x\",9,"), "{}", module.events);

    // `str.TrimSpace` is a call, not a construction: a lowercase callee binds
    // no class.
    assert!(facts.vars.is_empty(), "{:?}", facts.vars);
}

#[test]
fn java_facts_are_read_off_the_tree() {
    let facts = record("tests/fixtures/facts_java.java", Lang::Java);

    assert_eq!(facts.tag, "JAVA");
    assert_eq!(facts.lines, 19);

    let imports: Vec<(String, String, u8)> =
        facts.imports.iter().map(|row| (row.module.clone(), row.alias.clone(), row.kind)).collect();
    assert_eq!(
        imports,
        vec![
            ("java.util.List".to_string(), "List".to_string(), facts::IMPORT_MODULE),
            ("java.util.Map.Entry".to_string(), "Entry".to_string(), facts::IMPORT_MODULE),
        ]
    );

    // The class is a container with its declared bases, the constructor is a
    // method of it, and so is `find`.
    let store = symbol(&facts, "Store");
    assert_eq!((store.name.as_str(), store.kind, store.line), ("Store", KIND_CLASS, 6));
    assert_eq!(store.bases, owned(&["Base", "Repo"]));

    let constructor = symbol(&facts, "Store.Store");
    assert_eq!((constructor.name.as_str(), constructor.owner.as_str(), constructor.kind), ("Store", "Store", KIND_FUNCTION));
    assert_eq!(constructor.params, owned(&["names"]));

    let find = symbol(&facts, "Store.find");
    assert_eq!((find.name.as_str(), find.owner.as_str(), find.kind, find.line), ("find", "Store", KIND_FUNCTION, 13));
    // `int id`: the name is the last identifier of a `type name` parameter.
    assert_eq!(find.params, owned(&["id"]));

    // `this.names` is spelled `self.names`, and a chained call keeps its path.
    assert_eq!(
        calls(&facts, "Store.find"),
        vec![
            ("self.names.get".to_string(), 14),
            ("Logger.log".to_string(), 15),
            ("name.trim".to_string(), 15),
        ]
    );

    assert!(find.events.contains("\"a\",14,[\"name\"]"), "{}", find.events);
    assert!(find.events.contains("\"x\",15,"), "{}", find.events);
    assert!(find.events.contains("\"r\",16,"), "{}", find.events);

    // The constructor assigns to the field it takes: the target is the path the
    // lexer spells `self.names`.
    assert!(constructor.events.contains("\"a\",10,[\"self.names\"]"), "{}", constructor.events);

    // `private final List<String> names;` is a statement on the enclosing
    // symbol, as it is to the lexer: the field's type names are in it.
    let module = symbol(&facts, "<module>");
    assert!(module.events.contains("\"x\",7,"), "{}", module.events);
}
