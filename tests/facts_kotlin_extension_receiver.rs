//! A Kotlin extension function is named after its member, not its receiver.
//!
//! `fun String?.plus(other: Any?): String` is the member `plus` on a nullable
//! `String`; the receiver is a type, not an owner. The name chain only joined
//! `.`, `::` and `:`, so a nullable receiver's `?.` ended the chain at
//! `String` — the function took the receiver's name and every row inside it
//! was filed under a symbol no reader names that way (13 kotlin rows).

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

use facts::Facts;

fn symbols(source: &str) -> Vec<(String, String)> {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let facts: Facts = parser::parse(source.as_bytes(), language);
    facts.symbols.iter().map(|symbol| (symbol.name.clone(), symbol.owner.clone())).collect()
}

const NULLABLE: &str = "fun String?.plus(other: Any?): String = (this ?: \"null\") + other.toString()\n";

#[test]
fn a_nullable_receiver_names_the_member() {
    let rows = symbols(NULLABLE);
    assert!(
        rows.iter().any(|(name, _)| name == "plus"),
        "the member lost its name: {rows:?}"
    );
    assert!(
        !rows.iter().any(|(name, _)| name == "String"),
        "the receiver became the name: {rows:?}"
    );
    assert!(
        !rows.iter().any(|(name, owner)| name == "plus" && owner == "String"),
        "the receiver type became the owner: {rows:?}"
    );
}

const PLAIN: &str = "fun String.pad(n: Int): String = this + n\n";

#[test]
fn a_plain_receiver_still_names_the_member() {
    let rows = symbols(PLAIN);
    assert!(
        rows.iter().any(|(name, _)| name == "pad"),
        "the member lost its name: {rows:?}"
    );
}

const TYPE_PARAMETERS: &str = "public actual inline fun <T> arrayOf(vararg elements: T): Array<T> = elements as Array<T>\n";

#[test]
fn type_parameters_before_the_name_keep_the_declaration() {
    let rows = symbols(TYPE_PARAMETERS);
    assert!(
        rows.iter().any(|(name, _)| name == "arrayOf"),
        "the declaration lost its name: {rows:?}"
    );
    assert!(
        !rows.iter().any(|(name, _)| name.starts_with("<anonymous>")),
        "the declaration opened an anonymous frame: {rows:?}"
    );
}
