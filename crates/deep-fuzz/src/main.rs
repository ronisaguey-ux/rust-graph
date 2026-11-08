//! The `deep-fuzz` command line.
//!
//! ```text
//! deep-fuzz fuzz    --lang <name|all> --seconds N [--seed S] [--fixtures DIR] [--crash-dir DIR]
//! deep-fuzz shrink  --input FILE [--lang NAME] [--out FILE] [--seconds N]
//! deep-fuzz corpus  --dir DIR [--lang NAME]
//! deep-fuzz languages
//! ```
//!
//! Exit status: `0` nothing failed, `1` a panic, hang, memory breach or
//! unreachable tree, `2` a usage error or an unreadable input. Nothing else is
//! written to standard output except the report, one line per language or file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use deep_fuzz::corpus::files_in;
use deep_fuzz::harness::{check, quiet_panics, Failure, LanguageTarget, MEMORY_LIMIT};
use deep_fuzz::mutate::{mutate, Rng};
use deep_fuzz::registry::{self, Entry, ENTRIES};
use deep_fuzz::shrink::{escape, shrink, DEFAULT_SHRINK_TIME};

const USAGE: &str = "\
usage:
  deep-fuzz fuzz   --lang <name|all> --seconds N [--seed S] [--fixtures DIR] [--crash-dir DIR]
  deep-fuzz shrink --input FILE [--lang NAME] [--out FILE] [--seconds N]
  deep-fuzz corpus --dir DIR [--lang NAME]
  deep-fuzz languages

fuzz     mutate seed fixtures for N seconds per language and check every input
shrink   reduce a failing input to the smallest bytes that still fail
corpus   check every file in a directory, without mutation
languages  list fuzzable language names
";

