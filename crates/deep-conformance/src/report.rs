//! The report: one record per file, written to JSON so `summary`, `gates` and
//! `diff` can run on a scan from another process (or another machine).
//!
//! `None` means "not measured", and it is written as JSON `null`. A file with
//! no parser has no milliseconds, and a report that printed `0.0` for it would
//! make an unmeasured file look infinitely fast.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use crate::json::{self, Value};

/// What came out of one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A tree with no error node.
    Clean,
    /// A tree with at least one error node: the parser recovered.
    Errors,
    /// No tree at all.
    Fatal,
    /// The language is known but no parser is registered.
    NotMeasured,
    /// The file could not be read.
    Unreadable,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Clean => "clean",
            Outcome::Errors => "errors",
            Outcome::Fatal => "fatal",
            Outcome::NotMeasured => "not_measured",
            Outcome::Unreadable => "unreadable",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "clean" => Some(Outcome::Clean),
            "errors" => Some(Outcome::Errors),
            "fatal" => Some(Outcome::Fatal),
            "not_measured" => Some(Outcome::NotMeasured),
            "unreadable" => Some(Outcome::Unreadable),
            _ => None,
        }
    }

    /// A parser ran and produced an answer, clean or not. This is the only set
    /// that may enter a clean-rate denominator.
    pub fn measured(self) -> bool {
        matches!(self, Outcome::Clean | Outcome::Errors | Outcome::Fatal)
    }
}

/// One file's measurement. Every number here came from this scan.
#[derive(Clone, Debug)]
pub struct FileRecord {
    /// Relative to the scanned root, `/` separated.
    pub path: String,
    pub language: String,
    pub bytes: u64,
    pub outcome: Outcome,
    pub error_nodes: u64,
    pub error_bytes: u64,
    pub nodes: u64,
    pub max_depth: u32,
    /// Wall time around the parser call, in milliseconds. `None` if no parser ran.
    pub ms: Option<f64>,
    /// Peak RSS of the scanning process after this file, in bytes.
    pub rss_bytes: Option<u64>,
    /// Why the file is fatal/unreadable, or `None`.
    pub reason: Option<String>,
}

impl FileRecord {
    pub(crate) fn not_measured(path: String, language: &str, bytes: u64, rss_bytes: Option<u64>) -> Self {
        FileRecord {
            path,
            language: language.to_string(),
            bytes,
            outcome: Outcome::NotMeasured,
            error_nodes: 0,
            error_bytes: 0,
            nodes: 0,
            max_depth: 0,
            ms: None,
            rss_bytes,
            reason: None,
        }
    }
}

/// A whole scan.
#[derive(Clone, Debug)]
pub struct Report {
    pub root: String,
    /// Process age at the moment the scan was ready to walk: binary load,
    /// argument parsing and registry construction. Corpus I/O is excluded.
    /// `None` when the platform cannot report it, which fails the cold-start
    /// gate rather than passing it.
    pub cold_start_us: Option<f64>,
    /// Wall time of the whole walk and scan.
    pub scan_ms: f64,
    /// Files whose extension is not a known language.
    pub skipped: u64,
    /// Directories that could not be read.
    pub walk_errors: u64,
    pub files: Vec<FileRecord>,
}

