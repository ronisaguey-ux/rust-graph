//! Aggregation, the project gates and the two-report diff.
//!
//! Every number printed here is derived from the [`Report`] it is given, which
//! in turn was produced by a measurement (or is `null`, which is printed as
//! "not measured" and never as a zero). The gates are stated once, in
//! [`gate_results`], so `gates` is the single place a threshold can change.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::report::{Outcome, Report};

/// A language needs this many files before its clean rate can gate a release.
pub const CLEAN_MIN_FILES: u64 = 10;
/// The clean rate a language at or above [`CLEAN_MIN_FILES`] must reach.
pub const CLEAN_MIN_PCT: f64 = 95.0;
/// Aggregate parse throughput, in megabytes (10^6 bytes) per second: total
/// measured bytes divided by total measured parse time. This is the whole-corpus
/// number `deep-bench` compares across rounds, so it is the one a release gates.
pub const SPEED_MIN_MBPS: f64 = 15.0;
/// No single parse may leave the process above 2 GiB resident.
pub const RSS_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Cold start: process age when the parser is ready, in microseconds.
pub const COLD_START_MAX_US: f64 = 1000.0;
/// A clean rate that fell by more than this is a diff regression.
pub const DIFF_CLEAN_DROP_PP: f64 = 0.5;
/// A median parse time that grew by more than this factor is a diff regression.
pub const DIFF_SPEED_FACTOR: f64 = 1.10;

/// One language's share of a report.
#[derive(Clone, Debug, Default)]
pub struct LanguageStats {
    pub language: String,
    pub files: u64,
    pub measured: u64,
    pub clean: u64,
    pub not_measured: u64,
    pub unreadable: u64,
    pub error_bytes: u64,
    pub measured_bytes: u64,
    pub nodes: u64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub median_mbps: Option<f64>,
    pub aggregate_mbps: Option<f64>,
}

impl LanguageStats {
    /// `None` when no file in this language was parsed. A language with no
    /// measurement has no clean rate, and callers must not treat it as 100%.
    pub fn clean_rate(&self) -> Option<f64> {
        if self.measured == 0 {
            None
        } else {
            Some(self.clean as f64 * 100.0 / self.measured as f64)
        }
    }

    pub fn nodes_per_byte(&self) -> Option<f64> {
        if self.measured_bytes == 0 {
            None
        } else {
            Some(self.nodes as f64 / self.measured_bytes as f64)
        }
    }
}

/// Nearest-rank percentile on an already ascending slice. p is in 0..=1.
pub fn percentile(ascending: &[f64], p: f64) -> Option<f64> {
    if ascending.is_empty() {
        return None;
    }
    let rank = ((p * ascending.len() as f64).ceil() as usize).clamp(1, ascending.len());
    Some(ascending[rank - 1])
}

/// Group a report by language. Sorted, so output is deterministic.
pub fn per_language(report: &Report) -> Vec<LanguageStats> {
    struct Accum {
        stats: LanguageStats,
        times: Vec<f64>,
        speeds: Vec<f64>,
        total_ms: f64,
    }
    let mut map: BTreeMap<String, Accum> = BTreeMap::new();
    for file in &report.files {
        let entry = map.entry(file.language.clone()).or_insert_with(|| Accum {
            stats: LanguageStats { language: file.language.clone(), ..LanguageStats::default() },
            times: Vec::new(),
            speeds: Vec::new(),
            total_ms: 0.0,
        });
        let stats = &mut entry.stats;
        stats.files += 1;
        match file.outcome {
            Outcome::Clean => {
                stats.clean += 1;
                stats.measured += 1;
            }
            Outcome::Errors | Outcome::Fatal => stats.measured += 1,
            Outcome::NotMeasured => stats.not_measured += 1,
            Outcome::Unreadable => stats.unreadable += 1,
        }
        if file.outcome.measured() {
            stats.error_bytes += file.error_bytes;
            stats.measured_bytes += file.bytes;
            stats.nodes += file.nodes;
            if let Some(ms) = file.ms {
                entry.times.push(ms);
                entry.total_ms += ms;
                if file.bytes > 0 && ms > 0.0 {
                    let mbps = (file.bytes as f64 / 1_000_000.0) / (ms / 1000.0);
                    if mbps.is_finite() {
                        entry.speeds.push(mbps);
                    }
                }
            }
        }
    }
    map.into_values()
        .map(|mut accum| {
            accum.times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            accum.speeds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let measured_bytes = accum.stats.measured_bytes;
            let total_ms = accum.total_ms;
            let stats = &mut accum.stats;
            stats.p50_ms = percentile(&accum.times, 0.50);
            stats.p95_ms = percentile(&accum.times, 0.95);
            stats.max_ms = accum.times.last().copied();
            stats.median_mbps = percentile(&accum.speeds, 0.50);
            if measured_bytes > 0 && total_ms > 0.0 {
                stats.aggregate_mbps = Some((measured_bytes as f64 / 1_000_000.0) / (total_ms / 1000.0));
            }
            accum.stats
        })
        .collect()
}

pub(crate) fn format_opt(number: Option<f64>, decimals: usize) -> String {
    match number {
        Some(number) if number.is_finite() => format!("{number:.decimals$}"),
        _ => "-".to_string(),
    }
}