fn main() -> ExitCode {
    quiet_panics();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = arguments.split_first() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let options = match Options::parse(rest) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("deep-fuzz: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match command.as_str() {
        "fuzz" => fuzz(&options),
        "shrink" => shrink_command(&options),
        "corpus" => corpus_command(&options),
        "languages" => {
            for entry in ENTRIES {
                println!("{}\t{}", entry.name, entry.ext);
            }
            ExitCode::SUCCESS
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("deep-fuzz: unknown command {other:?}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[derive(Default)]
struct Options {
    lang: Option<String>,
    input: Option<PathBuf>,
    dir: Option<PathBuf>,
    out: Option<PathBuf>,
    fixtures: Option<PathBuf>,
    crash_dir: Option<PathBuf>,
    seconds: Option<u64>,
    seed: Option<u64>,
}

impl Options {
    fn parse(arguments: &[String]) -> Result<Options, String> {
        let mut options = Options::default();
        let mut at = 0;
        while at < arguments.len() {
            let flag = arguments[at].as_str();
            let mut value = || -> Result<String, String> {
                at += 1;
                arguments.get(at).cloned().ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag {
                "--lang" | "-l" => options.lang = Some(value()?),
                "--input" | "-i" => options.input = Some(PathBuf::from(value()?)),
                "--dir" | "-d" => options.dir = Some(PathBuf::from(value()?)),
                "--out" | "-o" => options.out = Some(PathBuf::from(value()?)),
                "--fixtures" => options.fixtures = Some(PathBuf::from(value()?)),
                "--crash-dir" => options.crash_dir = Some(PathBuf::from(value()?)),
                "--seconds" | "-s" => {
                    let raw = value()?;
                    options.seconds = Some(raw.parse().map_err(|_| format!("--seconds {raw:?} is not a number"))?);
                }
                "--seed" => {
                    let raw = value()?;
                    options.seed = Some(raw.parse().map_err(|_| format!("--seed {raw:?} is not a number"))?);
                }
                other => return Err(format!("unknown flag {other:?}")),
            }
            at += 1;
        }
        Ok(options)
    }
}

fn fuzz(options: &Options) -> ExitCode {
    let Some(lang) = options.lang.as_deref() else {
        eprintln!("deep-fuzz fuzz: --lang is required (a name, or `all`)");
        return ExitCode::from(2);
    };
    let seconds = options.seconds.unwrap_or(30);
    if seconds == 0 {
        eprintln!("deep-fuzz fuzz: --seconds must be at least 1");
        return ExitCode::from(2);
    }
    let seed = options.seed.unwrap_or(1);
    let selected: Vec<&'static Entry> = if lang.eq_ignore_ascii_case("all") {
        ENTRIES.iter().collect()
    } else {
        match registry::by_name(lang) {
            Some(entry) => vec![entry],
            None => {
                eprintln!("deep-fuzz fuzz: unknown language {lang:?}; try `deep-fuzz languages`");
                return ExitCode::from(2);
            }
        }
    };
    let fixtures = options.fixtures.clone().unwrap_or_else(registry::fixtures_root);
    let crash_dir = options.crash_dir.clone().unwrap_or_else(|| PathBuf::from("crates/deep-fuzz/crashes"));
    if cfg!(panic = "abort") {
        eprintln!("deep-fuzz: this build aborts on panic (panic = \"abort\"); a parser panic ends the run after the input is saved");
    }
    println!(
        "lang\tinputs\tpanics\thangs\tunreachable\tmemory\tworst_ms\tnodes_max\tpeak_mb\tseed\tseconds"
    );
    let mut failed = false;
    for entry in selected {
        let seeds = match registry::seeds_in(&fixtures, entry) {
            Ok(seeds) => seeds,
            Err(message) => {
                eprintln!("deep-fuzz: {message}");
                return ExitCode::from(2);
            }
        };
        let mut target = LanguageTarget::new(entry);
        let mut rng = Rng::new(mix(seed, entry.name));
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut inputs = 0u64;
        let mut panics = 0u64;
        let mut hangs = 0u64;
        let mut unreachable = 0u64;
        let mut memory = 0u64;
        let mut worst = Duration::ZERO;
        let mut nodes_max = 0u32;
        let mut saved = 0u32;
        let mut iteration = 0usize;
        while Instant::now() < deadline {
            let seed_bytes = &seeds[iteration % seeds.len()].bytes;
            let input = mutate(&mut rng, seed_bytes);
            let report = check(&mut target, &input);
            iteration += 1;
            inputs += 1;
            worst = worst.max(report.elapsed);
            nodes_max = nodes_max.max(report.nodes);
            if let Some(failure) = report.failure {
                match failure {
                    Failure::Panic { .. } => panics += 1,
                    Failure::Hang { .. } => hangs += 1,
                    Failure::Unreachable { .. } => unreachable += 1,
                    Failure::Memory { .. } => memory += 1,
                }
                failed = true;
                if saved < 32 {
                    saved += 1;
                    if let Err(error) = save_crash(&crash_dir, entry.name, seed, iteration, &input) {
                        eprintln!("deep-fuzz: cannot save crash: {error}");
                    }
                }
                if matches!(failure, Failure::Panic { .. }) {
                    // A panic can leave the workspace half-written; start clean.
                    target = LanguageTarget::new(entry);
                }
                if saved <= 5 {
                    eprintln!("FAIL {}: {} ({} bytes, iteration {iteration})", entry.name, failure, input.len());
                }
            }
        }
        let peak = deep_fuzz::harness::peak_rss_bytes().unwrap_or(0);
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.1}\t{}\t{}",
            entry.name,
            inputs,
            panics,
            hangs,
            unreachable,
            memory,
            worst.as_millis(),
            nodes_max,
            peak as f64 / (1024.0 * 1024.0),
            seed,
            seconds
        );
        if memory > 0 {
            eprintln!(
                "FAIL {}: peak RSS {} bytes over the {} byte cap",
                entry.name,
                peak,
                MEMORY_LIMIT
            );
        }
    }
    if failed {
        eprintln!("deep-fuzz: at least one input failed");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn shrink_command(options: &Options) -> ExitCode {
    let Some(input_path) = options.input.as_deref() else {
        eprintln!("deep-fuzz shrink: --input is required");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(input_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("deep-fuzz shrink: {input_path:?}: {error}");
            return ExitCode::from(2);
        }
    };
    let entry = match select_entry(options.lang.as_deref(), input_path) {
        Ok(entry) => entry,
        Err(message) => {
            eprintln!("deep-fuzz shrink: {message}");
            return ExitCode::from(2);
        }
    };
    let budget = Duration::from_secs(options.seconds.unwrap_or(DEFAULT_SHRINK_TIME.as_secs()));
    let Some(result) = shrink(|| LanguageTarget::new(entry), &bytes, budget) else {
        eprintln!("deep-fuzz shrink: {} does not fail on this parser, nothing to shrink", input_path.display());
        return ExitCode::from(2);
    };
    println!("language\t{}", entry.name);
    println!("original\t{} bytes", result.original_len);
    println!("minimal\t{} bytes", result.bytes.len());
    println!("attempts\t{}", result.attempts);
    println!("complete\t{}", !result.capped);
    if let Some(failure) = &result.failure {
        println!("failure\t{failure}");
    }
    println!("escaped\t{}", escape(&result.bytes));
    let mut hex = String::new();
    for byte in &result.bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    println!("hex\t{hex}");
    if let Some(out) = options.out.as_deref() {
        if let Err(error) = std::fs::write(out, &result.bytes) {
            eprintln!("deep-fuzz shrink: cannot write {out:?}: {error}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

fn corpus_command(options: &Options) -> ExitCode {
    let Some(dir) = options.dir.as_deref() else {
        eprintln!("deep-fuzz corpus: --dir is required");
        return ExitCode::from(2);
    };
    let files = match files_in(dir) {
        Ok(files) => files,
        Err(message) => {
            eprintln!("deep-fuzz corpus: {message}");
            return ExitCode::from(2);
        }
    };
    let mut targets: BTreeMap<&'static str, LanguageTarget> = BTreeMap::new();
    let mut failed = false;
    let mut checked = 0u64;
    let mut skipped = 0u64;
    let mut worst = Duration::ZERO;
    for path in &files {
        let entry = match select_entry(options.lang.as_deref(), path) {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("FAIL {}: {error}", path.display());
                failed = true;
                continue;
            }
        };
        let target = targets.entry(entry.name).or_insert_with(|| LanguageTarget::new(entry));
        let report = check(target, &bytes);
        checked += 1;
        worst = worst.max(report.elapsed);
        if let Some(failure) = report.failure {
            failed = true;
            eprintln!("FAIL {}: {failure}", path.display());
        }
    }
    println!("checked\t{checked}");
    println!("skipped\t{skipped}");
    println!("worst_ms\t{}", worst.as_millis());
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn select_entry(forced: Option<&str>, path: &Path) -> Result<&'static Entry, String> {
    if let Some(name) = forced {
        return registry::by_name(name).ok_or_else(|| format!("unknown language {name:?}"));
    }
    deep_fuzz::corpus::language_for_path(path)
        .ok_or_else(|| format!("{}: no language for this extension; pass --lang", path.display()))
}

fn save_crash(dir: &Path, language: &str, seed: u64, iteration: usize, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let name = format!("{language}-{seed}-{iteration}.bin");
    std::fs::write(dir.join(name), bytes)
}

/// Spread the user seed across languages so each one gets a different stream
/// while one `--seed` still reproduces the whole run.
fn mix(seed: u64, name: &str) -> u64 {
    let mut state = seed ^ 0xCBF2_9CE4_8422_2325;
    for &byte in name.as_bytes() {
        state ^= byte as u64;
        state = state.wrapping_mul(0x100_0000_01B3);
    }
    state
}
