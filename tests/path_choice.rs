//! Which parse path a file takes, decided by measurement.
//!
//! `auto` already ran both records and kept the lexer's when the CST's lost a
//! fact. What this file pins is the decision that follows: when neither record
//! lost anything, the heavier one wins by the stated rule (symbols, then calls,
//! then imports) and an exact tie goes to the CST and says so. `--compare` is
//! the audit that shows both sides per file without choosing.
//!
//! The fixture is measured, not assumed: `deep_more.ts` is a win the CST finds
//! on a generic call (`g<number>(x)`, which the one-pass lexer does not record),
//! `opaque.js` holds a call inside a JSX element's attribute, which the
//! JavaScript grammar keeps as one token, `tie.py` is one function with nothing
//! to differ on, and `plain.c` has no CST at all. The expected counters in the
//! tests are the values this fixture produces.
//!
//! `lossy.js` used to be the fallback case: a call inside a template literal's
//! `${...}`, which the grammar kept as one token. The grammar parses a
//! substitution now, so that file is a tie (`deep:tie`, `calls deep=1 lexer=1`)
//! and the fallback moved to the JSX gap, which is still open. The counters
//! below are a fresh measurement of the moved fixture, not the old numbers.
#![cfg(all(feature = "deep", feature = "parser"))]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_rust-graph");

const FIXTURE: &[(&str, &str)] = &[
    // The CST reads the call behind the type arguments; the lexer stops there.
    ("deep_more.ts", "import { a, b } from \"./m\";\n\nconst x: number = f(1);\nexport const y = g<number>(x);\n"),
    // A JSX element is one token to the deep grammar, so a call inside its
    // attribute is a fact only the lexer's record carries; the guard keeps it.
    ("opaque.js", "function panel(x) {\n  return <button onClick={() => handle(x)}>press</button>;\n}\n"),
    // Nothing to differ on: both records weigh the same.
    ("tie.py", "def f(x):\n    return x\n"),
    // No deep parser for C: the lexer stands alone and is not a comparison.
    ("plain.c", "int main(void) {\n  return helper(1);\n}\n"),
];

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Tree {
        let dir = std::env::temp_dir().join(format!("rust-graph-path-choice-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, body) in FIXTURE {
            let target = dir.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, body).unwrap();
        }
        Tree(dir)
    }

    fn build(&self, extra: &[&str]) -> Output {
        Command::new(BIN).arg("build").arg(&self.0).args(extra).output().unwrap()
    }

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

