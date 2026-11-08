//! Reading a directory of inputs, and the shared hostile seed corpus.
//!
//! `corpus --dir` runs every file in a directory: no extension filter (the
//! caller names the language or relies on the extension), no size cap beyond a
//! panic guard, recursive, sorted, symlinks not followed. Sorting means two
//! runs report the same order.

use std::path::{Path, PathBuf};

use crate::registry::{by_extension, Entry};

/// Every regular file under `dir`, sorted by path. Symlinks are not followed.
pub fn files_in(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Err(format!("{dir:?} is not a directory"));
    }
    let mut files = Vec::new();
    walk(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{dir:?}: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{dir:?}: {error}"))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| format!("{path:?}: {error}"))?;
        if kind.is_dir() {
            walk(&path, out)?;
        } else if kind.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// The language a path's extension names, if the registry knows it.
pub fn language_for_path(path: &Path) -> Option<&'static Entry> {
    let extension = path.extension()?.to_str()?;
    by_extension(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hostile_corpus_is_present_and_non_empty() {
        let dir = crate::registry::corpus_root();
        let files = files_in(&dir).expect("corpus/seeds exists");
        assert!(files.len() >= 5, "only {} hostile seeds", files.len());
        for file in &files {
            let bytes = std::fs::read(file).unwrap();
            assert!(!bytes.is_empty(), "{file:?} is empty");
        }
    }

    #[test]
    fn extension_lookup_is_case_insensitive() {
        assert_eq!(language_for_path(Path::new("a/b.py")).unwrap().name, "python");
        assert_eq!(language_for_path(Path::new("A.R")).unwrap().name, "r");
        assert!(language_for_path(Path::new("Makefile")).is_none());
    }

    #[test]
    fn a_missing_directory_is_an_error_not_an_empty_list() {
        assert!(files_in(Path::new("/nonexistent/deep-fuzz-corpus")).is_err());
    }
}
