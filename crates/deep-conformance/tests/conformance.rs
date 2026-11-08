//! The gates have to be able to fail, and `not_measured` has to stay out of
//! every clean rate. These tests build a synthetic corpus with known-good and
//! deliberately broken files, run the real scan and the real gate functions,
//! and assert on the numbers they print.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use deep_conformance::gates::{diff_text, gate_results, gates_text};
use deep_conformance::registry::{ParseFacts, Registry};
use deep_conformance::report::{FileRecord, Outcome, Report};
use deep_conformance::scan::scan;
use deep_conformance::summary::{per_language, summary_text};

struct Corpus {
    root: PathBuf,
}

impl Corpus {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!("deep-conformance-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for (relative, body) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, body).unwrap();
        }
        Corpus { root }
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn good(source: &[u8]) -> ParseFacts {
    ParseFacts::clean((source.len() / 8).max(1) as u64, 3)
}

fn broken(source: &[u8]) -> ParseFacts {
    ParseFacts::broken((source.len() / 8).max(1) as u64, 3, 1, source.len() as u64)
}

fn fatal(_: &[u8]) -> ParseFacts {
    ParseFacts::fatal("synthetic: no tree")
}

fn panics(_: &[u8]) -> ParseFacts {
    panic!("synthetic parser panic")
}

fn slow(source: &[u8]) -> ParseFacts {
    std::thread::sleep(Duration::from_millis(5));
    ParseFacts::clean((source.len() / 8).max(1) as u64, 3)
}

fn registry_with(entries: &[(&str, deep_conformance::ParseFn)]) -> Registry {
    let mut registry = Registry::new();
    for (language, parser) in entries {
        assert!(registry.set_parser(language, *parser), "unknown language {language}");
    }
    registry
}

/// A report assembled by hand, for the gates that depend on a process-level
/// fact (RSS, cold start) no synthetic parser can force without allocating
/// gigabytes or sleeping a thousand milliseconds.
fn hand_report(files: usize, bytes: u64, ms: Option<f64>, rss: Option<u64>, cold_us: Option<f64>) -> Report {
    let records = (0..files)
        .map(|index| FileRecord {
            path: format!("f{index}.py"),
            language: "Python".to_string(),
            bytes,
            outcome: Outcome::Clean,
            error_nodes: 0,
            error_bytes: 0,
            nodes: 100,
            max_depth: 4,
            ms,
            rss_bytes: rss,
            reason: None,
        })
        .collect();
    Report { root: "/synthetic".to_string(), cold_start_us: cold_us, scan_ms: 1.0, skipped: 0, walk_errors: 0, files: records }
}

fn gate(report: &Report, needle: &str) -> deep_conformance::GateResult {
    gate_results(report).into_iter().find(|gate| gate.name.contains(needle)).expect("gate exists")
}

#[test]
fn scan_measures_every_file_and_never_aborts_on_a_bad_one() {
    let corpus = Corpus::new(
        "scan",
        &[
            ("a.py", "x = 1\n"),
            ("b.py", "def broken(:\n"),
            ("c.py", "fatal\n"),
            ("d.py", "boom\n"),
            ("sub/e.py", "y = 2\n"),
            ("notes.txt", "not a language"),
            ("image.png", "not a language either"),
        ],
    );
    // A file with no read permission: fs::read fails, the scan records it and
    // keeps going.
    let locked = corpus.root().join("unreadable.py");
    fs::write(&locked, "x = 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    }

    let registry = registry_with(&[("Python", good)]);
    let report = scan(corpus.root(), &registry).unwrap();
    let python: Vec<&FileRecord> = report.files.iter().filter(|file| file.language == "Python").collect();
    assert_eq!(python.len(), 6, "five files plus the unreadable one: {python:#?}");
    assert!(python.iter().any(|file| file.outcome == Outcome::Unreadable));
    assert!(python.iter().filter(|file| file.outcome == Outcome::Clean).count() >= 4);
    assert_eq!(report.skipped, 2, "txt and png are not project languages");
    assert!(python.iter().all(|file| file.ms.is_some() || file.outcome == Outcome::Unreadable));

    // One panicking parser must not end the run: the file is fatal, the rest is
    // still measured.
    let panic_corpus = Corpus::new("panic", &[("a.py", "x\n"), ("b.py", "y\n")]);
    let registry = registry_with(&[("Python", panics)]);
    let report = scan(panic_corpus.root(), &registry).unwrap();
    assert_eq!(report.files.len(), 2);
    assert!(report.files.iter().all(|file| file.outcome == Outcome::Fatal));
    assert!(report.files.iter().all(|file| file.reason.as_deref() == Some("parser panicked")));
}

#[test]
fn summary_prints_the_denominator_and_fails_under_95_percent() {
    let mut files: Vec<(String, String)> = (0..9).map(|index| (format!("ok{index}.py"), "a = 1\n".to_string())).collect();
    files.push(("bad.py".to_string(), "a = (\n".to_string()));
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(name, body)| (name.as_str(), body.as_str())).collect();
    let corpus = Corpus::new("rate-90", &borrowed);

    let mut registry = Registry::new();
    registry.set_parser("Python", good);
    let report = scan(corpus.root(), &registry).unwrap();
    // Force the known mix by editing the records: the scan path is tested above;
    // this test is about the arithmetic and the verdict.
    let mut report = report;
    report.files.sort_by(|a, b| a.path.cmp(&b.path));
    report.files[0].outcome = Outcome::Errors;
    report.files[0].error_nodes = 1;
    report.files[0].error_bytes = 6;

    let (text, ok) = summary_text(&report);
    assert!(!ok, "{text}");
    assert!(text.contains("90.0% (9/10)"), "{text}");
    assert!(text.contains("FAIL  Python: 90.0% clean (9/10) is under 95%"), "{text}");
}

#[test]
fn summary_passes_at_exactly_95_percent() {
    let corpus = Corpus::new("rate-95", &[("a.py", "x = 1\n")]);
    let mut registry = Registry::new();
    registry.set_parser("Python", good);
    let report = scan(corpus.root(), &registry).unwrap();
    let mut report = report;
    let template = report.files[0].clone();
    report.files.clear();
    for index in 0..20 {
        let mut file = template.clone();
        file.path = format!("f{index}.py");
        if index == 0 {
            file.outcome = Outcome::Errors;
            file.error_nodes = 1;
            file.error_bytes = 1;
        }
        report.files.push(file);
    }
    let (text, ok) = summary_text(&report);
    assert!(ok, "{text}");
    assert!(text.contains("95.0% (19/20)"), "{text}");
}

#[test]
fn scan_records_clean_errors_and_fatal_outcomes_separately() {
    let corpus = Corpus::new(
        "outcomes",
        &[("a.py", "x\n"), ("b.go", "package main\n"), ("c.rs", "fn main() {}\n")],
    );
    let registry = registry_with(&[("Python", good), ("Go", broken), ("Rust", fatal)]);
    let report = scan(corpus.root(), &registry).unwrap();
    let find = |suffix: &str| report.files.iter().find(|file| file.path.ends_with(suffix)).unwrap();
    assert_eq!(find("a.py").outcome, Outcome::Clean);
    assert_eq!(find("b.go").outcome, Outcome::Errors);
    assert_eq!(find("b.go").error_bytes, 13);
    assert_eq!(find("c.rs").outcome, Outcome::Fatal);
    assert_eq!(find("c.rs").reason.as_deref(), Some("synthetic: no tree"));
}

#[test]
fn clean_rate_gate_fails_and_says_which_language_decided_it() {
    let report = hand_report(10, 1000, Some(0.05), Some(1 << 20), Some(100.0));
    let mut report = report;
    report.files[0].outcome = Outcome::Errors;
    let result = gate(&report, "clean rate");
    assert!(!result.pass, "{result:?}");
    assert!(result.detail.contains("Python"), "{}", result.detail);
    assert!(result.detail.contains("90.0% clean (9/10)"), "{}", result.detail);
    let (_, all) = gates_text(&report);
    assert!(!all);
}

#[test]
fn speed_gate_fails_on_a_slow_parser() {
    let files: Vec<(&str, &str)> = (0..10).map(|_| ("x.py", "0123456789abcdef")).collect();
    let corpus = Corpus::new("slow", &files);
    let registry = registry_with(&[("Python", slow)]);
    let report = scan(corpus.root(), &registry).unwrap();
    let result = gate(&report, "median parse");
    assert!(!result.pass, "{result:?}");
    // 16 bytes in 5 ms is ~0.003 MB/s aggregate, far under the 15 MB/s floor.
    assert!(result.detail.contains("aggregate"), "{}", result.detail);
    assert!(result.detail.contains("per-file median"), "{}", result.detail);
}

#[test]
fn all_four_gates_pass_on_a_measured_report() {
    let report = hand_report(12, 1000, Some(0.01), Some(100 << 20), Some(400.0));
    let results = gate_results(&report);
    assert_eq!(results.len(), 4);
    for result in &results {
        assert!(result.pass, "{result:?}");
    }
    let (text, ok) = gates_text(&report);
    assert!(ok, "{text}");
    assert_eq!(text.matches("PASS").count(), 5, "four gates plus the result line: {text}");
}

#[test]
fn rss_gate_fails_over_two_gib() {
    let report = hand_report(12, 1000, Some(0.01), Some(3 * 1024 * 1024 * 1024), Some(400.0));
    let result = gate(&report, "RSS");
    assert!(!result.pass, "{result:?}");
    assert!(result.detail.contains("3072.0 MB"), "{}", result.detail);
    let (_, all) = gates_text(&report);
    assert!(!all);
}

#[test]
fn cold_start_gate_fails_slow_and_fails_unmeasured() {
    let slow = hand_report(12, 1000, Some(0.01), Some(1 << 20), Some(2500.0));
    let result = gate(&slow, "cold start");
    assert!(!result.pass, "{result:?}");
    assert!(result.detail.contains("2.500 ms"), "{}", result.detail);

    let unmeasured = hand_report(12, 1000, Some(0.01), Some(1 << 20), None);
    let result = gate(&unmeasured, "cold start");
    assert!(!result.pass, "{result:?}");
    assert!(result.detail.contains("not measured"), "{}", result.detail);
}

#[test]
fn not_measured_is_never_clean_and_fails_the_clean_gate() {
    let files: Vec<(String, String)> = (0..12).map(|index| (format!("f{index}.go"), "package main\n".to_string())).collect();
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(name, body)| (name.as_str(), body.as_str())).collect();
    let corpus = Corpus::new("no-parser", &borrowed);
    let registry = Registry::new();
    let report = scan(corpus.root(), &registry).unwrap();

