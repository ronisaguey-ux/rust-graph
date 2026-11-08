//! The parser gate: the deep scan must parse every corpus in `bench/corpus` at
//! least [`MIN_SPEEDUP`] times faster than Tree-sitter, and keep at least the
//! clean-file count its list states. Timing a debug build proves nothing, so in
//! a debug build the gate is an ignored test. See the README, "Reproducing the parser gate":
//!
//!     sh bench/build-tsdrv.sh
//!     cargo test --release --features parser --test parser_gate -- --nocapture
//!
//! A list is a header and one `sha256 bytes path` line per file:
//!
//! ```text
//! # python-a: what the files are and where they come from
//! # tree-sitter: python        the grammar the driver links (tree_sitter_python)
//! # min-clean: 200             the deep scan must parse this many without an error
//! ```
//!
//! Paths are absolute; `RG_CORPUS_ROOT=DIR` reads each one under DIR instead,
//! for a copy of the corpus. Every file is checked by size and SHA-256 first,
//! so a changed corpus fails rather than measuring something else.
//!
//! Each round runs both parsers once on the whole corpus (each file parsed
//! `REPS` times, the best counting), alternating which goes first; the median of
//! `RG_GATE_ROUNDS` rounds (default 7) is compared. Both run pinned to one CPU
//! with `taskset` when it is installed: the last CPU, or `RG_GATE_CPU` (`none`
//! for no pinning). `RG_GATE_ONLY=python-a,rust-a` runs only those lists.

#![cfg_attr(debug_assertions, allow(dead_code, unused_imports))]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "../src/sha256.rs"]
#[allow(dead_code)]
mod sha256;

const MIN_SPEEDUP: f64 = 2.0;
const REPS: &str = "5";
const DEEP: &str = env!("CARGO_BIN_EXE_rust-graph-deep-scan");
const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// In a debug build the gate is listed as ignored, with the command that runs it.
#[test]
#[cfg(debug_assertions)]
#[ignore = "the parser gate times parsers: cargo test --release --features parser --test parser_gate -- --nocapture"]
fn the_parser_gate_needs_a_release_build() {
    panic!("the parser gate needs a release build");
}

struct Corpus {
    name: String,
    grammar: String,
    min_clean: usize,
    files: Vec<PathBuf>,
}

/// One run of either parser: the summed best time, and files without an error.
struct Run {
    best_ms: f64,
    clean: usize,
    line: String,
}

#[test]
#[cfg(not(debug_assertions))]
fn the_deep_scan_beats_tree_sitter_by_the_margin() {
    let tsdrv = std::env::var_os("RG_TSDRV").map(PathBuf::from).unwrap_or_else(|| Path::new(ROOT).join("target/bench/tsdrv"));
    assert!(tsdrv.is_file(), "{} is missing: build it with `sh bench/build-tsdrv.sh` (needs git, a C compiler and the network once)", tsdrv.display());
    let rounds: usize = std::env::var("RG_GATE_ROUNDS").ok().map(|value| value.parse().expect("RG_GATE_ROUNDS is a number")).unwrap_or(7);
    let pin = cpu_pin();
    let only: Option<Vec<String>> = std::env::var("RG_GATE_ONLY").ok().map(|value| value.split(',').map(str::to_string).collect());

    let corpora = load_corpora(only.as_deref());
    assert!(!corpora.is_empty(), "no corpus lists to run in bench/corpus (RG_GATE_ONLY={only:?})");
    let lists = Path::new(ROOT).join("target/bench/lists");
    fs::create_dir_all(&lists).unwrap();

    println!("parser gate: {} corpora, {rounds} rounds, reps {REPS}, cpu {}", corpora.len(), pin.as_deref().unwrap_or("not pinned"));
    let mut report = String::new();
    let mut failures = Vec::new();
    for corpus in &corpora {
        let list = lists.join(format!("{}.paths", corpus.name));
        let paths: String = corpus.files.iter().map(|path| format!("{}\n", path.display())).collect();
        fs::write(&list, paths).unwrap();
        let (mut deep, mut tree_sitter) = (Vec::new(), Vec::new());
        for round in 0..rounds {
            let deep_run = || run(&pin, DEEP.as_ref(), &["bench", list.to_str().unwrap(), "--reps", REPS]);
            let ts_run = || run(&pin, &tsdrv, &["bench", &corpus.grammar, list.to_str().unwrap(), REPS]);
            if round % 2 == 0 {
                deep.push(deep_run());
                tree_sitter.push(ts_run());
            } else {
                tree_sitter.push(ts_run());
                deep.push(deep_run());
            }
        }
        let deep_ms = median(deep.iter().map(|run| run.best_ms).collect());
        let ts_ms = median(tree_sitter.iter().map(|run| run.best_ms).collect());
        let speedup = ts_ms / deep_ms;
        let (deep_clean, ts_clean) = (deep[0].clean, tree_sitter[0].clean);
        let total = corpus.files.len();
        let bytes: u64 = corpus.files.iter().map(|path| fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)).sum();
        println!("  deep:        {}", deep[0].line);
        println!("  tree-sitter: {}", tree_sitter[0].line);
        let verdict = if speedup < MIN_SPEEDUP {
            failures.push(format!("{}: {speedup:.2}x is under the {MIN_SPEEDUP}x margin", corpus.name));
            "FAIL"
        } else if deep_clean < corpus.min_clean {
            failures.push(format!("{}: {deep_clean}/{total} clean, the list requires {}", corpus.name, corpus.min_clean));
            "FAIL"
        } else {
            "ok"
        };
        let line = format!(
            "{:<14} {:>5} files {:>9} B  deep {:>7.1} ms  tree-sitter {:>7.1} ms  {:>5.2}x  clean deep {deep_clean}/{total} (min {}) tree-sitter {ts_clean}/{total}  {verdict}",
            corpus.name, total, bytes, deep_ms, ts_ms, speedup, corpus.min_clean
        );
        println!("{line}");
        writeln!(report, "{line}").unwrap();
    }
    println!("\nparser gate summary (median of {rounds} rounds, margin {MIN_SPEEDUP}x):\n{report}");
    assert!(failures.is_empty(), "the parser gate FAILED:\n  {}", failures.join("\n  "));
}

