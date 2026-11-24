//! Asserts the x-event nesting invariant (`docs/X-EVENT-NESTING.md`) across
//! all 45 `corpus45` languages, plus the Shell (`SH`) redirection / path /
//! positional-parameter call rules.

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

struct Fixture {
    name: &'static str,
    suffix: &'static str,
    deep_lang: Option<Lang>,
    expected_calls: &'static [&'static str],
    src: &'static str,
}

const FIXTURES: &[Fixture] = &[
    Fixture { name: "assembly", suffix: "s", deep_lang: None, expected_calls: &[], src: "main:\n    mov rax, rbx\n    ret\n" },
    Fixture { name: "awk", suffix: "awk", deep_lang: Some(Lang::Awk), expected_calls: &["outer", "inner"], src: "function run(x) {\n    return outer(inner(x))\n}\n" },
    Fixture { name: "bash", suffix: "sh", deep_lang: Some(Lang::Shell), expected_calls: &["outer", "inner"], src: "run() {\n    outer inner \"$x\" >/dev/null 2>&1\n}\n" },
    Fixture { name: "c", suffix: "c", deep_lang: Some(Lang::C), expected_calls: &["outer", "inner"], src: "int run(int x) {\n    return outer(inner(x));\n}\n" },
    Fixture { name: "clojure", suffix: "clj", deep_lang: None, expected_calls: &["outer", "inner"], src: "(defn run [x]\n  (outer (inner x) y))\n" },
    Fixture { name: "cobol", suffix: "cob", deep_lang: None, expected_calls: &["work-step"], src: "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. DEMO.\n       PROCEDURE DIVISION.\n           PERFORM WORK-STEP.\n           STOP RUN.\n" },
    Fixture { name: "cpp", suffix: "cpp", deep_lang: Some(Lang::Cpp), expected_calls: &["outer", "inner"], src: "int run(int x) {\n    return outer(inner(x));\n}\n" },
    Fixture { name: "crystal", suffix: "cr", deep_lang: None, expected_calls: &["outer", "inner"], src: "def run(x)\n  outer inner(x)\nend\n" },
    Fixture { name: "csharp", suffix: "cs", deep_lang: Some(Lang::CSharp), expected_calls: &["outer", "inner"], src: "class Demo {\n    int Run(int x) {\n        return outer(inner(x));\n    }\n}\n" },
    Fixture { name: "d", suffix: "d", deep_lang: None, expected_calls: &["outer", "inner"], src: "int run(int x) {\n    return outer(inner(x));\n}\n" },
    Fixture { name: "dart", suffix: "dart", deep_lang: Some(Lang::Dart), expected_calls: &["outer", "inner"], src: "int run(int x) {\n  return outer(inner(x));\n}\n" },
    Fixture { name: "elixir", suffix: "ex", deep_lang: None, expected_calls: &["outer", "inner"], src: "defmodule Demo do\n  def run(x) do\n    outer(inner(x))\n  end\nend\n" },
    Fixture { name: "elm", suffix: "elm", deep_lang: None, expected_calls: &["outer", "inner"], src: "run x =\n    outer (inner x) y\n" },
    Fixture { name: "erlang", suffix: "erl", deep_lang: Some(Lang::Erlang), expected_calls: &["outer", "inner"], src: "-module(demo).\n-export([run/1]).\nrun(X) ->\n    outer(inner(X)).\n" },
    Fixture { name: "fortran", suffix: "f90", deep_lang: None, expected_calls: &["outer", "inner"], src: "subroutine run(x)\n  call outer(inner(x))\nend subroutine run\n" },
    Fixture { name: "fsharp", suffix: "fs", deep_lang: None, expected_calls: &["outer", "inner"], src: "let run x =\n    outer (inner x) y\n" },
    Fixture { name: "go", suffix: "go", deep_lang: Some(Lang::Go), expected_calls: &["outer", "inner"], src: "package demo\nfunc run(x int) int {\n    return outer(inner(x))\n}\n" },
    Fixture { name: "groovy", suffix: "groovy", deep_lang: None, expected_calls: &["outer", "inner"], src: "def run(x) {\n    return outer(inner(x))\n}\n" },
    Fixture { name: "haskell", suffix: "hs", deep_lang: Some(Lang::Haskell), expected_calls: &["outer", "inner"], src: "run x = outer (inner x) y\n" },
    Fixture { name: "java", suffix: "java", deep_lang: Some(Lang::Java), expected_calls: &["outer", "inner"], src: "class Demo {\n    int run(int x) {\n        return outer(inner(x));\n    }\n}\n" },
    Fixture { name: "javascript", suffix: "js", deep_lang: Some(Lang::JavaScript), expected_calls: &["outer", "inner"], src: "function run(x) {\n    return outer(inner(x));\n}\n" },
    Fixture { name: "julia", suffix: "jl", deep_lang: None, expected_calls: &["outer", "inner"], src: "function run(x)\n    return outer(inner(x))\nend\n" },
    Fixture { name: "kotlin", suffix: "kt", deep_lang: Some(Lang::Kotlin), expected_calls: &["outer", "inner"], src: "fun run(x: Int): Int {\n    return outer(inner(x))\n}\n" },
    Fixture { name: "lua", suffix: "lua", deep_lang: Some(Lang::Lua), expected_calls: &["outer", "inner"], src: "function run(x)\n    return outer(inner(x))\nend\n" },
    Fixture { name: "matlab", suffix: "m", deep_lang: None, expected_calls: &["outer", "inner"], src: "function y = run(x)\n    y = outer(inner(x));\nend\n" },
    Fixture { name: "nim", suffix: "nim", deep_lang: None, expected_calls: &["outer", "inner"], src: "proc run(x: int): int =\n  outer(inner(x))\n" },
    Fixture { name: "objective-c", suffix: "mm", deep_lang: Some(Lang::ObjC), expected_calls: &["prep:", "send:"], src: "@implementation Demo\n- (int)run:(int)x {\n    return [outer send:[inner prep:x]];\n}\n@end\n" },
    Fixture { name: "ocaml", suffix: "ml", deep_lang: None, expected_calls: &["outer", "inner"], src: "let run x =\n  outer (inner x) y\n" },
    Fixture { name: "pascal", suffix: "pas", deep_lang: Some(Lang::Pascal), expected_calls: &["outer", "inner"], src: "function Run(x: Integer): Integer;\nbegin\n  Run := Outer(Inner(x));\nend;\n" },
    Fixture { name: "perl", suffix: "pm", deep_lang: None, expected_calls: &["outer", "inner"], src: "sub run {\n    my ($x) = @_;\n    return outer(inner($x));\n}\n" },
    Fixture { name: "php", suffix: "php", deep_lang: None, expected_calls: &["outer", "inner"], src: "<?php\nfunction run($x) {\n    return outer(inner($x));\n}\n" },
    Fixture { name: "powershell", suffix: "ps1", deep_lang: Some(Lang::PowerShell), expected_calls: &["invoke-outer"], src: "function Invoke-Run($x) {\n    Invoke-Outer $x\n}\n" },
    Fixture { name: "python", suffix: "py", deep_lang: Some(Lang::Python), expected_calls: &["outer", "inner"], src: "def run(x):\n    return outer(inner(x))\n" },
    Fixture { name: "r", suffix: "r", deep_lang: Some(Lang::R), expected_calls: &["outer", "inner"], src: "run <- function(x) {\n  outer(inner(x))\n}\n" },
    Fixture { name: "ruby", suffix: "rb", deep_lang: None, expected_calls: &["outer", "inner"], src: "def run(x)\n  outer inner(x)\nend\n" },
    Fixture { name: "rust", suffix: "rs", deep_lang: Some(Lang::Rust), expected_calls: &["outer", "inner"], src: "fn run(x: i32) -> i32 {\n    outer(inner(x))\n}\n" },
    Fixture { name: "scala", suffix: "scala", deep_lang: Some(Lang::Scala), expected_calls: &["outer", "inner"], src: "object Demo {\n  def run(x: Int): Int = {\n    outer(inner(x))\n  }\n}\n" },
    Fixture { name: "solidity", suffix: "sol", deep_lang: None, expected_calls: &["outer", "inner"], src: "pragma solidity ^0.8.0;\ncontract Demo {\n    function run(uint256 x) public pure returns (uint256) {\n        return outer(inner(x));\n    }\n}\n" },
    Fixture { name: "sql", suffix: "sql", deep_lang: None, expected_calls: &["outer_fn", "inner_fn"], src: "SELECT outer_fn(inner_fn(x)) FROM items;\n" },
    Fixture { name: "swift", suffix: "swift", deep_lang: Some(Lang::Swift), expected_calls: &["outer", "inner"], src: "func run(_ x: Int) -> Int {\n    return outer(inner(x))\n}\n" },
    Fixture { name: "tcl", suffix: "tcl", deep_lang: Some(Lang::Tcl), expected_calls: &["outer", "inner"], src: "proc run {x} {\n    outer inner $x\n}\n" },
    Fixture { name: "typescript", suffix: "ts", deep_lang: Some(Lang::TypeScript), expected_calls: &["outer", "inner"], src: "function run(x: number): number {\n    return outer<number>(inner(x));\n}\n" },
    Fixture { name: "vbnet", suffix: "vb", deep_lang: None, expected_calls: &["outer", "inner"], src: "Sub Run(x As Integer)\n    Call Outer(Inner(x))\nEnd Sub\n" },
    Fixture { name: "visual-basic", suffix: "bas", deep_lang: None, expected_calls: &["outer", "inner"], src: "Sub Run(x)\n    Call Outer(Inner(x))\nEnd Sub\n" },
    Fixture { name: "zig", suffix: "zig", deep_lang: None, expected_calls: &["outer", "inner"], src: "fn run(x: i32) i32 {\n    return outer(inner(x));\n}\n" },
];

