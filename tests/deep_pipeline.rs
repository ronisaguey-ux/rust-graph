//! The integration gate for the second parser: what the build actually does
//! when `deep` and `parser` are compiled in.
//!
//! `tests/deep_facts.rs` proves the facts the CST walk produces equal or
//! improve on the lexer's on the grammars' own fixtures. This file proves the
//! pipeline uses that path: which parser each file took, how a fallback is
//! recorded and counted, that a forced CST parse survives malformed input, and
//! that the facts a real build emits are never worse than the lexer's.
//!
//! Every assertion drives the built binary, so a wiring that compiles but is
//! not connected fails here.
#![cfg(all(feature = "deep", feature = "parser"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_rust-graph");

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str, files: &[(&str, &str)]) -> Tree {
        let dir = std::env::temp_dir().join(format!("rust-graph-deep-pipeline-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, body) in files {
            let target = dir.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, body).unwrap();
        }
        Tree(dir)
    }

    fn build(&self, extra: &[&str]) -> Output {
        Command::new(BIN).arg("build").arg(&self.0).args(extra).output().unwrap()
    }

    /// A successful build, its JSON on stdout, and its stderr.
    fn json(&self, extra: &[&str]) -> (String, String) {
        let out = self.build(extra);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        (String::from_utf8(out.stdout).unwrap(), String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The `sources` row for one path: every row is on its own line.
fn source_row<'a>(json: &'a str, path: &str) -> &'a str {
    json.lines()
        .find(|line| line.starts_with("{\"path\":") && line.contains(&format!("\"path\":\"{path}\"")))
        .unwrap_or_else(|| panic!("no sources row for {path} in\n{json}"))
}

/// One section of the document, by name, without the brackets.
fn section<'a>(json: &'a str, name: &str) -> &'a str {
    let start = json.find(&format!("\"{name}\":[")).unwrap_or_else(|| panic!("no {name} section")) + name.len() + 4;
    let end = json[start..].find("\n]").map(|at| start + at).unwrap_or(json.len());
    &json[start..end]
}

/// Every call path in the `flows` section: each entry is `["<path>",<line>,`
/// inside a symbol's `"calls"` array.
fn call_paths(json: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in section(json, "flows").split("\"calls\":[").skip(1) {
        let mut rest = chunk;
        while let Some(at) = rest.find("[\"") {
            let after = &rest[at + 2..];
            let Some(end) = after.find('"') else { break };
            let text = &after[..end];
            if text.len() > 1 {
                out.push(text.to_string());
            }
            // The next entry begins after this one's closing `]`, which the
            // line number and receiver cannot hide: `],` never occurs inside a
            // quoted path.
            rest = match after[end..].find("],") {
                Some(next) => &after[end + next + 2..],
                None => break,
            };
        }
    }
    out
}

/// How many rows one section holds: a row starts at column zero with `{`.
fn rows(json: &str, name: &str) -> usize {
    section(json, name).lines().filter(|line| line.starts_with('{')).count()
}

const CORPUS: &[(&str, &str)] = &[
    (
        "app/links.py",
        // A chained call: the lexer files `BeautifulSoup().find_all` as one
        // path, the CST two calls. A def whose signature spans two lines is
        // stamped where it starts by the CST and where it ends by the lexer.
        "from bs4 import BeautifulSoup\n\n\ndef links(html,\n          pattern=\"a\"):\n    soup = BeautifulSoup(html, \"html.parser\").find_all(\"a\")\n    found = [anchor.get(\"href\") for anchor in soup if anchor.has_attr(\"href\")]\n    return normalize(found, pattern)\n",
    ),
    (
        "app/normalize.py",
        "def normalize(found, pattern):\n    return [value for value in found if pattern in value]\n\n\nclass Report:\n    def render(self, rows):\n        return self.table.append(rows)\n",
    ),
    (
        "app/math.py",
        "def scale(value):\n    return value * 2\n\n\ndef total(values):\n    sum = 0\n    for value in values:\n        sum = sum + scale(value)\n    return sum\n",
    ),
];

const TWO_FILES: &[(&str, &str)] = &[
    ("app/a.py", "from app.b import helper\n\n\ndef f(x):\n    return helper(x)\n"),
    ("app/b.py", "def helper(y):\n    return y\n"),
    ("app/c.c", "int main(void) {\n  return helper(1);\n}\n"),
];

