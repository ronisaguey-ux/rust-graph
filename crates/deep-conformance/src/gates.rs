//! The four project gates and the two-report diff.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::report::Report;
use crate::summary::{
    format_opt, per_language, percentile, LanguageStats, CLEAN_MIN_FILES, CLEAN_MIN_PCT,
    COLD_START_MAX_US, DIFF_CLEAN_DROP_PP, DIFF_SPEED_FACTOR, RSS_MAX_BYTES, SPEED_MIN_MBPS,
};

/// One gate: its statement, verdict, and the measurement that decided it.
#[derive(Clone, Debug)]
pub struct GateResult {
    pub name: &'static str,
    pub pass: bool,
    pub detail: String,
}

/// The project gates, in one place.
pub fn gate_results(report: &Report) -> Vec<GateResult> {
    let stats = per_language(report);
    let mut gates = Vec::new();

    // Gate 1: clean rate per language, with a real denominator. The deciding
    // language is the worst one at or above the file floor; a language with no
    // parser is the worst possible result and fails first.
    let gated: Vec<&LanguageStats> = stats.iter().filter(|row| row.files >= CLEAN_MIN_FILES).collect();
    let unmeasured: Vec<&&LanguageStats> = gated.iter().filter(|row| row.measured == 0).collect();
    let worst = gated
        .iter()
        .filter_map(|row| row.clean_rate().map(|rate| (rate, *row)))
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (pass, detail) = if gated.is_empty() {
        (
            false,
            format!("no language has at least {CLEAN_MIN_FILES} files; the gate has nothing to measure"),
        )
    } else if let Some(row) = unmeasured.first() {
        (
            false,
            format!("{}: not measured (0 of {} files have a parser)", row.language, row.files),
        )
    } else if let Some((rate, row)) = worst {
        (
            rate >= CLEAN_MIN_PCT,
            format!(
                "{}: {:.1}% clean ({}/{}), floor {:.0}%",
                row.language, rate, row.clean, row.measured, CLEAN_MIN_PCT
            ),
        )
    } else {
        (false, "no gated language produced a measurement".to_string())
    };
    gates.push(GateResult {
        name: "clean rate >= 95% per language (min 10 files)",
        pass,
        detail,
    });

    // Gate 2: aggregate throughput over every measured file. The per-file
    // median is printed next to it because it exposes constant per-file
    // overhead, but the gate is the whole-corpus number: that is what the
    // project's existing 2x comparison measures and what a release promises.
    let mut speeds: Vec<f64> = stats.iter().filter_map(|row| row.median_mbps).collect();
    let total_bytes: u64 = stats.iter().map(|row| row.measured_bytes).sum();
    let total_ms: f64 = report
        .files
        .iter()
        .filter(|file| file.outcome.measured())
        .filter_map(|file| file.ms)
        .sum();
    speeds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let aggregate = if total_bytes > 0 && total_ms > 0.0 {
        (total_bytes as f64 / 1_000_000.0) / (total_ms / 1000.0)
    } else {
        0.0
    };
    let per_file_median = percentile(&speeds, 0.50);
    let (pass, detail) = if total_bytes == 0 || total_ms <= 0.0 {
        (false, "no file was timed; speed was not measured".to_string())
    } else {
        (
            aggregate >= SPEED_MIN_MBPS,
            format!(
                "aggregate {aggregate:.2} MB/s over {total_bytes} bytes (floor {SPEED_MIN_MBPS:.0}); per-file median {} MB/s",
                format_opt(per_file_median, 2)
            ),
        )
    };
    gates.push(GateResult { name: "median parse >= 15 MB/s (aggregate)", pass, detail });

    // Gate 3: peak RSS after any single parse. The scanner samples VmHWM after
    // every file, so the maximum is the highest the process ever reached.
    let peak = report
        .files
        .iter()
        .filter_map(|file| file.rss_bytes.map(|bytes| (bytes, file.path.as_str())))
        .max_by_key(|(bytes, _)| *bytes);
    let (pass, detail) = match peak {
        Some((bytes, path)) => (
            bytes <= RSS_MAX_BYTES,
            format!(
                "{:.1} MB peak after {path} (limit {:.1} MB)",
                bytes as f64 / (1024.0 * 1024.0),
                RSS_MAX_BYTES as f64 / (1024.0 * 1024.0)
            ),
        ),
        None => (false, "no peak RSS was recorded by this platform".to_string()),
    };
    gates.push(GateResult { name: "no file over 2 GiB RSS", pass, detail });

    // Gate 4: cold start. The scan samples process age when it is ready to
    // walk, so this is binary load plus argument parsing plus registry
    // construction; it does not include corpus I/O.
    let (pass, detail) = match report.cold_start_us {
        Some(us) => (
            us < COLD_START_MAX_US,
            format!("{:.3} ms process age when ready to parse (limit {:.3} ms)", us / 1000.0, COLD_START_MAX_US / 1000.0),
        ),
        None => (false, "the platform did not report process age, so cold start was not measured".to_string()),
    };
    gates.push(GateResult { name: "cold start < 1 ms", pass, detail });

    gates
}