#[test]
fn all_45_language_fixtures_obey_the_x_event_nesting_invariant() {
    assert_eq!(FIXTURES.len(), 45);
    let mut lexer_failures: Vec<String> = Vec::new();
    let mut deep_losses: Vec<String> = Vec::new();

    for fx in FIXTURES {
        let bytes = fx.src.as_bytes();
        let Some(language) = langs::by_content(fx.suffix, bytes) else {
            if fx.name != "assembly" {
                lexer_failures.push(format!("{}: unexpected unmapped suffix", fx.name));
            }
            continue;
        };

        let lex = parser::parse(bytes, language);
        let mut all_calls: Vec<String> = Vec::new();
        let mut total_c_nodes = 0usize;
        let mut all_events = String::new();
        for sym in &lex.symbols {
            total_c_nodes += sym.events.matches("[\"c\",").count();
            all_events.push_str(&sym.events);
            for call in &sym.calls {
                all_calls.push(call.path.clone());
            }
        }

        if all_calls != fx.expected_calls {
            lexer_failures.push(format!(
                "{}: expected calls {:?}, got {:?}; events={all_events}",
                fx.name, fx.expected_calls, all_calls
            ));
            continue;
        }
        if total_c_nodes != all_calls.len() {
            lexer_failures.push(format!(
                "{}: `[\"c\",` count ({total_c_nodes}) != calls.len() ({}); events={all_events}",
                fx.name,
                all_calls.len()
            ));
            continue;
        }

        for callee in &all_calls {
            if !callee.contains('.') && !callee.contains(':') {
                let dup_name = format!("[\"n\",\"{callee}\"]");
                if all_events.contains(&dup_name) {
                    lexer_failures.push(format!(
                        "{}: bare callee {callee:?} also rendered as name in {all_events}",
                        fx.name
                    ));
                }
            }
        }

        if fx.expected_calls.len() == 2 {
            let nested = all_events.contains("[[\"c\",")
                || all_events.contains(",[\"c\",")
                || all_events.contains("[[\"m\",[[\"c\",");
            if !nested {
                lexer_failures.push(format!(
                    "{}: expected nested `[\"c\",` chain, got events={all_events}",
                    fx.name
                ));
            }
        }

        if let Some(lang) = fx.deep_lang {
            let cst = deep_integration::parse_file(bytes, lang)
                .unwrap_or_else(|| panic!("{}: deep parse returned None", fx.name));
            if cst.degraded() {
                deep_losses.push(format!("{}: degraded CST (errors={})", fx.name, cst.errors));
                continue;
            }
            let deep_record = deep_facts::extract(bytes, &cst, lang);
            let verdict = pipeline::loses_in(&lex, &deep_record, fx.src);
            if verdict.lost() {
                deep_losses.push(format!("{}: {}", fx.name, verdict.detail()));
            }
        }
    }

    assert!(
        lexer_failures.is_empty(),
        "x-event nesting invariant failures ({}):\n{}",
        lexer_failures.len(),
        lexer_failures.join("\n")
    );
    assert!(
        deep_losses.is_empty(),
        "CST guard parity failures on x-event fixtures ({}):\n{}",
        deep_losses.len(),
        deep_losses.join("\n")
    );
}

