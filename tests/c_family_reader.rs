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

/// `int i = 0, cnt;` declares two names: one initialised, one not. The lexer
/// read the comma as a comma operator and gave `i` the value `t:[0,n:cnt]`,
/// which is the next declarator and not part of the initialiser. The CST
/// writes `i` with `0` and nothing for the untouched name, so the two records
/// disagreed on five corpus files (musl mallocng and passwd) for a statement
/// both readers read correctly.
#[test]
fn a_c_declarator_list_reads_the_same_on_both_paths() {
    let mut failed: Vec<String> = Vec::new();
    for source in [
        "int i = 0, cnt;\n",
        "size_t size=0, nmem=0;\n",
        "char *p = 0, *q;\n",
        "unsigned long a = 0, b = 0;\n",
    ] {
        let language = langs::by_content("c", source.as_bytes()).expect("a C row");
        let lexer = parser::parse(source.as_bytes(), language);
        let deep = read(source, Lang::C);
        let verdict = pipeline::loses(&lexer, &deep);
        if verdict.lost() {
            failed.push(format!("{source:?}: {verdict:?}"));
        }
    }
    assert!(failed.is_empty(), "a declarator list disagreed in {} file(s):\n{}", failed.len(), failed.join("\n"));
}

/// `void (f) (int x);` declares `f` and writes no value: the parentheses around
/// the name exist so a macro over that name cannot expand it, which is the lua
/// headers' whole style (`LUALIB_API void (luaL_setn) (lua_State *L, ...)`).
/// The lexer used to write the name as the line's value, and the CST wrote the
/// parameter list as a call, so every declaration in those headers lost a row.
#[test]
fn a_parenthesised_declarator_writes_no_value() {
    let mut failed: Vec<String> = Vec::new();
    for source in [
        "void (f) (int x);\n",
        "LUALIB_API void (luaL_setn) (lua_State *L, int t, int n);\n",
        "void kvstoreEmpty(kvstore *kvs, void(callback)(dict*));\n",
    ] {
        let language = langs::by_content("c", source.as_bytes()).expect("a C row");
        let lexer = parser::parse(source.as_bytes(), language);
        let deep = read(source, Lang::C);
        let verdict = pipeline::loses(&lexer, &deep);
        if verdict.lost() {
            failed.push(format!("{source:?}: {verdict:?}"));
        }
    }
    assert!(failed.is_empty(), "a parenthesised declarator lost {} line(s):\n{}", failed.len(), failed.join("\n"));
}

/// `{ 0x80, 36 },` is a row of a table header that a declaration `#include`s
/// (musl's locale and ctype tables). At file scope C has no compound
/// statement, so the row is data and writes no fact; read as a block, the lexer
/// filed one tuple value per row and the CST had nothing to match — 99 rows in
/// gb18030utf.h alone.
#[test]
fn a_table_header_row_writes_no_value() {
    let source = "{ 0x80, 36 },\n{ 0xa5, 2 },\n{ 0xa9, 7 },\n";
    let language = langs::by_content("c", source.as_bytes()).expect("a C row");
    let lexer = parser::parse(source.as_bytes(), language);
    let deep = read(source, Lang::C);
    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "a table row lost a value: {verdict:?}");
}