/// The summary table. Returns the text and whether every gated language met
/// [`CLEAN_MIN_PCT`]. A language with at least [`CLEAN_MIN_FILES`] files and no
/// measurement is a failure: unmeasured is not clean.
pub fn summary_text(report: &Report) -> (String, bool) {
    let stats = per_language(report);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "deep-conformance summary  root={}  files={}  skipped={}  walk_errors={}",
        report.root,
        report.files.len(),
        report.skipped,
        report.walk_errors
    );
    if stats.is_empty() {
        out.push_str("no file in this tree has a known language extension\n");
        return (out, false);
    }

    let widths = stats.iter().map(|row| row.language.len()).max().unwrap_or(8).max(8);
    let _ = writeln!(
        out,
        "{:<widths$}  {:>5}  {:>8}  {:>5}  {:<22}  {:>12}  {:>8}  {:>8}  {:>8}  {:>8}",
        "language",
        "files",
        "measured",
        "clean",
        "clean rate",
        "error bytes",
        "p50 ms",
        "p95 ms",
        "max ms",
        "nodes/B",
        widths = widths
    );
    let mut failures = Vec::new();
    for row in &stats {
        let rate = match row.clean_rate() {
            Some(rate) => format!("{rate:.1}% ({}/{})", row.clean, row.measured),
            None if row.files >= CLEAN_MIN_FILES => "not measured".to_string(),
            None => "not measured (small)".to_string(),
        };
        let _ = writeln!(
            out,
            "{:<widths$}  {:>5}  {:>8}  {:>5}  {:<22}  {:>12}  {:>8}  {:>8}  {:>8}  {:>8}",
            row.language,
            row.files,
            row.measured,
            row.clean,
            rate,
            row.error_bytes,
            format_opt(row.p50_ms, 3),
            format_opt(row.p95_ms, 3),
            format_opt(row.max_ms, 3),
            format_opt(row.nodes_per_byte(), 4),
            widths = widths
        );
        if row.files >= CLEAN_MIN_FILES {
            match row.clean_rate() {
                Some(rate) if rate >= CLEAN_MIN_PCT => {}
                Some(rate) => failures.push(format!(
                    "{}: {:.1}% clean ({}/{}) is under {:.0}%",
                    row.language, rate, row.clean, row.measured, CLEAN_MIN_PCT
                )),
                None => failures.push(format!(
                    "{}: not measured (0 of {} files have a parser)",
                    row.language, row.files
                )),
            }
        }
    }

    let unreadable: u64 = stats.iter().map(|row| row.unreadable).sum();
    let not_measured: u64 = stats.iter().map(|row| row.not_measured).sum();
    let _ = writeln!(
        out,
        "\ntotal: {} clean, {} with errors, {} fatal, {} not measured, {} unreadable",
        report.files.iter().filter(|file| file.outcome == Outcome::Clean).count(),
        report.files.iter().filter(|file| file.outcome == Outcome::Errors).count(),
        report.files.iter().filter(|file| file.outcome == Outcome::Fatal).count(),
        not_measured,
        unreadable
    );
    let _ = writeln!(
        out,
        "clean-rate floor: {CLEAN_MIN_PCT:.0}% per language with at least {CLEAN_MIN_FILES} files"
    );
    if failures.is_empty() {
        out.push_str("result: PASS\n");
        (out, true)
    } else {
        for failure in &failures {
            let _ = writeln!(out, "FAIL  {failure}");
        }
        out.push_str("result: FAIL\n");
        (out, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::FileRecord;

    fn record(language: &str, path: &str, outcome: Outcome, bytes: u64, ms: Option<f64>, rss: u64) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            language: language.to_string(),
            bytes,
            outcome,
            error_nodes: if outcome == Outcome::Errors { 1 } else { 0 },
            error_bytes: if outcome == Outcome::Errors { bytes } else { 0 },
            nodes: 10,
            max_depth: 2,
            ms,
            rss_bytes: Some(rss),
            reason: None,
        }
    }

    fn report(files: Vec<FileRecord>, cold_start_us: Option<f64>) -> Report {
        Report { root: "/x".to_string(), cold_start_us, scan_ms: 1.0, skipped: 0, walk_errors: 0, files }
    }

    #[test]
    fn nearest_rank_percentiles() {
        let values = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&values, 0.50), Some(2.0));
        assert_eq!(percentile(&values, 0.95), Some(4.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn clean_rate_has_the_measured_denominator() {
        let files = vec![
            record("Python", "a.py", Outcome::Clean, 100, Some(0.1), 1),
            record("Python", "b.py", Outcome::Errors, 100, Some(0.1), 1),
            record("Python", "c.py", Outcome::NotMeasured, 100, None, 1),
            record("Python", "d.py", Outcome::Unreadable, 100, None, 1),
        ];
        let stats = per_language(&report(files, Some(1.0)));
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].measured, 2);
        assert_eq!(stats[0].clean, 1);
        assert_eq!(stats[0].clean_rate(), Some(50.0));
    }

    #[test]
    fn a_language_with_no_measurement_has_no_rate() {
        let files: Vec<FileRecord> =
            (0..12).map(|index| record("Go", &format!("{index}.go"), Outcome::NotMeasured, 10, None, 1)).collect();
        let stats = per_language(&report(files, None));
        assert_eq!(stats[0].measured, 0);
        assert_eq!(stats[0].clean_rate(), None);
        assert_eq!(stats[0].nodes_per_byte(), None);
    }
}