#[test]
fn shell_redirections_paths_and_positional_sigils_do_not_invent_calls() {
    let sh = langs::by_content("sh", b"").expect("SH row");
    let src = br#"
check() {
    command -v yq >/dev/null 2>&1
    kube::util::read-array protos
    /usr/bin/env python - <<EOF
hello
EOF
    for item in alpha beta gamma; do
        reject $1 https
    done
    go list all 2>/dev/null
}
"#;
    let lex = parser::parse(src, sh);
    let calls: Vec<&str> = lex
        .symbols
        .iter()
        .flat_map(|s| s.calls.iter().map(|c| c.path.as_str()))
        .collect();
    assert!(
        !calls.contains(&"null")
            && !calls.contains(&"kube.util.read-array")
            && !calls.contains(&"env")
            && !calls.contains(&"python")
            && !calls.contains(&"alpha")
            && !calls.contains(&"beta")
            && !calls.contains(&"1")
            && !calls.contains(&"go"),
        "unexpected invented SH calls: {calls:?}"
    );
    assert!(calls.contains(&"v") && calls.contains(&"reject"), "real SH calls preserved: {calls:?}");

    let cst = deep_integration::parse_file(src, Lang::Shell).expect("shell CST");
    let deep_record = deep_facts::extract(src, &cst, Lang::Shell);
    let verdict = pipeline::loses_in(&lex, &deep_record, std::str::from_utf8(src).unwrap());
    assert!(
        !verdict.lost(),
        "SH construct fixture lost: {verdict:?} ({})",
        verdict.detail()
    );
}
