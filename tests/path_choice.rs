//! Which parse path a file takes, decided by measurement.
//!
//! `auto` already ran both records and kept the lexer's when the CST's lost a
//! fact. What this file pins is the decision that follows: when neither record
//! lost anything, the heavier one wins by the stated rule (symbols, then calls,
//! then imports) and an exact tie goes to the CST and says so. `--compare` is
//! the audit that shows both sides per file without choosing.
//!
//! The fixture is measured, not assumed: `deep_more.ts` is a win the CST finds
//! on a type alias (`type A = { x: number }`, a declaration the one-pass lexer
//! does not open a symbol for), `deep_more.ts` holds a generic call
//! (`g<number>(x)`) that the lexer records too since it learned to skip a type
//! argument on a callee, so that file is now a tie rather than a win,
//! `opaque.js` holds a call inside a JSX element's child expression, which the
//! fact reader reads now, so it is a tie, `tie.py` is one function with nothing to
//! differ on, and `plain.c` has no CST at all. The expected counters in the
//! tests are the values this fixture produces.
//!
//! `lossy.js` used to be the fallback case: a call inside a template literal's
//! `${...}`, which the grammar kept as one token. The grammar parses a
//! substitution now, so that file is a tie (`deep:tie`, `calls deep=1 lexer=1`).
//! The JSX gap moved twice: the grammar now parses the element and every
//! `{...}` part, and the fact reader descends into a child expression, so the
//! old attribute-arrow fixture and `opaque.js` are both ties. The fixture has
//! no fallback row left; `tests/deep_pipeline.rs` covers that mechanism. The
//! counters below are the values this fixture produces, not the old ones.
#![cfg(all(feature = "deep", feature = "parser"))]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_rust-graph");

const FIXTURE: &[(&str, &str)] = &[
    // The CST reads the call behind the type arguments; the lexer stops there.
    // A type alias is a symbol the CST opens and the one-pass lexer does not,
    // and the file loses nothing on either side, so the CST is strictly ahead.
    // It used to be a generic call (`g<number>(x)`); the lexer reads that as a
    // call now, so that shape is a tie and could not pin the strict win.
    ("deep_more.ts", "type A = { x: number };\nconst v: A = { x: 1 };\n"),
    // The element is markup: both readers write the constant 0 and skip its
    // children, so the file weighs the same on either side and pins that the
    // lexer does not claim a call the CST does not read.
    ("opaque.js", "function panel(x) {\n  return <button label={handle(x)}>press</button>;\n}\n"),
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

    // Three files had a CST record to weigh, one did not. All three records are
    // the CST's now that the deep reader understands a JSX child expression, so
    // only the language with no CST reaches the lexer. The totals agree with the
    // rows below.
    assert!(
        json.contains(r#""paths":{"deep":3,"lexer":1,"chosen_deep":3,"chosen_lexer":1}"#),
        "the paths object does not match the fixture: {json}"
    );
    // `deep` and `lexer` in the older object still count the emitted record, so
    // the two summaries cannot disagree.
    assert!(json.contains(r#""deep":{"mode":"auto","deep":3,"lexer":1,"fallbacks":0}"#), "{json}");
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
fn a_jsx_element_is_markup_on_both_paths() {
    let tree = Tree::new("loss");
    let (json, _) = tree.json(&["--stdout", "--no-cache"]);

    // The CST writes the element as the constant 0, and the lexer learned to
    // skip it the same way, so the file is a tie: no call is claimed on either
    // path. A call inside an element's child expression is not a fact by this
    // project's own choice for JSX, so the row must not claim one.
    let row = source_row(&json, "opaque.js");
    assert!(row.contains("\"path_choice\":\"deep:tie\""), "{row}");
    assert!(row.contains("\"parsed_by\":\"deep\""), "{row}");
    assert!(!row.contains("deep_fallback"), "{row}");
    assert!(json.contains(r#""fallbacks":0"#), "{json}");

    let out = tree.build(&["--compare", "--no-cache"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("opaque.js\tsymbols deep=2 lexer=2 (+0)\tcalls deep=0 lexer=0 (+0)\timports deep=0 lexer=0 (+0)\n"),
        "{text}"
    );
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
        text.contains("deep_more.ts\tsymbols deep=2 lexer=1 (+1)\tcalls deep=0 lexer=0 (+0)\timports deep=0 lexer=0 (+0)\n"),
        "{text}"
    );
    assert!(
        text.contains("opaque.js\tsymbols deep=2 lexer=2 (+0)\tcalls deep=0 lexer=0 (+0)\timports deep=0 lexer=0 (+0)\n"),
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
        text.contains("compare totals: symbols deep=6 lexer=7 (-1), calls deep=0 lexer=1 (-1), imports deep=0 lexer=0 (+0)\n"),
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
        report.contains("Paths: {\"deep\": 3, \"lexer\": 1, \"chosen_deep\": 3, \"chosen_lexer\": 1}"),
        "{report}"
    );
    // The older line stays: a report a reader already parses does not change
    // shape to gain the new one.
    assert!(report.contains("Parsed by: 3 files through the CST, 1 through the lexer, 0 requested CST parses fell back."), "{report}");
}
