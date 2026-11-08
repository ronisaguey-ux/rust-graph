//! The corpus: a manifest that has been read and verified. Nothing is timed
//! until every path in it exists, so a report can never describe a corpus other
//! than the one the list names.

use std::path::{Path, PathBuf};

use super::{environment, hex, sha256, Options};

#[derive(Debug)]
/// A manifest that has been read and verified: the tool never reports numbers
/// from a corpus it could not see in full.
pub(crate) struct Manifest {
    /// The manifest as it was named on the command line, for the report.
    pub(crate) path: String,
    /// SHA-256 of the manifest's own bytes.
    pub(crate) digest: String,
    pub(crate) bytes: u64,
    pub(crate) files: Vec<PathBuf>,
}

impl Manifest {
    pub(crate) fn load(path: &str) -> Result<Manifest, String> {
        let text = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
        let digest = hex(&sha256::digest(&text));
        let text = String::from_utf8_lossy(&text);
        let mut files = Vec::new();
        let mut bytes = 0u64;
        let mut problems = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let file = Path::new(line);
            if !file.is_absolute() {
                problems.push(format!("{path}:{}: {line} is not an absolute path", number + 1));
                continue;
            }
            match std::fs::metadata(file) {
                Err(error) => problems.push(format!("{path}:{}: {line}: {error}", number + 1)),
                Ok(meta) if !meta.is_file() => problems.push(format!("{path}:{}: {line} is not a regular file", number + 1)),
                Ok(meta) => {
                    bytes += meta.len();
                    files.push(file.to_path_buf());
                }
            }
        }
        if !problems.is_empty() {
            let shown: Vec<&str> = problems.iter().take(12).map(String::as_str).collect();
            return Err(format!(
                "refusing to report: {} manifest entr{} cannot be measured:\n  {}{}",
                problems.len(),
                if problems.len() == 1 { "y" } else { "ies" },
                shown.join("\n  "),
                if problems.len() > shown.len() { format!("\n  ... and {} more", problems.len() - shown.len()) } else { String::new() }
            ));
        }
        if files.is_empty() {
            return Err(format!("{path}: the manifest names no files"));
        }
        Ok(Manifest { path: path.to_string(), digest, bytes, files })
    }

    /// The one line every report carries: what was measured and which manifest
    /// said so.
    pub(crate) fn summary(&self) -> String {
        format!("manifest: {}  files: {}  bytes: {}  sha256: {}", self.path, self.files.len(), self.bytes, self.digest)
    }
}

/// `corpus`: the manifest on its own, for a quick sanity check of a list.
pub(super) fn corpus(options: &Options, argv: &[String]) -> Result<i32, String> {
    let manifest = Manifest::load(options.list()?)?;
    environment(argv);
    println!("{}", manifest.summary());
    Ok(0)
}
