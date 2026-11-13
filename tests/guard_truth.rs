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
#[path = "../src/deep_facts/mod.rs"]
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

/// Defect 1, now closed: `f"{a}"` used to reach the CST as the constant `0`
/// because the field selection asked for a part with children of its own, and a
/// leaf identifier has none. Both paths read the interpolation today, so this
/// checks the fix rather than the loss it used to cause.
#[test]
fn an_interpolation_field_is_read_by_both_paths() {
    let source = "def f(x):\n    a, b = g(x), 1\n    return f\"{a}\"\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);

    assert!(events(&lexer, "f").contains(r#"["r",3,["n","a"]]"#), "{}", events(&lexer, "f"));
    assert!(events(&deep, "f").contains(r#"["r",3,["n","a"]]"#), "{}", events(&deep, "f"));
    // The multi-target assignment is mapped by both paths, so the *only*
    // difference here is the value the CST cannot read.
    assert!(events(&lexer, "f").contains(r#"["a",2,["a","b"]"#), "{}", events(&lexer, "f"));
    assert!(events(&deep, "f").contains(r#"["a",2,["a","b"]"#), "{}", events(&deep, "f"));

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "the field is read on both paths now: {verdict:?}");
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
fn a_call_inside_an_f_string_is_no_longer_a_loss() {
    // This case used to be the guard's example of a call only the lexer could
    // see: an f-string field was one opaque token. The Python grammar now parses
    // the field's expression, so the CST reads the call and the two records
    // agree — the fixture stays, inverted, so a regression behind this shape
    // reappears here rather than silently turning the guard back on.
    let source = "def f(x):\n    return f\"{len(x)}\"\n";
    let lexer = lexer_facts(source);
    let deep = deep_facts(source);
    assert!(calls(&lexer).iter().any(|path| path == "len"), "{:?}", calls(&lexer));
    assert!(calls(&deep).iter().any(|path| path == "len"), "the CST reads a field's calls now: {:?}", calls(&deep));

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "nothing is lost here: {verdict:?}");
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

/// Two events on one line is normal: `if (x) g(x);` is a condition and a call.
/// The condition's value is a name the call's value also carries, so a pairing
/// that takes the first *covering* row handed the condition the call's row and
/// left the call with nothing — the file was reported as losing a call the CST
/// had written all along. The identical row must win over a covering one.
#[test]
fn two_events_on_one_line_are_paired_with_their_own_rows() {
    let source = "function f(descriptor) {\n  const parts = [];\n  if (descriptor) parts.push(descriptor);\n}\n";
    let language = langs::by_content("js", source.as_bytes()).expect("a JavaScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::JavaScript);
    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "{verdict:?}");
}

/// A statement that continues onto the next line with a member access is one
/// statement: `String(name)` then a newline then `.replace(…)`. JavaScript ends
/// a statement at a newline only where a statement can start, and one cannot
/// start with `.`. Splitting there gave the continuation its own event, in the
/// module, while the CST (rightly) wrote the whole expression once in the arrow
/// — 160 JS files fell back as "no event on a line both readers can mean".
#[test]
fn a_member_access_that_starts_a_line_continues_the_statement() {
    let source = "const escapeName = (name) => String(name)\n    .replace(/\\r/g, \"%0D\")\n    .replace(/\"/g, \"%22\");\nexport default escapeName;\n";
    let language = langs::by_content("js", source.as_bytes()).expect("a JavaScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::JavaScript);
    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "{verdict:?}");
}

/// A miss is a miss: pairing never fuses two CST rows into one to satisfy a
/// single lexer event. This is the shape that tempts it — the lexer reads the
/// `var` chain's continuation lines as one statement whose value names five
/// calls, while the CST writes each binding on its own row. Adding two rows
/// together would let the event pass and would also hide the next real loss, so
/// the test pins the opposite: the file falls back, and the reason names the row
/// that could not be paired — the wanted line with its value and the one row the
/// CST did write there — instead of just naming the file.
#[test]
fn a_miss_names_the_row_and_two_rows_are_never_fused() {
    let source = "function _default(polygon, point) {\n  var lambda = longitude(point),\n    phi = point[1],\n    sinPhi = (0, _math.sin)(phi),\n    normal = [(0, _math.sin)(lambda), -(0, _math.cos)(lambda), 0],\n    angle = 0,\n    winding = 0;\n  return winding;\n}\n";
    let language = langs::by_content("js", source.as_bytes()).expect("a JavaScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::JavaScript);

    let verdict = pipeline::loses(&lexer, &deep);
    let detail = format!("{verdict:?}");
    assert!(verdict.lost(), "the chain is a real loss: {detail}");
    assert!(detail.contains("wanted line"), "the reason names the wanted row: {detail}");
    assert!(detail.contains("targets ["), "the reason names the wanted targets: {detail}");
    assert_eq!(detail.matches("found line").count(), 1, "exactly one found row, never two fused: {detail}");
}

/// Two CST rows that the record cannot tell apart are a counted tie, not a
/// silent choice. `a.set(x); a.set(x);` on one line is the cheapest real shape:
/// both readers write two events with the same marker, line, targets and value,
/// so the first want matches two identical rows. The pairing still succeeds, and
/// the tie is reported in `accepted.ambiguous_pairs` so a reader knows the choice
/// was arbitrary rather than forced.
#[test]
fn an_ambiguous_pair_is_counted_not_silently_resolved() {
    let source = "function f(a, x) {\n  a.set(x); a.set(x);\n}\n";
    let language = langs::by_content("js", source.as_bytes()).expect("a JavaScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::JavaScript);
    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "{verdict:?}");
    assert!(
        verdict.accepted().ambiguous_pairs >= 1,
        "the tie is counted: {:?}",
        verdict.accepted()
    );
}

/// Duplicate names are paired by line, not by the first match. Two functions
/// called `f` in one file — a real shape in bundled JS, where a redefinition
/// shadows the earlier one — must pair each lexer symbol with the CST symbol on
/// its own line. Pairing `f` with the first symbol named `f` would hand the
/// second body the first body's calls and report a loss that does not exist.
#[test]
fn duplicate_function_names_are_paired_by_their_line() {
    let source = "function f() {\n  g(1);\n}\nfunction f() {\n  h(2);\n}\n";
    let language = langs::by_content("js", source.as_bytes()).expect("a JavaScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::JavaScript);
    let same_name = |facts: &Facts| facts.symbols.iter().filter(|row| row.name == "f").count();
    assert_eq!(same_name(&lexer), 2, "the fixture needs two symbols named f: {lexer:?}");
    assert_eq!(same_name(&deep), 2, "and the CST must write both: {deep:?}");

    // The two symbols are distinguishable only by line, which is why the
    // preference is the near-line one: a name-keyed lookup would have to guess
    // which body it was comparing.
    let lines = |facts: &Facts| {
        facts.symbols.iter().filter(|row| row.name == "f").map(|row| row.line).collect::<Vec<_>>()
    };
    assert_eq!(lines(&lexer), vec![1, 4], "the lexer stamps both bodies");
    assert_eq!(lines(&deep), vec![1, 4], "and so does the CST");

    let verdict = pipeline::loses(&lexer, &deep);
    assert!(!verdict.lost(), "near-line pairing pairs each body with its own: {verdict:?}");
}

/// The oracle is only worth something if it is independent. The comparison core
/// — `loses_in` through `uncovered` — reads two row lists and a taxonomy of
/// accepted artefacts; it must never reach into the CST's grammar, its node
/// kinds, or the extractor. The moment it does, a CST bug can be mirrored by the
/// check that is supposed to catch it, and the guard starts agreeing with itself.
#[test]
fn the_comparison_core_never_consults_the_cst() {
    let source = include_str!("../src/pipeline.rs");
    let start = source.find("pub(crate) fn loses_in(").expect("the comparison entry point");
    let end = source.find("fn is_positional_call(").expect("the end of the comparison core");
    let core = &source[start..end];
    for forbidden in ["deep_integration", "grammar", "deep_facts::", "node(", "kind("] {
        assert!(
            !core.contains(forbidden),
            "the guard consulted {forbidden}; it must see only row lists and the taxonomy"
        );
    }
}

/// The four reason classes and the strings the corpus really produces for them.
/// The prose is for a reader and the code is for a histogram; this pins both ends
/// of that mapping against real output rather than against strings invented for
/// the test, and it fails if a reason is reworded without its bucket following.
#[test]
fn the_corpus_reason_strings_map_to_their_buckets() {
    // Verbatim from a sweep of the JS corpus (000085_polygonContains.js), the
    // rust subset (000005_mode.rs) and the rust subset (000004_scanner.rs).
    let call = "the CST lost a event: event a line 14 targets [\"lambda\"] in _default (paired _default line 13): a call or constant the lexer read is not in the CST's value; wanted line 14 targets [\"lambda\"] value t:[c(0;[n:point])]; found line 14 targets [\"lambda\"] value c(0;[n:point])";
    let line = "the CST lost a event: event x line 60 targets  in new (no event on a line both readers can mean): lexer m:[n:ModeStack]; CST line 60 targets  value m:[n:ModeStack,m:[0,n:MODE_CAP]]";
    let constant = "the CST lost a event: event a line 16 targets [\"v\"] in h (value as the constant 0): lexer m:[n:match,c(0;[])]; CST line 16 targets [\"v\"] value m:[0,0]";

    assert_eq!(pipeline::reason_code(call), "LEX_OVER_EMIT:call");
    assert_eq!(pipeline::reason_code(line), "PAIR_AMBIGUOUS:1");
    assert_eq!(pipeline::reason_code(constant), "CST_MISSING_KIND:constant");

    // The two reasons the corpus has not needed yet are matched by the same
    // phrase the guard writes, so a reworded reason moves them too.
    assert_eq!(
        pipeline::reason_code("the CST lost a event: event a line 3 targets [\"a\"] in f (the statement's target list differs (names or order)): …"),
        "CST_SHAPE_MISMATCH:targets"
    );
    assert_eq!(
        pipeline::reason_code("the CST lost a event: event r line 3 targets  in f (a name the lexer read is not in the CST's value): …"),
        "LEX_OVER_EMIT:name"
    );

    // A loss that is not an event carries its kind in the sentence: every
    // fallback is written "the CST lost a <kind>: <detail>", and reading the
    // first word instead put a lost variable in a bucket called `the`.
    assert_eq!(pipeline::reason_code("the CST lost a var: var repo = Repo"), "CST_MISSING_KIND:var");
    assert_eq!(pipeline::reason_code("the CST lost a symbol: symbol f kind 1 line 9"), "CST_MISSING_KIND:symbol");
    assert_eq!(pipeline::reason_code("the CST lost a import: import os name x alias "), "CST_MISSING_KIND:import");
    assert_eq!(pipeline::reason_code("symbol f kind 1 line 9"), "CST_MISSING_KIND:symbol");
}

/// An `interface` whose heritage clause sits on the next line is a declaration,
/// not an expression: no statement starts with `extends`, so the lexer's event on
/// that line is its own reading of a header it could not see. It is accepted like
/// a union continuation, and this is the shape vm.d.ts writes 267 times across
/// the TypeScript corpus.
#[test]
fn a_heritage_clause_on_its_own_line_is_not_an_expression() {
    let source = "interface A\n    extends B, Pick<C, \"x\">\n{\n    y?: string;\n}\n";
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript);
    let verdict = pipeline::loses_in(&lexer, &deep, source);
    assert!(!verdict.lost(), "a heritage clause is not a statement: {verdict:?}");
}

/// A `.d.ts` property whose type is a function type is a declaration, not an
/// assignment: `initializeImportMeta?: ((meta: M) => void) | undefined;` was
/// refused because the arrow `=>` was read as `=`, which made the lexer's event
/// on the line a fallback.
#[test]
fn an_arrow_type_in_a_property_is_not_an_assignment() {
    let source = "interface M {\n    initializeImportMeta?: ((meta: M) => void) | undefined;\n}\n";
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript);
    let verdict = pipeline::loses_in(&lexer, &deep, source);
    assert!(!verdict.lost(), "a function-typed property is a declaration: {verdict:?}");
}

/// `[Symbol.dispose](): void;` declares a method under a computed name. The
/// bracket is part of the name, so the line is a signature and not a call.
#[test]
fn a_computed_method_name_is_a_declaration() {
    let source = "interface N {\n    [Symbol.dispose](): void;\n}\n";
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript);
    let verdict = pipeline::loses_in(&lexer, &deep, source);
    assert!(!verdict.lost(), "a computed-name signature is a declaration: {verdict:?}");
}

/// The machine strings are a contract: a dashboard counts them, the queue file
/// names them, and `tools/strict_gate.py` gates on them. A rename that is not a
/// migration resets those counts to zero, and a zero looks like a fix. So the
/// set of prefixes is pinned here — a new bucket is a decision, not an accident.
#[test]
fn the_taxonomy_prefixes_are_frozen() {
    const PREFIXES: [&str; 5] = ["LEX_OVER_EMIT:", "CST_MISSING_KIND:", "CST_SHAPE_MISMATCH:", "PAIR_AMBIGUOUS:", "CST_DEGRADED:"];
    let reasons = [
        "event a line 4 targets [\"x\"] in f (a name the lexer read is not in the CST's value): …",
        "event a line 4 targets [\"x\"] in f (a call or constant the lexer read is not in the CST's value): …",
        "event a line 4 targets [\"x\"] in f (the CST wrote the value as the constant 0): …",
        "event a line 4 targets [\"x\"] in f (the statement's target list differs (names or order)): …",
        "event a line 4 targets  in f (no event on a line both readers can mean): …",
        "the CST lost a var: var repo = Repo",
        "the CST lost a call: call x line 3 in f",
        "the CST needed 4 error nodes in 21: more than 1% is recovery noise",
    ];
    for reason in reasons {
        let code = pipeline::reason_code(reason);
        assert!(
            PREFIXES.iter().any(|prefix| code.starts_with(prefix)),
            "{code} is outside the frozen taxonomy for {reason}"
        );
        assert!(!code.ends_with(':'), "{code} names no construct");
        assert!(!code.contains(":the"), "{code} buckets a sentence word");
    }
}

/// `private _sseResponse?;` is a class field with a modifier, no type, and the
/// optional marker between the name and the semicolon. Trimming only the
/// semicolon left a head that ended in `?`, so the line named nothing and the
/// lexer's value became a fallback.
#[test]
fn an_untyped_optional_field_is_a_declaration() {
    let source = "declare class T {\n    private _sseResponse?;\n}\n";
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript);
    let verdict = pipeline::loses_in(&lexer, &deep, source);
    assert!(!verdict.lost(), "an untyped optional field is a declaration: {verdict:?}");
}

/// `import clearImmediate = globalThis.clearImmediate;` is an import alias. The
/// lexer reads it as an assignment, and the CST records the import; the
/// assignment is the lexer's invention, compared through the import table.
#[test]
fn an_import_alias_is_not_an_assignment() {
    let source = "import clearImmediate = globalThis.clearImmediate;\n";
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript);
    let verdict = pipeline::loses_in(&lexer, &deep, source);
    assert!(!verdict.lost(), "an import alias is an import: {verdict:?}");
}

/// A property name may be a string literal: `"DELETE /app/{id}": Operation<…>;`
/// is a signature in an interface, and the path inside the name is not an
/// expression. Three `.d.ts` files write about a thousand of these each, which
/// made them the largest loss bucket in the corpus — 4,673 losses down to 1,036
/// once the line is recognised.
#[test]
fn a_string_literal_property_name_is_a_declaration() {
    let source = "export interface E {\n    \"DELETE /app/{id}\": Operation<\"/a\", \"delete\">;\n}\n";
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript);
    let verdict = pipeline::loses_in(&lexer, &deep, source);
    assert!(!verdict.lost(), "a quoted property name is a declaration: {verdict:?}");
}

/// The guard is a function of two `Facts` records and nothing else: it never
/// sees a `Language`, a `Grammar`, a parse tree or a kind table, so it cannot
/// accidentally grow a preference for one reader. This test drives it the way
/// the type system allows — two hand-built records, no parser in sight — and
/// takes a verdict out. If the guard ever needed the grammar to decide, this
/// would not compile.
#[test]
fn the_guard_decides_from_two_records_and_no_grammar() {
    let lexer = facts::Facts {
        tag: "PY".to_string(),
        lines: 2,
        imports: Vec::new(),
        symbols: vec![facts::Symbol {
            key: "a.py:f".to_string(),
            name: "f".to_string(),
            owner: String::new(),
            kind: facts::KIND_FUNCTION,
            line: 1,
            end_line: 2,
            params: Vec::new(),
            bases: Vec::new(),
            events: String::new(),
            calls: Vec::new(),
        }],
        vars: Vec::new(),
    };
    let deep = lexer.clone();

    let verdict = pipeline::loses_in(&lexer, &deep, "def f():\n    pass\n");
    assert!(!verdict.lost(), "identical records lose nothing: {verdict:?}");

    let mut short = deep.clone();
    short.symbols.clear();
    let verdict = pipeline::loses_in(&lexer, &short, "def f():\n    pass\n");
    assert!(verdict.lost(), "a record with no symbols lost one");
}

/// Two declaration shapes the lexer reads as code and the CST rightly does not:
/// a generic method signature (`g<T extends B>(type: T): other is T;`) and a
/// method whose return type is a function with a type predicate
/// (`f(x: unknown): (y: unknown) => y is string;`). Both are members of an
/// interface, both were fallbacks, and both are the same mistake — a signature
/// is not a call, and an arrow in a return type is not an assignment.
#[test]
fn interface_member_signatures_are_not_calls() {
    let source = "interface A {\n    g<T extends B>(type: T, other?: T | null): other is T;\n    f(x: unknown): (y: unknown) => y is string;\n}\n";
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    let lexer = parser::parse(source.as_bytes(), language);
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    let deep = deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript);
    let verdict = pipeline::loses_in(&lexer, &deep, source);
    assert!(!verdict.lost(), "a member signature is a declaration: {verdict:?}");
}