fn load_corpora(only: Option<&[String]>) -> Vec<Corpus> {
    let root = std::env::var_os("RG_CORPUS_ROOT").map(PathBuf::from);
    let mut names: Vec<PathBuf> = fs::read_dir(Path::new(ROOT).join("bench/corpus"))
        .expect("bench/corpus")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "list"))
        .collect();
    names.sort();
    let mut corpora = Vec::new();
    let mut problems = Vec::new();
    for path in names {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if only.is_some_and(|only| !only.contains(&name)) {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let (mut grammar, mut min_clean) = (None, None);
        let mut files = Vec::new();
        for line in text.lines() {
            if let Some(header) = line.strip_prefix('#') {
                if let Some((key, value)) = header.split_once(':') {
                    match key.trim() {
                        "tree-sitter" => grammar = Some(value.trim().to_string()),
                        "min-clean" => min_clean = Some(value.trim().parse::<usize>().expect("min-clean is a number")),
                        _ => {}
                    }
                }
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            let mut fields = line.splitn(3, ' ');
            let (sum, size, file) = (fields.next().unwrap(), fields.next().unwrap_or(""), fields.next().unwrap_or(""));
            let file = match &root {
                Some(root) => root.join(file.trim_start_matches('/')),
                None => PathBuf::from(file),
            };
            match fs::read(&file) {
                Err(error) => problems.push(format!("{name}: {}: {error}", file.display())),
                Ok(bytes) if bytes.len().to_string() != size => problems.push(format!("{name}: {}: {} bytes, the list says {size}", file.display(), bytes.len())),
                Ok(bytes) if hex(&sha256::digest(&bytes)) != sum => problems.push(format!("{name}: {}: SHA-256 differs from the list", file.display())),
                Ok(_) => files.push(file),
            }
        }
        let grammar = grammar.unwrap_or_else(|| panic!("{}: no `# tree-sitter:` header", path.display()));
        let min_clean = min_clean.unwrap_or_else(|| panic!("{}: no `# min-clean:` header", path.display()));
        corpora.push(Corpus { name, grammar, min_clean, files });
    }
    if !problems.is_empty() {
        let shown: Vec<&String> = problems.iter().take(12).collect();
        panic!(
            "{} corpus files are missing or differ from their lists, so the gate would measure something else:\n  {}\n(set RG_CORPUS_ROOT to read a copy of the corpus)",
            problems.len(),
            shown.iter().map(|problem| problem.as_str()).collect::<Vec<_>>().join("\n  ")
        );
    }
    corpora
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `taskset -c CPU` if pinning is possible and wanted.
fn cpu_pin() -> Option<String> {
    let wanted = std::env::var("RG_GATE_CPU").ok();
    if wanted.as_deref() == Some("none") {
        return None;
    }
    let installed = Command::new("taskset").arg("--version").output().is_ok_and(|output| output.status.success());
    if !installed {
        assert!(wanted.is_none(), "RG_GATE_CPU is set but taskset is not installed");
        return None;
    }
    Some(wanted.unwrap_or_else(|| (std::thread::available_parallelism().map_or(1, |count| count.get()) - 1).to_string()))
}

fn run(pin: &Option<String>, program: &Path, args: &[&str]) -> Run {
    let mut command = match pin {
        Some(cpu) => {
            let mut command = Command::new("taskset");
            command.args(["-c", cpu]).arg(program);
            command
        }
        None => Command::new(program),
    };
    let output = command.args(args).output().unwrap_or_else(|error| panic!("{}: {error}", program.display()));
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(output.status.success(), "{} {args:?} failed: {}{}", program.display(), stdout, String::from_utf8_lossy(&output.stderr));
    let field = |key: &str| {
        stdout
            .split_whitespace()
            .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("no {key}= in {stdout:?}"))
            .to_string()
    };
    if let Some(incomplete) = stdout.split_whitespace().find_map(|pair| pair.strip_prefix("incomplete=")) {
        assert_eq!(incomplete, "0", "a parse ran out of budget: {stdout}");
    }
    let clean = field("clean_files");
    Run {
        best_ms: field("best_ms").parse().expect("best_ms is a number"),
        clean: clean.split('/').next().unwrap().parse().expect("clean_files is a count"),
        line: stdout,
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
