//! The C-family reader: eight languages, one walk and one node table each.
//!
//! C, C++, Java, C#, Go, Kotlin, Scala and Swift all read through
//! `deep_facts::extract` — the single `Walker` — and differ only in the `Kinds`
//! table selected by the language. This test is the structural claim in code:
//! every tag resolves to a language, every language has a non-empty definition
//! table, and the smallest source of each language yields the symbol and the
//! call the reader is supposed to find.
//!
//! The reader arms themselves are tested where they live (`facts_c_cpp`,
//! `facts_go_java`, `facts_rust`). This file is about the shared spine.
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

fn read(source: &str, lang: Lang) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), lang).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, lang)
}

/// The eight languages the plan names, with the smallest source that declares
/// one container member and calls one function from it.
const FAMILY: &[(&str, &str, Lang, &str)] = &[
    ("C", "C", Lang::C, "int add(int a, int b) {\n    return helper(a) + b;\n}\n"),
    (
        "C++",
        "CPP",
        Lang::Cpp,
        "class Box {\npublic:\n    int get() {\n        return load();\n    }\n    int size_;\n};\n",
    ),
    ("Java", "JAVA", Lang::Java, "class A {\n    int get() {\n        return load();\n    }\n}\n"),
    ("C#", "CS", Lang::CSharp, "class A {\n    int Get() {\n        return Load();\n    }\n}\n"),
    ("Go", "GO", Lang::Go, "package p\n\nfunc Get() int {\n    return load()\n}\n"),
    ("Kotlin", "KT", Lang::Kotlin, "class A {\n    fun get(): Int {\n        return load()\n    }\n}\n"),
    ("Scala", "SCALA", Lang::Scala, "class A {\n  def get(): Int = load()\n}\n"),
    ("Swift", "SWIFT", Lang::Swift, "class A {\n    func get() -> Int {\n        return load()\n    }\n}\n"),
];

#[test]
fn every_family_tag_resolves_to_a_language() {
    let mut missing: Vec<&str> = Vec::new();
    for (name, tag, _, _) in FAMILY {
        if deep_facts::lang_for_forced(tag).is_none() {
            missing.push(name);
        }
    }
    assert!(missing.is_empty(), "no deep language for: {missing:?}");
}

#[test]
fn every_family_language_has_a_node_table() {
    let mut empty: Vec<&str> = Vec::new();
    for (name, _, lang, _) in FAMILY {
        if deep_facts::definition_kinds(*lang) == 0 {
            empty.push(name);
        }
    }
    assert!(empty.is_empty(), "no definition table for: {empty:?}");
}

/// `struct group *getgrent() { … }` defines a function whose return type is a
/// struct; it does not define the struct. Read as a class body, every statement
/// of the function belonged to a symbol the CST never writes, so the events
/// landed on the module and were paired against the whole file (121 events on
/// 68 musl files).
#[test]
fn a_c_function_returning_a_struct_opens_the_function_not_a_class() {
    let source = "struct group *getgrent()\n{\n\tstruct group *res;\n\tres = read_group();\n\treturn res;\n}\n";
    let language = langs::by_content("c", source.as_bytes()).expect("a C row");
    let facts = parser::parse(source.as_bytes(), language);
    let names: Vec<(&str, u8)> = facts.symbols.iter().map(|symbol| (symbol.name.as_str(), symbol.kind)).collect();
    assert!(
        facts.symbols.iter().any(|symbol| symbol.name == "getgrent" && symbol.kind == facts::KIND_FUNCTION),
        "the function was not opened: {names:?}"
    );
    assert!(
        !facts.symbols.iter().any(|symbol| symbol.name == "group" && symbol.kind == facts::KIND_CLASS),
        "the return type was opened as a class: {names:?}"
    );
}

#[test]
fn every_family_language_reads_its_own_declaration_and_call() {
    let mut failed: Vec<String> = Vec::new();
    for (name, _, lang, source) in FAMILY {
        let facts = read(source, *lang);
        let keys: Vec<&str> = facts.symbols.iter().map(|symbol| symbol.key.as_str()).collect();
        let declares = facts.symbols.iter().any(|symbol| symbol.kind != 0 && symbol.key != "<module>");
        let calls: usize = facts.symbols.iter().map(|symbol| symbol.calls.len()).sum();
        if !declares || calls == 0 {
            failed.push(format!("{name}: symbols {keys:?}, calls {calls}"));
        }
    }
    assert!(failed.is_empty(), "the shared reader lost a declaration or a call:\n{}", failed.join("\n"));
}
