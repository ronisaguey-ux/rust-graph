//! Measure the deep parser against real code: one command in, a report and a
//! verdict out.
//!
//! ```text
//! deep-conformance scan    --root DIR --out report.json
//! deep-conformance summary --report report.json
//! deep-conformance gates   --report report.json
//! deep-conformance diff    --a report.json --b report.json
//! deep-conformance languages
//! ```
//!
//! `scan` walks a tree, parses every file whose extension names a project
//! language, and writes one record per file: bytes, outcome, error nodes,
//! error bytes, node count, depth, milliseconds and peak RSS. A file with no
//! registered parser is `not_measured`; a file that cannot be read is
//! `unreadable`; a parser that panics is `fatal`. None of them stops the run.
//!
//! `summary` prints a per-language table and fails when a language with at
//! least ten files is under 95% clean. `gates` states all four project gates
//! with the number that decided each one. `diff` compares two reports and
//! names per-language clean-rate and speed regressions.
//!
//! The registry seam is [`registry::Registry`]: the integrator calls
//! `set_parser("Rust", parse_rust)` once the grammar crate lands. Until then
//! the answer is "not measured", never "clean".

use std::path::Path;
use std::time::Instant;

pub mod gates;
pub mod json;
pub mod registry;
pub mod report;
pub mod scan;
pub mod summary;

#[cfg(feature = "deep-grammars")]
mod grammars;

pub use gates::GateResult;
pub use registry::{LanguageSpec, ParseFacts, ParseFn, Registry};
pub use report::{FileRecord, Outcome, Report};
pub use scan::{peak_rss_bytes, process_age_us, scan, scan_from};

/// Everything ran and every gate or threshold held.
pub const EXIT_OK: i32 = 0;
/// A gate, a clean-rate floor or a diff regression failed.
pub const EXIT_GATE: i32 = 1;
/// Usage error, unreadable report, or an unwritable output path.
pub const EXIT_REFUSED: i32 = 2;

const USAGE: &str = "usage: deep-conformance scan --root DIR --out report.json
       deep-conformance summary --report report.json
       deep-conformance gates --report report.json
       deep-conformance diff --a report.json --b report.json
       deep-conformance languages

exit: 0 pass, 1 gate/regression failure, 2 usage or unreadable input";

fn flag(args: &[String], name: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == name)?;
    args.get(at + 1).cloned()
}

/// Run the command line. `command_start` is captured by [`cli`] before argument
/// parsing, so the cold-start gate covers the whole path from process entry to
/// the scan being ready.
pub fn run(args: &[String], command_start: Instant) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("{USAGE}");
        return EXIT_REFUSED;
    };
    match command {
        "scan" => {
            let (Some(root), Some(out)) = (flag(args, "--root"), flag(args, "--out")) else {
                eprintln!("scan needs --root DIR and --out FILE\n{USAGE}");
                return EXIT_REFUSED;
            };
            let registry = registry::builtin();
            let report = match scan::scan_from(Path::new(&root), &registry, command_start) {
                Ok(report) => report,
                Err(error) => {
                    eprintln!("scan {root}: {error}");
                    return EXIT_REFUSED;
                }
            };
            if let Err(error) = report.save(Path::new(&out)) {
                eprintln!("write {out}: {error}");
                return EXIT_REFUSED;
            }
            let measured = report.files.iter().filter(|file| file.outcome.measured()).count();
            let not_measured = report.files.iter().filter(|file| file.outcome == Outcome::NotMeasured).count();
            println!(
                "scanned {} files ({} measured, {} not measured, {} skipped) in {:.1} ms -> {out}",
                report.files.len(),
                measured,
                not_measured,
                report.skipped,
                report.scan_ms
            );
            if registry.registered_count() == 0 {
                println!("note: no parser is registered in this build; every file is not_measured");
            }
            EXIT_OK
        }
        "summary" => {
            let Some(path) = flag(args, "--report") else {
                eprintln!("summary needs --report FILE\n{USAGE}");
                return EXIT_REFUSED;
            };
            match Report::load(Path::new(&path)) {
                Ok(report) => {
                    let (text, ok) = summary::summary_text(&report);
                    print!("{text}");
                    if ok {
                        EXIT_OK
                    } else {
                        EXIT_GATE
                    }
                }
                Err(error) => {
                    eprintln!("read {path}: {error}");
                    EXIT_REFUSED
                }
            }
        }
        "gates" => {
            let Some(path) = flag(args, "--report") else {
                eprintln!("gates needs --report FILE\n{USAGE}");
                return EXIT_REFUSED;
            };
            match Report::load(Path::new(&path)) {
                Ok(report) => {
                    let (text, ok) = gates::gates_text(&report);
                    print!("{text}");
                    if ok {
                        EXIT_OK
                    } else {
                        EXIT_GATE
                    }
                }
                Err(error) => {
                    eprintln!("read {path}: {error}");
                    EXIT_REFUSED
                }
            }
        }
        "diff" => {
            let (Some(left), Some(right)) = (flag(args, "--a"), flag(args, "--b")) else {
                eprintln!("diff needs --a FILE and --b FILE\n{USAGE}");
                return EXIT_REFUSED;
            };
            let load = |path: &str| match Report::load(Path::new(path)) {
                Ok(report) => Ok(report),
                Err(error) => {
                    eprintln!("read {path}: {error}");
                    Err(())
                }
            };
            let a = match load(&left) {
                Ok(report) => report,
                Err(()) => return EXIT_REFUSED,
            };
            let b = match load(&right) {
                Ok(report) => report,
                Err(()) => return EXIT_REFUSED,
            };
            let (text, ok) = gates::diff_text(&a, &b);
            print!("{text}");
            if ok {
                EXIT_OK
            } else {
                EXIT_GATE
            }
        }
        "languages" => {
            let registry = registry::builtin();
            println!("{:<20}  {:<34}  {}", "language", "extensions", "parser");
            for spec in registry.languages() {
                println!(
                    "{:<20}  {:<34}  {}",
                    spec.name,
                    spec.extensions_display(),
                    if spec.registered() { "registered" } else { "not measured" }
                );
            }
            println!(
                "\n{} of {} languages have a parser in this build",
                registry.registered_count(),
                registry.languages().len()
            );
            EXIT_OK
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            EXIT_OK
        }
        other => {
            eprintln!("unknown command {other:?}\n{USAGE}");
            EXIT_REFUSED
        }
    }
}

/// Parse `std::env::args` and return the process exit code.
pub fn cli() -> i32 {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    run(&args, started)
}
