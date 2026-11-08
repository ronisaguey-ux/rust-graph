//! The harness behind the deep parser's 2x gate.
//!
//! Three commands, all of which print the machine, the core count and the exact
//! command line first, so any number below can be reproduced or disbelieved:
//!
//! ```text
//! rust-graph-deep-bench corpus --list FILE
//!     file count, total bytes, SHA-256 of the manifest
//! rust-graph-deep-bench compare --tree-sitter PATH --deep PATH --list FILE --rounds N
//!     median/mean/min/max per parser, run alternately; exits 1 under the margin
//! rust-graph-deep-bench memory --deep PATH --list FILE
//!     peak RSS of the deep parser, in MB, from VmHWM
//! ```
//!
//! A corpus manifest is plain text, one absolute path per line; blank lines and
//! lines starting with `#` are ignored. Every path is checked for existence and
//! summed before anything is timed: a manifest with a missing file is refused
//! ([`EXIT_REFUSED`]), never reported on.
//!
//! What gets timed is a whole process, so the harness never needs to know how a
//! parser works. A parser binary must accept
//!
//! ```text
//! <binary> --list FILE [--reps N]
//! ```
//!
//! and parse every path in `FILE` before exiting 0. `crates/deep-bench/adapters`
//! wraps the two CLIs already in this repository (`rust-graph-deep-scan bench`
//! and `target/bench/tsdrv bench`) in that shape.
//!
//! std only: the SHA-256 is the repository's own `src/sha256.rs`, included by
//! path rather than reimplemented.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

mod bench;
mod manifest;
#[cfg(test)]
mod tests;

#[path = "../../../src/sha256.rs"]
mod sha256;

/// The margin `compare` enforces: tree-sitter's time divided by the deep
/// parser's time.
pub const MIN_SPEEDUP: f64 = 2.0;
/// The manifest or a binary is wrong, so there is nothing to report.
pub const EXIT_REFUSED: i32 = 2;
/// Both parsers ran and the deep one is under [`MIN_SPEEDUP`].
pub const EXIT_GATE: i32 = 1;

const USAGE: &str = "usage: rust-graph-deep-bench corpus --list FILE
       rust-graph-deep-bench compare --tree-sitter PATH --deep PATH --list FILE [--rounds N] [--reps N]
       rust-graph-deep-bench memory --deep PATH --list FILE";

/// Parse argv, run the command, and return the process exit code.
pub fn cli() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("rust-graph-deep-bench: {message}");
            EXIT_REFUSED
        }
    }
}

fn run(args: &[String]) -> Result<i32, String> {
    // `args` is the whole argv, program included: the report prints it back
    // verbatim, so the run can be repeated from the log.
    let (command, rest) = args.get(1..).unwrap_or_default().split_first().ok_or_else(|| USAGE.to_string())?;
    let options = Options::parse(rest)?;
    match command.as_str() {
        "corpus" => manifest::corpus(&options, args),
        "compare" => bench::compare(&options, args),
        "memory" => bench::memory(&options, args),
        "--version" => {
            println!("rust-graph-deep-bench {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(0)
        }
        other => Err(format!("unknown command {other:?}\n{USAGE}")),
    }
}

/// The flags every subcommand reads. Anything unknown is an error: a typo must
/// never silently measure a different corpus.
#[derive(Debug, Default)]
pub(crate) struct Options {
    pub list: Option<String>,
    pub tree_sitter: Option<String>,
    pub deep: Option<String>,
    pub rounds: Option<usize>,
    pub reps: Option<usize>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Options, String> {
        let mut options = Options::default();
        let mut at = 0;
        while at < args.len() {
            let flag = args[at].as_str();
            let value = |at: usize| args.get(at + 1).cloned().ok_or_else(|| format!("{flag} needs a value"));
            match flag {
                "--list" => {
                    options.list = Some(value(at)?);
                    at += 2;
                }
                "--tree-sitter" => {
                    options.tree_sitter = Some(value(at)?);
                    at += 2;
                }
                "--deep" => {
                    options.deep = Some(value(at)?);
                    at += 2;
                }
                "--rounds" => {
                    let text = value(at)?;
                    let rounds = text.parse::<usize>().map_err(|_| format!("--rounds needs a number, not {text:?}"))?;
                    if rounds == 0 {
                        return Err("--rounds must be at least 1".into());
                    }
                    options.rounds = Some(rounds);
                    at += 2;
                }
                "--reps" => {
                    let text = value(at)?;
                    let reps = text.parse::<usize>().map_err(|_| format!("--reps needs a number, not {text:?}"))?;
                    if reps == 0 {
                        return Err("--reps must be at least 1".into());
                    }
                    options.reps = Some(reps);
                    at += 2;
                }
                other => return Err(format!("unknown option {other:?}\n{USAGE}")),
            }
        }
        Ok(options)
    }

    pub(crate) fn list(&self) -> Result<&str, String> {
        self.list.as_deref().ok_or_else(|| format!("--list is required\n{USAGE}"))
    }

    /// The `PATH` a subcommand needs, or a message naming the flag.
    pub(crate) fn program<'a>(&self, flag: &str, value: &'a Option<String>) -> Result<&'a str, String> {
        value.as_deref().ok_or_else(|| format!("{flag} is required\n{USAGE}"))
    }
}