impl Report {
    /// Serialize the report. Field order is stable so reports diff cleanly.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(256 + self.files.len() * 180);
        out.push_str("{\n  \"tool\": \"deep-conformance\",\n  \"version\": 1,\n  \"root\": ");
        json::push_string(&mut out, &self.root);
        out.push_str(",\n  \"cold_start_us\": ");
        json::push_number(&mut out, self.cold_start_us);
        out.push_str(",\n  \"scan_ms\": ");
        json::push_number(&mut out, Some(self.scan_ms));
        let _ = write!(out, ",\n  \"skipped\": {},\n  \"walk_errors\": {},\n  \"files\": [", self.skipped, self.walk_errors);
        for (index, file) in self.files.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("\n    {\"path\": ");
            json::push_string(&mut out, &file.path);
            out.push_str(", \"language\": ");
            json::push_string(&mut out, &file.language);
            let _ = write!(out, ", \"bytes\": {}, \"outcome\": ", file.bytes);
            json::push_string(&mut out, file.outcome.as_str());
            let _ = write!(
                out,
                ", \"error_nodes\": {}, \"error_bytes\": {}, \"nodes\": {}, \"max_depth\": {}, \"ms\": ",
                file.error_nodes, file.error_bytes, file.nodes, file.max_depth
            );
            json::push_number(&mut out, file.ms);
            out.push_str(", \"rss_bytes\": ");
            json::push_number(&mut out, file.rss_bytes.map(|bytes| bytes as f64));
            out.push_str(", \"reason\": ");
            match &file.reason {
                Some(reason) => json::push_string(&mut out, reason),
                None => out.push_str("null"),
            }
            out.push('}');
        }
        out.push_str("\n  ]\n}\n");
        out
    }

    /// Read a report back. A missing or malformed field is an error, not a
    /// default: a zero substituted for a missing measurement would be a lie.
    pub fn from_json(text: &str) -> Result<Report, String> {
        let value = json::parse(text)?;
        let root = value.get("root").and_then(Value::as_str).ok_or("report has no root")?.to_string();
        let cold_start_us = match value.get("cold_start_us") {
            Some(Value::Null) | None => None,
            Some(other) => Some(other.as_f64().ok_or("cold_start_us is not a number")?),
        };
        let scan_ms = value.get("scan_ms").and_then(Value::as_f64).ok_or("report has no scan_ms")?;
        let skipped = value.get("skipped").and_then(Value::as_u64).unwrap_or(0);
        let walk_errors = value.get("walk_errors").and_then(Value::as_u64).unwrap_or(0);
        let rows = value.get("files").and_then(Value::as_array).ok_or("report has no files array")?;
        let mut files = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let field = |name: &str| -> Result<&Value, String> {
                row.get(name).ok_or_else(|| format!("file {index} has no {name}"))
            };
            let path = field("path")?.as_str().ok_or_else(|| format!("file {index}: path is not a string"))?.to_string();
            let language =
                field("language")?.as_str().ok_or_else(|| format!("file {index}: language is not a string"))?.to_string();
            let bytes = field("bytes")?.as_u64().ok_or_else(|| format!("file {index}: bytes is not a number"))?;
            let outcome_text = field("outcome")?.as_str().ok_or_else(|| format!("file {index}: outcome is not a string"))?;
            let outcome = Outcome::parse(outcome_text)
                .ok_or_else(|| format!("file {index}: unknown outcome {outcome_text:?}"))?;
            let error_nodes = field("error_nodes")?.as_u64().unwrap_or(0);
            let error_bytes = field("error_bytes")?.as_u64().unwrap_or(0);
            let nodes = field("nodes")?.as_u64().unwrap_or(0);
            let max_depth = field("max_depth")?.as_u64().unwrap_or(0) as u32;
            let ms = match row.get("ms") {
                Some(Value::Null) | None => None,
                Some(other) => Some(other.as_f64().ok_or_else(|| format!("file {index}: ms is not a number"))?),
            };
            let rss_bytes = match row.get("rss_bytes") {
                Some(Value::Null) | None => None,
                Some(other) => Some(other.as_u64().ok_or_else(|| format!("file {index}: rss_bytes is not a number"))?),
            };
            let reason = row.get("reason").and_then(Value::as_str).map(str::to_string);
            files.push(FileRecord {
                path,
                language,
                bytes,
                outcome,
                error_nodes,
                error_bytes,
                nodes,
                max_depth,
                ms,
                rss_bytes,
                reason,
            });
        }
        Ok(Report { root, cold_start_us, scan_ms, skipped, walk_errors, files })
    }

    pub fn load(path: &Path) -> Result<Report, String> {
        let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        Report::from_json(&text)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        fs::write(path, self.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str, outcome: Outcome, bytes: u64, ms: Option<f64>) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            language: "Python".to_string(),
            bytes,
            outcome,
            error_nodes: 0,
            error_bytes: 0,
            nodes: 10,
            max_depth: 2,
            ms,
            rss_bytes: Some(1 << 20),
            reason: None,
        }
    }

    #[test]
    fn report_round_trips_through_json() {
        let report = Report {
            root: "/tmp/x".to_string(),
            cold_start_us: Some(12.5),
            scan_ms: 34.0,
            skipped: 2,
            walk_errors: 0,
            files: vec![record("a.py", Outcome::Clean, 10, Some(0.25)), {
                let mut file = record("b.py", Outcome::NotMeasured, 20, None);
                file.reason = Some("no parser".to_string());
                file
            }],
        };
        let back = Report::from_json(&report.to_json()).unwrap();
        assert_eq!(back.root, report.root);
        assert_eq!(back.cold_start_us, report.cold_start_us);
        assert_eq!(back.files.len(), 2);
        assert_eq!(back.files[1].outcome, Outcome::NotMeasured);
        assert_eq!(back.files[1].ms, None);
    }

    #[test]
    fn a_missing_measurement_stays_null() {
        let report = Report {
            root: "/tmp/x".to_string(),
            cold_start_us: None,
            scan_ms: 1.0,
            skipped: 0,
            walk_errors: 0,
            files: vec![record("a.py", Outcome::NotMeasured, 10, None)],
        };
        let text = report.to_json();
        assert!(text.contains("\"ms\": null"), "{text}");
        assert!(text.contains("\"cold_start_us\": null"), "{text}");
        assert!(text.contains("\"rss_bytes\": 1048576"), "{text}");
    }
}
