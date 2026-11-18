//! Kotlin enum entry lists are entries, not a comma expression.
//!
//! `A,` / `B;` closes a statement when the `;` ends the list, so the one-pass
//! reader saw the entry lines as one statement and wrote their names as a
//! tuple (`t:[n:A,n:B]`). The CST reads them as `enum_entry` nodes and writes
//! no value, so every enum that ended its list with `;` cost the file its
//! events: 30 files of the kotlin dump, `KmVersionRequirementVersionKind` and
//! the rest of the `Jvm,` rows among them.
//!
//! What is pinned is that neither reader writes an event on an entry line, and
//! that the members after the `;` keep theirs.

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

fn events(facts: &Facts) -> String {
    let mut out = String::new();
    for symbol in &facts.symbols {
        out.push_str(&symbol.events);
    }
    out
}

fn both(source: &str) -> (String, String) {
    let language = langs::by_content("kt", source.as_bytes()).expect("a Kotlin row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Kotlin).expect("a Kotlin tree");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::Kotlin);
    (events(&lexer), events(&deep))
}

/// An event of the marker `marker` filed on `line`.
fn filed(events: &str, marker: &str, line: u32) -> bool {
    events.contains(&format!("[\"{marker}\",{line},"))
}

const ROWS: &[(&str, &str, &[u32])] = &[
    (
        "a list closed by a semicolon",
        "enum class L {\n    A,\n    B;\n}\n",
        &[2, 3],
    ),
    (
        "a list with a KDoc per entry",
        "enum class M {\n    /**\n     * One.\n     */\n    A,\n\n    /**\n     * Two.\n     */\n    B\n    ;\n}\n",
        &[5, 10],
    ),
    (
        "a list with a member after the semicolon",
        "enum class N {\n    A,\n    B;\n\n    fun value(): Int = 1\n}\n",
        &[2, 3],
    ),
];

#[test]
fn kotlin_enum_entry_list_states_nothing_of_its_own() {
    let mut failures: Vec<String> = Vec::new();
    for (label, source, lines) in ROWS {
        let (lexer, deep) = both(source);
        for line in *lines {
            for marker in ["x", "a", "t"] {
                if filed(&lexer, marker, *line) {
                    failures.push(format!("{label}: the lexer filed {marker} on entry line {line}: {lexer}"));
                }
                if filed(&deep, marker, *line) {
                    failures.push(format!("{label}: the cst filed {marker} on entry line {line}: {deep}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "kotlin enum entries disagree:\n{}", failures.join("\n"));
}