#[test]
fn the_paths_object_counts_candidates_and_choices() {
    let tree = Tree::new("counts");
    let (json, _) = tree.json(&["--stdout", "--no-cache"]);

    // Three files had a CST record to weigh, one did not. Two records were the
    // CST's (the strict win and the tie), two the lexer's (the lost-fact file
    // and the language with no CST). The totals agree with the rows below.
    assert!(
        json.contains(r#""paths":{"deep":3,"lexer":1,"chosen_deep":2,"chosen_lexer":2}"#),
        "the paths object does not match the fixture: {json}"
    );
    // `deep` and `lexer` in the older object still count the emitted record, so
    // the two summaries cannot disagree.
    assert!(json.contains(r#""deep":{"mode":"auto","deep":2,"lexer":2,"fallbacks":1}"#), "{json}");
}

#[test]
fn a_strict_win_goes_to_the_cst_and_says_which_count_won() {
    let tree = Tree::new("strict");
    let (json, _) = tree.json(&["--stdout", "--no-cache"]);

    for path in ["deep_more.ts"] {
        let row = source_row(&json, path);
        assert!(row.contains("\"path_choice\":\"deep:more\""), "{path}: {row}");
        assert!(row.contains("\"parsed_by\":\"deep\""), "{path}: {row}");
        assert!(!row.contains("deep_fallback"), "{path} lost nothing, so it is not a fallback: {row}");
    }
}

#[test]
fn a_lost_fact_keeps_the_lexer_and_is_counted_as_a_fallback() {
    let tree = Tree::new("loss");
    let (json, _) = tree.json(&["--stdout", "--no-cache"]);

    let row = source_row(&json, "opaque.js");
    assert!(row.contains("\"path_choice\":\"lexer:fallback\""), "{row}");
    assert!(row.contains("\"parsed_by\":\"lexer\""), "{row}");
    assert!(row.contains("\"deep_fallback\":\"the CST record lost facts the lexer found\""), "{row}");
    // The call inside the JSX attribute is only in the lexer's record, so it
    // survives the fallback.
    assert!(json.contains(r#""calls":[["handle",2,null,null,0]]"#), "{json}");
    assert!(json.contains(r#""fallbacks":1"#), "{json}");
}

#[test]
fn an_exact_tie_goes_to_the_cst_and_the_row_says_so() {
    let tree = Tree::new("tie");
    let (json, _) = tree.json(&["--stdout", "--no-cache"]);

    let row = source_row(&json, "tie.py");
    assert!(row.contains("\"path_choice\":\"deep:tie\""), "a tie must be named, not silent: {row}");
    assert!(row.contains("\"parsed_by\":\"deep\""), "{row}");
    // No CST record: the lexer is the only candidate, not a tie and not a
    // fallback, so the row carries no choice at all.
    let plain = source_row(&json, "plain.c");
    assert!(!plain.contains("path_choice"), "{plain}");
    assert!(plain.contains("\"parsed_by\":\"lexer\""), "{plain}");
}

#[test]
fn compare_prints_both_sides_per_file_and_chooses_nothing() {
    let tree = Tree::new("compare");
    let out = tree.build(&["--compare", "--no-cache"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();

    // Every file, its three weights on each path and the difference. The
    // numbers are the fixture's measurements, so a change in either parser
    // moves a line here rather than passing quietly.
    assert!(
        text.contains("deep_more.ts\tsymbols deep=1 lexer=1 (+0)\tcalls deep=2 lexer=1 (+1)\timports deep=2 lexer=2 (+0)\n"),
        "{text}"
    );
    assert!(
        text.contains("opaque.js\tsymbols deep=2 lexer=2 (+0)\tcalls deep=0 lexer=1 (-1)\timports deep=0 lexer=0 (+0)\n"),
        "{text}"
    );
    assert!(
        text.contains("plain.c\tno CST record\tsymbols lexer=2\tcalls lexer=1\timports lexer=0\n"),
        "{text}"
    );
    assert!(
        text.contains("tie.py\tsymbols deep=2 lexer=2 (+0)\tcalls deep=0 lexer=0 (+0)\timports deep=0 lexer=0 (+0)\n"),
        "{text}"
    );
    assert!(text.contains("compare: 4 files, 3 with a CST record, 1 lexer-only\n"), "{text}");
    assert!(
        text.contains("compare totals: symbols deep=5 lexer=7 (-2), calls deep=2 lexer=3 (-1), imports deep=2 lexer=2 (+0)\n"),
        "{text}"
    );

    // No choice was made and nothing was built: no graph, no report, and no
    // cache directory was created in the tree.
    assert!(!tree.0.join(".rust-graph").exists(), "a comparison run wrote a cache");
    assert!(!text.contains("\"version\""), "a comparison run emitted a graph:\n{text}");
}

#[test]
fn compare_ignores_a_warm_cache_and_leaves_it_alone() {
    // A normal build writes a pack and its provenance. A comparison must read
    // the tree as it stands rather than trusting either, and must not replace
    // them: its own header word is distinct and it returns before the save.
    let tree = Tree::new("warm-compare");
    let (_, built) = tree.json(&["--stdout", "--deep"]);
    assert!(built.contains("4 parsed, 0 cached"), "{built}");
    let cache = tree.0.join(".rust-graph");
    let pack = fs::read(cache.join("facts.pack")).unwrap();
    let meta = fs::read_to_string(cache.join("facts.meta")).unwrap();

    let out = tree.build(&["--compare"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // Four rows proves the tree was read: a reused pack records no comparison,
    // so the audit would say `compare: 0 files`.
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("compare: 4 files"), "the comparison trusted the pack: {text}");
    assert_eq!(fs::read(cache.join("facts.pack")).unwrap(), pack, "the comparison rewrote the pack");
    assert_eq!(fs::read_to_string(cache.join("facts.meta")).unwrap(), meta, "the comparison rewrote the provenance");
    assert_eq!(fs::read(cache.join("facts.pack")).unwrap(), pack, "the comparison rewrote the pack");
    assert_eq!(fs::read_to_string(cache.join("facts.meta")).unwrap(), meta, "the comparison rewrote the provenance");
}

#[test]
fn the_report_states_the_same_split_as_the_json() {
    let tree = Tree::new("report");
    let out = tree.0.join("out");
    let built = tree.build(&["--out", out.to_str().unwrap(), "--no-cache"]);
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let report = fs::read_to_string(out.join("report.md")).unwrap();
    assert!(
        report.contains("Paths: {\"deep\": 3, \"lexer\": 1, \"chosen_deep\": 2, \"chosen_lexer\": 2}"),
        "{report}"
    );
    // The older line stays: a report a reader already parses does not change
    // shape to gain the new one.
    assert!(report.contains("Parsed by: 2 files through the CST, 2 through the lexer, 1 requested CST parses fell back."), "{report}");
}
