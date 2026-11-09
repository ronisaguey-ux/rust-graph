//! The guard's own truth table.
//!
//! `pipeline::loses` decides whether a CST record may replace the lexer's. The
//! three defects this file pins are behavioural, not cosmetic:
//!
//! 1. a value the CST wrote as `0` where the lexer followed it (`f"{a}"`) is a
//!    loss even though both records write the same number of events;
//! 2. a lexer call with no name in the source (`.call`) and a lexer call to a
//!    statement keyword (`assert`) are the lexer's artefacts, not facts the
//!    CST lost — they are accepted and counted, as is an assignment target the
//!    lexer read off an annotation (`x: Dict[str, int] = {}` binds `Dict`);
//! 3. a class binding (`repo = Repo()`) the CST misses is a loss, because the
//!    resolver's typed-variable edges come from `Facts::vars`.
//!
//! Two controls keep the acceptance from swallowing real differences: a call
//! only the lexer can see (`f"{len(x)}"`) still falls back, and a file both
//! readers agree on is neither a loss nor an acceptance.
//!
//! The fixtures below are compiled from the two real readers, not from hand
//! written `Facts`, so a grammar change moves these tests with it.
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
#[path = "../src/deep_facts.rs"]
mod deep_facts;
#[path = "../src/pipeline.rs"]
mod pipeline;

// `src/main.rs` gives the crate root the name `crate::deep_integration` as a
// glob of the one-name bridge module, so `deep_facts` reads the same copy the
// pipeline does. Without this the test would compile two bridges and their
// `Lang` types would not meet.
use pipeline::gate::*;

use deep_integration::Lang;
use facts::Facts;

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("py", source.as_bytes()).expect("a Python row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Python).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Python)
}

fn events(facts: &Facts, name: &str) -> String {
    facts
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .map(|symbol| symbol.events.clone())
        .unwrap_or_else(|| panic!("no symbol {name} in {facts:?}"))
}

fn calls(facts: &Facts) -> Vec<String> {
    facts.symbols.iter().flat_map(|symbol| symbol.calls.iter().map(|call| call.path.clone())).collect()
}