/// The gate table. Returns the text and whether every gate passed.
pub fn gates_text(report: &Report) -> (String, bool) {
    let gates = gate_results(report);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "deep-conformance gates  root={}  files={}  measured={}",
        report.root,
        report.files.len(),
        report.files.iter().filter(|file| file.outcome.measured()).count()
    );
    let width = gates.iter().map(|gate| gate.name.len()).max().unwrap_or(10);
    let _ = writeln!(out, "{:<width$}  {:<6}  {}", "gate", "result", "measured", width = width);
    let mut all = true;
    for gate in &gates {
        all &= gate.pass;
        let _ = writeln!(
            out,
            "{:<width$}  {:<6}  {}",
            gate.name,
            if gate.pass { "PASS" } else { "FAIL" },
            gate.detail,
            width = width
        );
    }
    let _ = writeln!(out, "\nresult: {}", if all { "PASS" } else { "FAIL" });
    (out, all)
}

fn rate_display(row: &LanguageStats) -> String {
    match row.clean_rate() {
        Some(rate) => format!("{rate:.1}% ({}/{})", row.clean, row.measured),
        None => "not measured".to_string(),
    }
}

/// Compare two reports per language and flag clean-rate and speed regressions.
/// Returns the text and whether there is no regression.
pub fn diff_text(a: &Report, b: &Report) -> (String, bool) {
    let left: BTreeMap<String, LanguageStats> = per_language(a).into_iter().map(|row| (row.language.clone(), row)).collect();
    let right: BTreeMap<String, LanguageStats> = per_language(b).into_iter().map(|row| (row.language.clone(), row)).collect();
    let mut names: Vec<&String> = left.keys().chain(right.keys()).collect();
    names.sort();
    names.dedup();

    let mut out = String::new();
    let _ = writeln!(out, "deep-conformance diff  a={}  b={}", a.root, b.root);
    let width = names.iter().map(|name| name.len()).max().unwrap_or(8).max(8);
    let _ = writeln!(
        out,
        "{:<width$}  {:<24}  {:<24}  {:<18}  {}",
        "language",
        "clean a -> b",
        "p50 ms a -> b",
        "speed factor",
        "verdict",
        width = width
    );

    let mut regressions = 0u64;
    for name in names {
        match (left.get(name), right.get(name)) {
            (Some(before), Some(after)) => {
                let mut flags = Vec::new();
                if before.measured > 0 && after.measured == 0 {
                    flags.push("REGRESSION: not measured now".to_string());
                } else if before.measured > 0 && after.measured > 0 {
                    let rate_before = before.clean_rate().unwrap_or(0.0);
                    let rate_after = after.clean_rate().unwrap_or(0.0);
                    if rate_before - rate_after > DIFF_CLEAN_DROP_PP {
                        flags.push(format!("REGRESSION: clean {:.1}pp", rate_after - rate_before));
                    }
                }
                let factor = match (before.p50_ms, after.p50_ms) {
                    (Some(old), Some(new)) if old > 0.0 => Some(new / old),
                    _ => None,
                };
                if let Some(factor) = factor {
                    if factor > DIFF_SPEED_FACTOR {
                        flags.push(format!("REGRESSION: p50 {factor:.2}x slower"));
                    }
                }
                let verdict = if flags.is_empty() { "ok".to_string() } else { flags.join("; ") };
                if !flags.is_empty() {
                    regressions += 1;
                }
                let _ = writeln!(
                    out,
                    "{:<width$}  {:<24}  {:<24}  {:<18}  {}",
                    name,
                    format!("{} -> {}", rate_display(before), rate_display(after)),
                    format!("{} -> {}", format_opt(before.p50_ms, 3), format_opt(after.p50_ms, 3)),
                    match factor {
                        Some(factor) => format!("{factor:.2}x"),
                        None => "-".to_string(),
                    },
                    verdict,
                    width = width
                );
            }
            (Some(before), None) => {
                let lost = before.files >= CLEAN_MIN_FILES;
                if lost {
                    regressions += 1;
                }
                let _ = writeln!(
                    out,
                    "{:<width$}  {:<24}  {:<24}  {:<18}  {}",
                    name,
                    format!("{} -> absent", rate_display(before)),
                    "-",
                    "-",
                    if lost { "REGRESSION: language missing from b" } else { "only in a" },
                    width = width
                );
            }
            (None, Some(after)) => {
                let _ = writeln!(
                    out,
                    "{:<width$}  {:<24}  {:<24}  {:<18}  {}",
                    name,
                    format!("absent -> {}", rate_display(after)),
                    "-",
                    "-",
                    "new in b",
                    width = width
                );
            }
            (None, None) => {}
        }
    }
    let _ = writeln!(
        out,
        "\nregression rules: clean rate down more than {DIFF_CLEAN_DROP_PP:.1}pp, or p50 slower by more than {:.0}%",
        (DIFF_SPEED_FACTOR - 1.0) * 100.0
    );
    let _ = writeln!(out, "result: {} ({} regressions)", if regressions == 0 { "PASS" } else { "FAIL" }, regressions);
    (out, regressions == 0)
}

