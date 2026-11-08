//! Walking the tree and measuring each file.
//!
//! Kept apart from the report types so the measurement path (filesystem,
//! panics, process counters) and the data it produces can be read separately.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::registry::{ParseFacts, Registry};
use crate::report::{FileRecord, Outcome, Report};

/// Walk `root`, parse every file whose extension names a registered or
/// registerable language, and measure it. A single bad file is a record, never
/// an abort.
///
/// Cold start is measured from this call. The CLI calls [`scan_from`] with an
/// instant captured at process entry, so the binary reports the real startup.
pub fn scan(root: &Path, registry: &Registry) -> io::Result<Report> {
    scan_from(root, registry, Instant::now())
}

/// The same scan, with the starting instant supplied by the caller.
pub fn scan_from(root: &Path, registry: &Registry, command_start: Instant) -> io::Result<Report> {
    let metadata = fs::metadata(root)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not a directory", root.display())));
    }

    let cold_start_us = Some(command_start.elapsed().as_secs_f64() * 1_000_000.0);
    let scan_start = Instant::now();
    let mut paths = Vec::new();
    let mut walk_errors = 0u64;
    walk(root, &mut paths, &mut walk_errors);
    paths.sort();

    let mut files = Vec::with_capacity(paths.len());
    let mut skipped = 0u64;

    for path in &paths {
        let extension = match path.extension().and_then(|ext| ext.to_str()) {
            Some(extension) => extension.to_ascii_lowercase(),
            None => {
                skipped += 1;
                continue;
            }
        };
        let spec = match registry.language_for_extension(&extension) {
            Some(spec) => spec,
            None => {
                skipped += 1;
                continue;
            }
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");

        let source = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                let rss = peak_rss_bytes();
                files.push(FileRecord {
                    path: relative,
                    language: spec.name.to_string(),
                    bytes: fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
                    outcome: Outcome::Unreadable,
                    error_nodes: 0,
                    error_bytes: 0,
                    nodes: 0,
                    max_depth: 0,
                    ms: None,
                    rss_bytes: rss,
                    reason: Some(error.to_string()),
                });
                continue;
            }
        };

        let bytes = source.len() as u64;
        let parser = match registry.parser_for(spec.name, &extension) {
            Some(parser) => parser,
            None => {
                let rss = peak_rss_bytes();
                files.push(FileRecord::not_measured(relative, spec.name, bytes, rss));
                continue;
            }
        };

        let started = Instant::now();
        let facts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parser(&source)));
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let rss = peak_rss_bytes();

        let (outcome, reason, facts) = match facts {
            Ok(facts) => {
                let outcome = if facts.fatal.is_some() {
                    Outcome::Fatal
                } else if facts.error_nodes > 0 || facts.error_bytes > 0 {
                    Outcome::Errors
                } else {
                    Outcome::Clean
                };
                (outcome, facts.fatal.map(str::to_string), facts)
            }
            Err(_) => (Outcome::Fatal, Some("parser panicked".to_string()), ParseFacts::fatal("panic")),
        };

        files.push(FileRecord {
            path: relative,
            language: spec.name.to_string(),
            bytes,
            outcome,
            error_nodes: facts.error_nodes,
            error_bytes: facts.error_bytes,
            nodes: facts.nodes,
            max_depth: facts.max_depth,
            ms: Some(ms),
            rss_bytes: rss,
            reason,
        });
    }

    Ok(Report {
        root: root.to_string_lossy().to_string(),
        cold_start_us,
        scan_ms: scan_start.elapsed().as_secs_f64() * 1000.0,
        skipped,
        walk_errors,
        files,
    })
}

/// Collect regular files without following symlinks, so a link loop cannot
/// hang the scan and a link to /dev/zero cannot fill memory.
fn walk(dir: &Path, out: &mut Vec<PathBuf>, errors: &mut u64) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => {
            *errors += 1;
            return;
        }
    };
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                *errors += 1;
                continue;
            }
        };
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        let file_type = match fs::symlink_metadata(&child) {
            Ok(meta) => meta.file_type(),
            Err(_) => {
                *errors += 1;
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            walk(&child, out, errors);
        } else if file_type.is_file() {
            out.push(child);
        }
    }
}

/// Peak resident set of this process, from Linux `VmHWM`. Other platforms get
/// `None`, and the RSS gate then fails as unmeasured rather than passing.
pub fn peak_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = fs::read_to_string("/proc/self/status").ok()?;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("VmHWM:") {
                let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some(kib * 1024);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// How long the process has existed, from `/proc` (10 ms resolution on most
/// kernels). Kept for diagnostics; the gate uses [`scan_from`]'s nanosecond
/// instant because a 1 ms gate cannot be decided by a 10 ms clock.
#[allow(dead_code)]
pub fn process_age_us() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        // USER_HZ is 100 on every Linux ABI this project targets; the field is
        // documented in proc(5).
        const USER_HZ: f64 = 100.0;
        let stat = fs::read_to_string("/proc/self/stat").ok()?;
        // The command name is parenthesized and may contain spaces, so fields
        // are counted after the last ')'.
        let rest = stat.rsplit_once(')')?.1;
        // rest starts at field 3 (state). starttime is field 22, index 19 here.
        let start_ticks: f64 = rest.split_whitespace().nth(19)?.parse().ok()?;
        let uptime = fs::read_to_string("/proc/uptime").ok()?;
        let uptime_s: f64 = uptime.split_whitespace().next()?.parse().ok()?;
        let age_us = uptime_s * 1_000_000.0 - (start_ticks / USER_HZ) * 1_000_000.0;
        Some(age_us.max(0.0))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