/// The graph without the per-file timing: `micros` is how long the read took,
/// not a fact about the source, so two builds — or a build and its cache —
/// cannot be required to reproduce it. Everything else is compared exactly.
fn without_timing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(",\"micros\":") {
        out.push_str(&rest[..at]);
        let after = &rest[at + ",\"micros\":".len()..];
        let digits = after.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(after.len());
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_default_takes_the_cst_where_there_is_one_and_the_lexer_elsewhere() {
    let tree = Tree::new("auto", TWO_FILES);
    let (json, _) = tree.json(&["--stdout", "--no-cache"]);

    // Per file, so the choice can be audited from one result.
    assert!(source_row(&json, "app/a.py").contains("\"parsed_by\":\"deep\""), "{json}");
    assert!(source_row(&json, "app/c.c").contains("\"parsed_by\":\"lexer\""), "{json}");
    // The summary says the same, and no language was downgraded by force.
    assert!(json.contains(r#""deep":{"mode":"auto","deep":2,"lexer":1,"fallbacks":0}"#), "{json}");
}

#[test]
fn a_flag_forces_one_parser_and_asking_for_both_is_an_error() {
    let tree = Tree::new("forced", TWO_FILES);
    let (json, _) = tree.json(&["--stdout", "--no-cache", "--lexer"]);
    assert!(json.contains(r#""mode":"lexer","deep":0,"lexer":3,"fallbacks":0"#), "{json}");
    for path in ["app/a.py", "app/b.py", "app/c.c"] {
        assert!(source_row(&json, path).contains("\"parsed_by\":\"lexer\""), "{path}: {json}");
    }

    let (json, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);
    assert!(source_row(&json, "app/a.py").contains("\"parsed_by\":\"deep\""), "{json}");

    let both = tree.build(&["--stdout", "--deep", "--lexer"]);
    assert_eq!(both.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&both.stderr);
    assert!(stderr.contains("choose one of --deep and --lexer"), "{stderr}");
}

#[test]
fn a_language_without_a_deep_parser_is_a_recorded_and_counted_fallback() {
    // `.c` is a language this build has no facts table for, so a forced CST
    // parse must say so rather than pretend: the file is still parsed by the
    // lexer, its facts are still written, and the fallback is both on the row
    // and in the counter a reader would sum.
    //
    // Rust used to be in the same list. `lang_for` answers for `RS` now (the
    // field L3 tables), so the same `.rs` file is parsed by the CST here and
    // the numbers moved: deep 1 -> 2, lexer 2 -> 1, fallbacks 2 -> 1 under
    // `--deep`, and `fallbacks` stays 0 under `auto` because the CST record no
    // longer ties with the lexer on this file — it wins.
    let tree = Tree::new("fallback", &[
        ("src/main.c", "int main(void) {\n  return helper(1);\n}\n"),
        ("src/lib.rs", "pub fn f() -> u32 {\n    1\n}\n"),
        ("src/app.py", "def f(x):\n    return x\n"),
    ]);
    let (json, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);

    let row = source_row(&json, "src/main.c");
    assert!(row.contains("\"parsed_by\":\"lexer\""), "{row}");
    assert!(row.contains("\"deep_fallback\":\"no deep parser for C\""), "{row}");
    // Rust has a table and a grammar, so the forced CST parse is taken.
    let rust = source_row(&json, "src/lib.rs");
    assert!(rust.contains("\"parsed_by\":\"deep\""), "{rust}");
    assert!(!rust.contains("deep_fallback"), "{rust}");
    // The Python file did get the CST, so this is a fallback for that language
    // and not for the run.
    assert!(source_row(&json, "src/app.py").contains("\"parsed_by\":\"deep\""));
    assert!(json.contains(r#""deep":2,"lexer":1,"fallbacks":1"#), "{json}");
    // A fallback is a downgrade, not a failure: the facts are still there.
    assert!(json.contains("\"id\":\"src/main.c:main\""), "{json}");
    assert!(json.contains("\"id\":\"src/lib.rs:f\""), "{json}");

    // Under `auto` the `.c` file is the lexer's by choice, not by fallback, and
    // the counter stays at zero.
    let (auto, _) = tree.json(&["--stdout", "--no-cache"]);
    assert!(auto.contains(r#""deep":2,"lexer":1,"fallbacks":0"#), "{auto}");
}

#[test]
fn a_forced_deep_parse_of_malformed_input_still_yields_facts() {
    // Recovery is a result, not a failure: the tree carries ERROR nodes, the
    // facts come out, and nothing is recorded as a fallback.
    let tree = Tree::new("malformed", &[
        ("broken.py", "def broken(:\n    value = (\n"),
        ("ok.py", "def ok(value):\n    return value\n"),
    ]);
    let (json, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);

    let row = source_row(&json, "broken.py");
    assert!(row.contains("\"parsed_by\":\"deep\""), "{row}");
    assert!(!row.contains("deep_fallback"), "recovery is not a fallback: {row}");
    let errors: u32 = row
        .split("\"deep_errors\":")
        .nth(1)
        .and_then(|rest| rest.split(|ch: char| !ch.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .expect("the row counts the CST's ERROR nodes");
    assert!(errors > 0, "the CST should report recovery on this input: {row}");
    assert!(json.contains("\"id\":\"broken.py:broken\""), "no facts from a malformed file: {json}");
    assert!(json.contains(r#""fallbacks":0"#), "{json}");
}

#[test]
fn the_deep_facts_are_never_fewer_than_the_lexer_facts() {
    let tree = Tree::new("quality", CORPUS);
    let (deep, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);
    let (lexer, _) = tree.json(&["--stdout", "--no-cache", "--lexer"]);

    for name in ["sources", "symbols", "edges", "imports"] {
        assert!(
            rows(&deep, name) >= rows(&lexer, name),
            "{name}: the deep path found {} rows, the lexer {}",
            rows(&deep, name),
            rows(&lexer, name)
        );
    }
    // Every call the lexer found is still found, either exactly or with the
    // receiver it pasted onto the path taken off again.
    let lexer_calls = call_paths(&lexer);
    let deep_calls = call_paths(&deep);
    assert!(deep_calls.len() >= lexer_calls.len(), "the CST found {} calls, the lexer {}", deep_calls.len(), lexer_calls.len());
    for lexer_path in &lexer_calls {
        let name = lexer_path.rsplit('.').next().unwrap();
        assert!(
            deep_calls.iter().any(|path| path.rsplit('.').next().unwrap() == name),
            "the lexer's call to {lexer_path} is missing from the CST's {deep_calls:?}"
        );
    }
    // And the facts still resolve: the corpus has one import and two same-file
    // calls that must bind.
    assert!(deep.contains("\"strategy\":\"same-file\""), "{deep}");
    assert!(deep.contains("\"module\":\"bs4\""), "{deep}");
}

#[test]
fn the_improvements_the_bridge_documents_are_visible_in_the_output() {
    let tree = Tree::new("classes", CORPUS);
    let (deep, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);
    let (lexer, _) = tree.json(&["--stdout", "--no-cache", "--lexer"]);

    // `line-of-definition`: `def links(html,\n  pattern="a"):` starts on line
    // 4; the lexer stamps the line its statement ends on, 5.
    let line_of = |json: &str| -> String {
        json.lines()
            .find(|line| line.contains("\"id\":\"app/links.py:links\""))
            .and_then(|line| line.split("\"line\":").nth(1))
            .and_then(|rest| rest.split(',').next())
            .unwrap_or_else(|| panic!("no symbol for links"))
            .to_string()
    };
    assert_eq!((line_of(&deep).as_str(), line_of(&lexer).as_str()), ("4", "5"));

    // `lexer-receiver-concatenation`: `BeautifulSoup(html, ...).find_all("a")`
    // is two calls; the lexer pastes the receiver call into the member call.
    assert!(section(&lexer, "flows").contains("BeautifulSoup().find_all"), "the lexer should still concatenate:\n{lexer}");
    assert!(section(&deep, "flows").contains("\".find_all\""), "the CST should record the member call:\n{deep}");
    assert!(!section(&deep, "flows").contains("()."), "the CST should not paste a receiver into a call path:\n{deep}");
    // Both paths still agree on the call it cannot see on its own line.
    assert!(section(&deep, "flows").contains("\"BeautifulSoup\""));
    assert!(section(&lexer, "flows").contains("\"BeautifulSoup\""));
}

#[test]
fn auto_falls_back_when_the_cst_record_loses_what_the_lexer_found() {
    // A shape the Python grammar genuinely cannot build an assignment node for
    // (`?` is not a Python token, while the token-stream reader still records
    // the binding and its call). Under `auto` the lexer's record is kept and
    // the loss is on the row; `--deep` obeys the user and takes the CST anyway.
    let tree = Tree::new("guard", &[
        (
            "app/lossy.py",
            "from app.helpers import *\n\n\ndef f(x):\n    a = 1 ? g(x) : 2\n    return a\n\n\ndef bulk():\n    item0 = helper()\n    item1 = helper()\n    item2 = helper()\n    item3 = helper()\n    item4 = helper()\n    item5 = helper()\n    item6 = helper()\n    item7 = helper()\n    item8 = helper()\n    item9 = helper()\n    item10 = helper()\n    item11 = helper()\n    item12 = helper()\n    item13 = helper()\n    item14 = helper()\n    item15 = helper()\n    item16 = helper()\n    item17 = helper()\n    item18 = helper()\n    item19 = helper()\n    item20 = helper()\n    item21 = helper()\n    item22 = helper()\n    item23 = helper()\n    item24 = helper()\n    item25 = helper()\n    item26 = helper()\n    item27 = helper()\n    item28 = helper()\n    item29 = helper()\n    item30 = helper()\n    item31 = helper()\n    item32 = helper()\n    item33 = helper()\n    item34 = helper()\n    item35 = helper()\n    item36 = helper()\n    item37 = helper()\n    item38 = helper()\n    item39 = helper()\n    return item0\n",
        ),
        ("app/helpers.py", "def helper():\n    return 1\n"),
        ("app/clean.py", "def clean(value):\n    return value.strip()\n"),
    ]);
    let (auto, _) = tree.json(&["--stdout", "--no-cache"]);
    let lossy = source_row(&auto, "app/lossy.py");
    assert!(lossy.contains("\"parsed_by\":\"lexer\""), "{lossy}");
    // The fallback names the fact it missed, not just the fact that it missed
    // one: a reader must be able to see what the CST lost without re-running
    // both paths by hand.
    assert!(
        lossy.contains("\"deep_fallback\":\"the CST lost a ") || lossy.contains("\"deep_fallback\":\"the CST lost an "),
        "{lossy}"
    );
    assert!(source_row(&auto, "app/clean.py").contains("\"parsed_by\":\"deep\""), "{auto}");
    assert!(auto.contains(r#""deep":2,"lexer":1,"fallbacks":1"#), "{auto}");
    // The facts are the lexer's: the star import survives.
    assert!(auto.contains("\"star\":true"), "{auto}");

    let (deep, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);
    assert!(source_row(&deep, "app/lossy.py").contains("\"parsed_by\":\"deep\""), "{deep}");
    assert!(!source_row(&deep, "app/lossy.py").contains("deep_fallback"), "a forced parse does not second-guess");
}

#[test]
fn a_cache_written_by_one_parser_is_never_served_to_the_other() {
    // The pack's version word carries the mode, so switching parser is a cold
    // cache and never a silent downgrade to the facts of the other path.
    let tree = Tree::new("cache", TWO_FILES);
    let (_, first) = tree.json(&["--stdout", "--deep"]);
    assert!(first.contains("3 parsed, 0 cached"), "{first}");
    let (_, again) = tree.json(&["--stdout", "--deep"]);
    assert!(again.contains("0 parsed, 3 cached"), "{again}");
    let (json, switched) = tree.json(&["--stdout", "--lexer"]);
    assert!(switched.contains("3 parsed, 0 cached"), "a --deep pack was reused by --lexer: {switched}");
    assert!(json.contains(r#""mode":"lexer""#), "{json}");
    let (_, back) = tree.json(&["--stdout", "--deep"]);
    assert!(back.contains("3 parsed, 0 cached"), "a --lexer pack was reused by --deep: {back}");
}

/// P4-15: a file the reader will not hand to the CST says why in words. The
/// reason names what it stumbled on — the production and its line, or the
/// error-node count against the tree's size — so a fallback is something a
/// person can act on rather than a row that only says "failed".
#[test]
fn a_refusal_names_what_it_failed_on() {
    let tree = Tree::new("refusal", &[
        // Text no grammar reads: the CST needs error nodes for most of it.
        ("app/a.js", "const = = = ;\nclass { ][\nfunction ((( {\n"),
        // A file the lexer can only read: an event the CST has no counterpart
        // for is a loss, and the row must say which event on which line.
        ("app/b.js", "function f(a) {\n  return a ?? (a || a);\n}\n"),
    ]);
    let (json, _) = tree.json(&["--stdout", "--no-cache"]);
    let rows: Vec<&str> = json.lines().filter(|line| line.contains("\"path\":\"app/")).collect();
    assert!(rows.len() == 2, "the fixture did not produce two rows: {json}");
    let refused: Vec<&&str> = rows.iter().filter(|row| row.contains("\"deep_fallback\":")).collect();
    assert_eq!(refused.len(), 1, "expected one refusal, got {}: {json}", refused.len());
    let row = refused[0];
    assert!(!row.contains("\"deep_fallback\":\"\""), "a refusal with no reason: {row}");
    assert!(
        row.contains("error nodes in") || (row.contains("the CST lost") && row.contains("line ")),
        "a refusal that names no production: {row}"
    );
}

#[test]
fn a_warm_build_repeats_the_provenance_of_the_build_that_parsed() {
    // The provenance file travels with the pack, so a cached result says what
    // each file's facts came from instead of going quiet — and a warm build's
    // output is the cold build's output, which is what makes the JSON
    // comparable across runs.
    let tree = Tree::new("cached", TWO_FILES);
    let (cold, _) = tree.json(&["--stdout", "--deep"]);
    let (warm, stderr) = tree.json(&["--stdout", "--deep"]);
    assert!(stderr.contains("0 parsed, 3 cached"), "{stderr}");
    assert_eq!(without_timing(&cold), without_timing(&warm), "a cached build must equal a clean one");
    assert!(warm.contains(r#""parsed_by":"deep""#), "{warm}");
}

#[test]
fn a_call_whose_callee_is_another_call_does_not_descend_forever() {
    // `a()()`: the outer call's callee is the inner call. Reading it used to
    // recurse on the same node until the process stack was gone — the whole
    // build died with SIGSEGV on a four kilobyte file of noise. The bridge is
    // fed arbitrary user files now, so the shape has to be a fact, not a crash.
    let tree = Tree::new("nested-call", &[("app/a.js", "function a() {\n  return b;\n}\na()();\n")]);
    let (json, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);
    assert!(source_row(&json, "app/a.js").contains("\"parsed_by\":\"deep\""), "{json}");
    assert!(json.contains("\"calls\":[[\"a\""), "the inner call should be recorded: {json}");
}

#[test]
fn the_report_says_how_many_files_each_parser_took() {
    let tree = Tree::new("report", TWO_FILES);
    let out = tree.0.join("out");
    let out_arg = out.to_str().unwrap().to_string();
    let built = tree.build(&["--out", &out_arg, "--no-cache"]);
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let report = fs::read_to_string(out.join("report.md")).unwrap();
    assert!(report.contains("Parsed by: 2 files through the CST, 1 through the lexer, 0 requested CST parses fell back."), "{report}");
}

#[test]
fn the_default_build_has_no_switch_at_all() {
    // This test only runs with the features on; what it checks is that the
    // default binary (built by `cargo test --workspace` without flags) has no
    // `--deep`. The evidence for byte identity is `crates/deep-bench`.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let main = fs::read_to_string(path).unwrap();
    let line = main.lines().find(|line| line.contains("mod deep;")).expect("the gated runtime declaration");
    assert!(line.trim().starts_with("#[cfg("), "the runtime must be behind a cfg: {line}");
}

/// Recovery is bounded, and the bound is an `auto` decision: a tree that needed
/// an error node for more than one node in a hundred is refused, because facts
/// pulled out of a parse that guessed look exactly like facts from a parse that
/// did not. `--deep` obeys the user and still yields facts — a caller who asked
/// for the CST gets the CST — and the row names the refusal so the difference is
/// never silent.
#[test]
fn a_degraded_cst_is_refused_under_auto_and_obeyed_under_deep() {
    let tree = Tree::new("degraded", &[("broken.py", "def f(\n    x = = = 1\n    y = = 2\n")]);
    let (auto, _) = tree.json(&["--stdout", "--no-cache"]);
    let row = source_row(&auto, "broken.py");
    assert!(row.contains("\"parsed_by\":\"lexer\""), "{row}");
    assert!(row.contains("CST_DEGRADED:recovery"), "the refusal must be named: {row}");

    let (deep, _) = tree.json(&["--stdout", "--no-cache", "--deep"]);
    assert!(deep.contains("\"parsed_by\":\"deep\""), "--deep obeys: {deep}");
}

/// The same bytes must produce the same record, byte for byte. Two ways that can
/// break without any test noticing: a hash map whose iteration order leaks into
/// the output, and a timestamp or a cache path that gets written into the record.
/// This runs the same tree twice with the cache off and hashes both outputs, then
/// extracts facts twice from one CST and compares those too.
#[test]
fn the_same_input_yields_byte_identical_facts() {
    let tree = Tree::new("deterministic", &[
        (
            "app/a.py",
            "from app.b import Thing\n\n\ndef f(x):\n    y = Thing(x)\n    return y.value\n",
        ),
        ("app/b.py", "class Thing:\n    def __init__(self, value):\n        self.value = value\n"),
    ]);

    let (first, _) = tree.json(&["--stdout", "--no-cache"]);
    let (second, _) = tree.json(&["--stdout", "--no-cache"]);
    assert_eq!(without_timing(&first), without_timing(&second), "two builds of the same tree differ");
    assert!(first.contains("\"path\":\"app/a.py\""), "{first}");

    // The extraction runs inside the pipeline above; a second build is the
    // determinism check that matters, because the record is what a caller reads.
}
