//! Kotlin and Swift string interpolation: the hole is code, the text is not.
//!
//! `"$name"` / `"${greet(x)}"` and `"\(greet(x))"` put an expression inside a
//! string. The lexer's string reader and the CST's `interpolation` node are two
//! different readings of the same bytes, and a reader that takes the whole
//! literal as text drops the call in the hole, while one that takes the text as
//! code invents names. Each row pins one shape and asserts the CST lost nothing
//! the lexer recorded.
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

fn lexer_facts(tag: &str, source: &str) -> Facts {
    let language = langs::by_content(tag, source.as_bytes()).expect("a language row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(lang: Lang, source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), lang).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, lang)
}

struct Row {
    name: &'static str,
    tag: &'static str,
    lang: Lang,
    source: &'static str,
}

const COVERAGE: &[Row] = &[
    Row {
        name: "kotlin: a simple template names a variable",
        tag: "kt",
        lang: Lang::Kotlin,
        source: "fun greet(name: String): String {\n    return \"hi $name\"\n}\n",
    },
    Row {
        name: "kotlin: a braced template calls a function",
        tag: "kt",
        lang: Lang::Kotlin,
        source: "fun greet(name: String): String {\n    return \"hi ${load(name)}\"\n}\n",
    },
    Row {
        name: "kotlin: a braced template reads a property",
        tag: "kt",
        lang: Lang::Kotlin,
        source: "fun label(user: User): String {\n    return \"user ${user.title}\"\n}\n",
    },
    Row {
        name: "kotlin: two templates in one literal",
        tag: "kt",
        lang: Lang::Kotlin,
        source: "fun pair(a: Int, b: Int): String {\n    return \"$a:${b + 1}\"\n}\n",
    },
    Row {
        name: "swift: an interpolation calls a function",
        tag: "swift",
        lang: Lang::Swift,
        source: "func greet(name: String) -> String {\n    return \"hi \\(load(name))\"\n}\n",
    },
    Row {
        name: "swift: an interpolation reads a variable",
        tag: "swift",
        lang: Lang::Swift,
        source: "func greet(name: String) -> String {\n    return \"hi \\(name)\"\n}\n",
    },
    Row {
        name: "swift: two interpolations in one literal",
        tag: "swift",
        lang: Lang::Swift,
        source: "func pair(a: Int, b: Int) -> String {\n    return \"\\(a):\\(sum(a, b))\"\n}\n",
    },
];

/// Shapes that still disagree.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];

#[test]
fn every_interpolation_shape_writes_the_same_record() {
    let mut failed: Vec<String> = Vec::new();
    for row in COVERAGE {
        let lexer = lexer_facts(row.tag, row.source);
        let deep = deep_facts(row.lang, row.source);
        let verdict = pipeline::loses(&lexer, &deep);
        if verdict.lost() {
            failed.push(format!("{}: {verdict:?}", row.name));
        }
    }
    assert!(failed.is_empty(), "interpolation disagrees with the lexer:\n{}", failed.join("\n"));
}

#[test]
fn the_known_interpolation_gaps_are_still_gaps() {
    for (name, source, disagreement) in KNOWN_GAPS {
        let row = COVERAGE.iter().find(|row| row.name.contains(name)).expect("a row for the gap");
        let verdict = pipeline::loses(&lexer_facts(row.tag, source), &deep_facts(row.lang, source));
        assert!(verdict.lost(), "{name} now reads cleanly — move it into COVERAGE: {disagreement}");
    }
}