// ---------------------------------------------------------------- the machine

/// The lines every report starts with, so a number can be reproduced or argued
/// with.
pub(crate) fn environment(argv: &[String]) {
    println!("machine: {} / {}", os_name(), host_name());
    println!("cpu: {}", cpu_model());
    println!("cores: {}", std::thread::available_parallelism().map_or(1, |count| count.get()));
    println!("command: {}", shell_join(argv));
    println!();
}

fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|name| name.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn host_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|name| name.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("model name"))
                .and_then(|rest| rest.split_once(':').map(|(_, model)| model.trim().to_string()))
        })
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

fn shell_join(argv: &[String]) -> String {
    argv.iter().map(|argument| shell_quote(argument)).collect::<Vec<_>>().join(" ")
}

fn shell_quote(argument: &str) -> String {
    let plain = !argument.is_empty() && argument.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@,+".contains(c));
    if plain {
        argument.to_string()
    } else {
        format!("'{}'", argument.replace('\'', "'\\''"))
    }
}

/// The command as it would be typed, for the "cannot run" errors.
pub(crate) fn describe(command: &Command) -> String {
    let mut parts = vec![shell_quote(&command.get_program().to_string_lossy())];
    parts.extend(command.get_args().map(|argument| shell_quote(&argument.to_string_lossy())));
    parts.join(" ")
}

pub(crate) fn tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().rev().take(8).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// A missing parser is a refusal, not a zero: the message says what is missing
/// and, for the tree-sitter side, why this machine may not be able to build it.
pub(crate) fn check_program(program: &str, flag: &str) -> Result<(), String> {
    let path = Path::new(program);
    if !path.exists() {
        let mut message = format!("{flag}: {program} does not exist");
        if flag == "--tree-sitter" {
            message.push_str(&format!(
                "; Tree-sitter is not installed here and there is no C compiler to build it from source, so no ratio can be measured.\n  {}",
                toolchain()
            ));
        }
        return Err(message);
    }
    let meta = std::fs::metadata(path).map_err(|error| format!("{flag}: {program}: {error}"))?;
    if !meta.is_file() {
        return Err(format!("{flag}: {program} is not a file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(format!("{flag}: {program} is not executable"));
        }
    }
    Ok(())
}

fn toolchain() -> String {
    let mut found = Vec::new();
    for name in ["cc", "gcc", "clang", "tree-sitter"] {
        if let Some(path) = std::env::var_os("PATH") {
            let hit = std::env::split_paths(&path).map(|dir| dir.join(name)).find(|candidate| candidate.is_file());
            found.push(format!("{name}={}", hit.map_or_else(|| "missing".into(), |path| path.display().to_string())));
        }
    }
    format!("toolchain: {}", found.join(" "))
}

// ---------------------------------------------------------------------- sha

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