/// Defect 1: the same event count, a different value. `f"{a}"` is one `r` event
/// on both paths; the lexer follows the interpolation to `a`, the grammar keeps
/// the whole string as one token and the CST writes `0`.
#[test]
fn a_value_the_cst_wrote_as_a_constant_falls_back() {
    let source = "def f(x):\n    a, b = g(x), 1\n    return f\"{a}\"\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);

    // The fixture is honest about both records: same events, different value.
    assert!(events(&lexer, "f").contains(r#"["r",3,["n","a"]]"#), "{}", events(&lexer, "f"));
    assert!(events(&deep, "f").contains(r#"["r",3,0]"#), "{}", events(&deep, "f"));
    // The multi-target assignment is mapped by both paths, so the *only*
    // difference here is the value the CST cannot read.
    assert!(events(&lexer, "f").contains(r#"["a",2,["a","b"]"#), "{}", events(&lexer, "f"));
    assert!(events(&deep, "f").contains(r#"["a",2,["a","b"]"#), "{}", events(&deep, "f"));

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(verdict.lost(), "a value the CST wrote as 0 is a loss: {verdict:?}");
    assert_eq!(verdict.why(), Some("event"));
    assert_eq!(verdict.accepted(), pipeline::Accepted::default(), "nothing here is a lexer artefact");
}

/// Defect 2, first shape: `f(x)(y)` has no name for the outer call, so the
/// lexer invents `.call`. The CST must not be asked to reproduce it.
#[test]
fn an_anonymous_call_is_an_accepted_difference() {
    let source = "def f(x):\n    return x\ndef g(y):\n    return f(x)(y)\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);

    assert!(calls(&lexer).iter().any(|path| path == ".call"), "the lexer invents .call: {:?}", calls(&lexer));
    assert!(!calls(&deep).iter().any(|path| path == ".call"), "the CST has no such name: {:?}", calls(&deep));

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "an invented name is not a lost fact: {verdict:?}");
    let accepted = verdict.accepted();
    assert_eq!(accepted.anonymous_calls, 1);
    assert_eq!(accepted.keyword_calls, 0);
    assert_eq!(accepted.keyword_events, 0);
    assert_eq!(accepted.total(), 1);
}

/// Defect 2, second shape: `assert (a, b) == c` is a statement, and the lexer
/// reads it as a call to `assert` and as an expression event.
#[test]
fn a_statement_keyword_read_as_a_call_is_an_accepted_difference() {
    let source = "def h():\n    assert (a, b) == c\n    return 1\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);

    assert!(calls(&lexer).iter().any(|path| path == "assert"), "the lexer invents the call: {:?}", calls(&lexer));
    assert!(!calls(&deep).iter().any(|path| path == "assert"), "the CST does not: {:?}", calls(&deep));
    assert!(events(&lexer, "h").contains(r#"["x",2,"#), "the lexer writes the statement as an expression: {}", events(&lexer, "h"));
    assert!(!events(&deep, "h").contains(r#"["x",2,"#), "the CST writes no expression event: {}", events(&deep, "h"));

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "a keyword statement is not a call: {verdict:?}");
    let accepted = verdict.accepted();
    assert_eq!(accepted.keyword_calls, 1);
    assert_eq!(accepted.keyword_events, 1);
    assert_eq!(accepted.anonymous_calls, 0);
    assert_eq!(accepted.total(), 2);
}

/// Defect 2 in one file: both artefacts together, and the counts are per kind.
#[test]
fn both_phantoms_are_accepted_and_counted_by_kind() {
    let source = "def f(x):\n    return x\ndef g(y):\n    return f(x)(y)\ndef h():\n    assert (a, b) == c\n    return 1\n";
    let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
    assert!(!verdict.lost(), "{verdict:?}");
    let accepted = verdict.accepted();
    assert_eq!((accepted.anonymous_calls, accepted.keyword_calls, accepted.keyword_events), (1, 1, 1));
    assert_eq!(accepted.total(), 3);
}

/// Defect 3: the class binding. `Facts::vars` is what turns `repo.find()` into a
/// `Repo.find` edge, so a binding the CST misses must keep the lexer's record.
#[test]
fn a_missing_class_binding_falls_back() {
    let source = "def f():\n    repo, other = Repo(), 1\n    return repo.find(other)\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);
    // The lexer reads the last name of `repo, other` as the target and binds it
    // to the constructed class; the CST reads both targets but only binds a
    // class when the value *starts* with the constructor, and here it starts
    // with a call in a tuple.
    assert!(
        lexer.vars.iter().any(|(name, class)| name == "other" && class == "Repo"),
        "the lexer binds the target: {:?}",
        lexer.vars
    );
    assert!(
        !deep.vars.iter().any(|(name, class)| name == "other" && class == "Repo"),
        "the CST misses the binding this fixture needs: {:?}",
        deep.vars
    );

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(verdict.lost(), "a lost typed variable is a lost fact: {verdict:?}");
    assert_eq!(verdict.why(), Some("var"));
}

/// A lexer assignment event whose target is an imported type, not a binder:
/// `table: Dict[str, int] = {...}` is read by the one-pass lexer as an
/// assignment to `Dict`. The CST writes the real name and the same value, so the
/// statement is in the record — the invented binder is accepted and counted.
#[test]
fn an_annotation_name_read_as_a_target_is_an_accepted_difference() {
    let source = "from typing import Dict\n\ndef f():\n    table: Dict[str, int] = {\"a\": 1}\n    return table\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);

    assert!(events(&lexer, "f").contains(r#"["a",4,["Dict"]"#), "the lexer reads the annotation as the target: {}", events(&lexer, "f"));
    assert!(events(&deep, "f").contains(r#"["a",4,["table"]"#), "the CST reads the name: {}", events(&deep, "f"));

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "an imported type is not a binding: {verdict:?}");
    let accepted = verdict.accepted();
    assert_eq!(accepted.annotation_targets, 1);
    assert_eq!(accepted.total(), 1);
}

/// A real lost call still keeps the lexer's record: the grammar keeps an
/// f-string as one token, so the call inside it never reaches the CST walk.
#[test]
fn a_call_only_the_lexer_saw_falls_back() {
    let source = "def f(x):\n    return f\"{len(x)}\"\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);
    assert!(calls(&lexer).iter().any(|path| path == "len"), "{:?}", calls(&lexer));
    assert!(!calls(&deep).iter().any(|path| path == "len"), "{:?}", calls(&deep));

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(verdict.lost(), "a call the CST cannot see is a loss: {verdict:?}");
    assert_eq!(verdict.why(), Some("call"));
    assert!(verdict.detail().contains("len"), "{}", verdict.detail());
}

/// The control: a file the two readers agree on is not a fallback, and the guard
/// does not invent an acceptance where there is no difference.
#[test]
fn agreement_is_neither_a_loss_nor_an_acceptance() {
    let source = "def f(x):\n    y = g(x)\n    return y\n";
    let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
    assert!(!verdict.lost(), "{verdict:?}");
    assert_eq!(verdict.accepted(), pipeline::Accepted::default());
}

/// The same guard, over a real tree: every file's verdict, the check that fired,
/// and the lexer artefacts accepted in its place.
///
/// ```text
/// FACT_GUARD_DIR=/path/to/tree cargo test --features "deep parser" \
///     --test guard_truth -- --ignored --nocapture
/// ```
///
/// `FACT_GUARD_DUMP=<suffix>` prints both records for the files whose suffix
/// matches, which is how a fallback is explained rather than counted. The test
/// asserts the partition is total — a loss always names its check — so a new
/// verdict without a reason fails here.
#[test]
#[ignore = "reads a tree outside this repository"]
fn the_real_tree_is_classified_by_its_verdict() {
    let Some(root) = std::env::var_os("FACT_GUARD_DIR") else { return };
    let root = std::path::PathBuf::from(root);
    let dump = std::env::var("FACT_GUARD_DUMP").ok();
    let mut paths = Vec::new();
    collect(&root, &mut paths);
    paths.sort();
    assert!(!paths.is_empty(), "no source files under {root:?}");

    let (mut kept, mut lost) = (0u32, 0u32);
    let mut by_why: std::collections::BTreeMap<&'static str, u32> = Default::default();
    let mut accepted = [0u32; 4];
    let mut lines = Vec::new();
    for path in &paths {
        let bytes = std::fs::read(path).expect("a readable file");
        let Some(lang) = lang_of(path) else { continue };
        let language = langs::by_content("py", &bytes[..bytes.len().min(4096)]).expect("a lexer row");
        let lexer = parser::parse(&bytes, language);
        let Some(cst) = deep_integration::parse_file(&bytes, lang) else { continue };
        let deep = deep_facts::extract(&bytes, &cst, lang);
        let verdict = pipeline::loses(&lexer, &deep);
        let shown = path.strip_prefix(&root).unwrap_or(path).display().to_string();
        let got = verdict.accepted();
        accepted[0] += got.anonymous_calls;
        accepted[1] += got.keyword_calls;
        accepted[2] += got.keyword_events;
        accepted[3] += got.annotation_targets;
        if verdict.lost() {
            lost += 1;
            let why = verdict.why().expect("a loss names the check that fired");
            *by_why.entry(why).or_default() += 1;
            lines.push(format!("{shown}\tfallback\t{why}\t{}\t{}", got.total(), verdict.detail()));
            if dump.as_deref().is_some_and(|suffix| shown.ends_with(suffix)) {
                for symbol in &lexer.symbols {
                    let paired = deep.symbols.iter().find(|row| row.name == symbol.name && row.kind == symbol.kind);
                    lines.push(format!("  {} L calls={:?} events={}", symbol.name, symbol.calls.iter().map(|call| (&call.path, call.line)).collect::<Vec<_>>(), symbol.events));
                    if let Some(paired) = paired {
                        lines.push(format!(
                            "  {} D calls={:?} events={}",
                            paired.name,
                            paired.calls.iter().map(|call| (&call.path, call.line)).collect::<Vec<_>>(),
                            paired.events
                        ));
                    } else {
                        lines.push(format!("  {} has no CST symbol", symbol.name));
                    }
                }
                lines.push(format!("  vars L={:?} D={:?}", lexer.vars, deep.vars));
            }
        } else {
            kept += 1;
            lines.push(format!("{shown}\tdeep\t\t{}", got.total()));
        }
    }
    for line in &lines {
        println!("{line}");
    }
    println!(
        "guard: {} files, {kept} kept the CST record, {lost} fell back; by check {:?}; accepted anonymous={} keyword_call={} keyword_event={} annotation_target={}",
        kept + lost,
        by_why,
        accepted[0],
        accepted[1],
        accepted[2],
        accepted[3]
    );
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect(&path, out),
            Ok(kind) if kind.is_file() && lang_of(&path).is_some() => out.push(path),
            _ => {}
        }
    }
}

fn lang_of(path: &std::path::Path) -> Option<Lang> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py") => Some(Lang::Python),
        Some("js") => Some(Lang::JavaScript),
        Some("ts") => Some(Lang::TypeScript),
        Some("tsx") => Some(Lang::Tsx),
        _ => None,
    }
}

/// The machine check for the note `assignment_targets` used to carry: the left
/// side of `a, b = g(x), 1` is a `pattern_list` in this grammar, an arm the
/// walk already maps, so the record carries both target names. `expression_list`
/// is the *value* form, read by `form`, not a target node.
#[test]
fn the_multi_target_mapping_is_enabled_and_carries_both_names() {
    let source = "def f(x):\n    a, b = g(x), 1\n    return x\n";
    let deep = deep_facts(source);
    assert!(events(&deep, "f").contains(r#"["a",2,["a","b"],"#), "{}", events(&deep, "f"));

    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Python).expect("a deep parse");
    let language = Lang::Python.language();
    let assignment = cst
        .nodes
        .iter()
        .find(|node| language.name(node.kind) == "assignment")
        .expect("the fixture holds one assignment");
    let first = cst.nodes[assignment.first_child as usize];
    assert_eq!(language.name(first.kind), "pattern_list", "the target node is the arm that is mapped");
}