    assert_eq!(report.files.len(), 12);
    assert!(report.files.iter().all(|file| file.outcome == Outcome::NotMeasured));
    assert!(report.files.iter().all(|file| file.ms.is_none()));
    let stats = per_language(&report);
    assert_eq!(stats[0].clean, 0);
    assert_eq!(stats[0].measured, 0);
    assert_eq!(stats[0].clean_rate(), None);

    let (text, ok) = summary_text(&report);
    assert!(!ok, "{text}");
    assert!(text.contains("not measured"), "{text}");
    assert!(!text.contains("100.0%"), "{text}");

    let result = gate(&report, "clean rate");
    assert!(!result.pass, "{result:?}");
    assert!(result.detail.contains("Go: not measured"), "{}", result.detail);
}

#[test]
fn diff_flags_a_clean_rate_regression() {
    let mut before = hand_report(20, 1000, Some(0.10), Some(1 << 20), Some(400.0));
    let mut after = before.clone();
    for file in &before.files {
        assert_eq!(file.outcome, Outcome::Clean);
    }
    after.files[0].outcome = Outcome::Errors;
    after.files[1].outcome = Outcome::Errors;
    let _ = &mut before;

    let (text, ok) = diff_text(&before, &after);
    assert!(!ok, "{text}");
    assert!(text.contains("REGRESSION: clean -10.0pp"), "{text}");
    assert!(text.contains("90.0% (18/20)"), "{text}");
}

#[test]
fn diff_flags_a_speed_regression() {
    let before = hand_report(12, 1000, Some(0.10), Some(1 << 20), Some(400.0));
    let after = hand_report(12, 1000, Some(0.50), Some(1 << 20), Some(400.0));
    let (text, ok) = diff_text(&before, &after);
    assert!(!ok, "{text}");
    assert!(text.contains("REGRESSION: p50 5.00x slower"), "{text}");
}

#[test]
fn diff_of_a_report_against_itself_passes() {
    let report = hand_report(12, 1000, Some(0.10), Some(1 << 20), Some(400.0));
    let (text, ok) = diff_text(&report, &report);
    assert!(ok, "{text}");
    assert!(text.contains("result: PASS (0 regressions)"), "{text}");
}

#[test]
fn a_language_that_disappears_from_the_corpus_is_a_regression() {
    let before = hand_report(12, 1000, Some(0.1), Some(1 << 20), Some(400.0));
    let after = Report { files: Vec::new(), ..before.clone() };
    let (text, ok) = diff_text(&before, &after);
    assert!(!ok, "{text}");
    assert!(text.contains("REGRESSION: language missing from b"), "{text}");
}
