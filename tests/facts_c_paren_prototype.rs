//! A declarator in parentheses names the function, and both readers say so.
//!
//! Lua's headers spell every prototype as `LUA_API lua_State *(lua_newstate)
//! (lua_Alloc f, void *ud);` or `LUALIB_API int (luaopen_base) (lua_State *L);`:
//! the declared name sits inside a parenthesis instead of after the type. The
//! one-pass lexer reads the name off the line; the CST read the return type, or
//! nothing at all, so 113 rows of redis/deps/lua/src/{lua,lauxlib,lualib}.h fell
//! back. This pins the four shapes the headers use, plus the preprocessor
//! fragment that sits beside them in musl's ctype tables.
//!
//! Each row is the smallest source that keeps the shape, and the guard has to
//! find nothing to fall back for. A row that stops agreeing fails here before it
//! costs a corpus file.
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

fn lexer_facts(extension: &str, source: &str) -> Facts {
    let language = langs::by_content(extension, source.as_bytes()).expect("a C row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(lang: Lang, source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), lang).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, lang)
}

/// One row per parenthesised-declarator shape the lua headers write.
const COVERAGE: &[(&str, Lang, &str)] = &[
    (
        "a macro, a pointer return type and a parenthesised name",
        Lang::C,
        "typedef struct lua_State lua_State;\ntypedef void *(*lua_Alloc)(void *ud, void *ptr);\n\nLUA_API lua_State *(lua_newstate) (lua_Alloc f, void *ud);\n",
    ),
    (
        "a macro and a parenthesised name with a keyword return type",
        Lang::C,
        "typedef struct lua_State lua_State;\n\nLUA_API int (lua_gettop) (lua_State *L);\n",
    ),
    (
        "a parenthesised name with a void return type",
        Lang::C,
        "typedef struct lua_State lua_State;\n\nLUA_API void (lua_close) (lua_State *L);\n",
    ),
    (
        "a typedef return type and a parenthesised name",
        Lang::C,
        "typedef struct lua_State lua_State;\ntypedef long lua_Number;\n\nLUALIB_API lua_Number (luaL_checknumber) (lua_State *L);\n",
    ),
    (
        "a macro, a pointer and a parenthesised name over two lines",
        Lang::C,
        "typedef struct lua_State lua_State;\n\nLUALIB_API const char *(luaL_checklstring) (lua_State *L, int numArg,\n                                                          size_t *l);\n",
    ),
    (
        "a header that is a fragment of brace groups",
        Lang::C,
        "{ 0x80, 36 },\n{ 0xa5, 2 },\n{ 0xffe6, 26 },\n",
    ),
    (
        "a header that is a fragment of bare numbers",
        Lang::C,
        "18,17,19,20,21,22,\n23,24,25,26,27,28,\n",
    ),
];

#[test]
fn every_parenthesised_declarator_keeps_its_statement() {
    let mut failed: Vec<String> = Vec::new();
    for (name, lang, source) in COVERAGE {
        let lexer = lexer_facts("c", source);
        let deep = deep_facts(*lang, source);
        let verdict = pipeline::loses_in(&lexer, &deep, source);
        if verdict.lost() {
            failed.push(format!("{name}: {verdict:?}"));
        }
    }
    assert!(
        failed.is_empty(),
        "the parenthesised declarator cost a statement in {} shape(s):\n{}",
        failed.len(),
        failed.join("\n")
    );
}
