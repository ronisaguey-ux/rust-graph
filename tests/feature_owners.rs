//! Every feature flag names an owner and a removal condition.
//!
//! A flag is a phase, not a permanent part of the language. The failure this
//! catches is the one that already happened once: `event-split` sat in
//! Cargo.toml for a phase that ended, and nothing in the build said it was time
//! to take it out. The rule here is cheap to satisfy and impossible to forget —
//! the comment directly above a feature names who owns it and what removes it,
//! so the next person to touch the manifest knows whether the flag is still
//! earning its place.

use std::fs;
use std::path::{Path, PathBuf};

fn manifests() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut found = vec![root.join("Cargo.toml")];
    if let Ok(entries) = fs::read_dir(root.join("crates")) {
        for entry in entries.flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if manifest.is_file() {
                found.push(manifest);
            }
        }
    }
    found
}

/// The features declared in one manifest and the comment block above each.
fn features_in(path: &Path) -> Vec<(String, Vec<String>)> {
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    let mut inside = false;
    let mut pending: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[features]";
            pending.clear();
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix('#') {
            pending.push(comment.trim().to_string());
            continue;
        }
        if trimmed.is_empty() || !trimmed.contains('=') {
            continue;
        }
        let name = trimmed.split('=').next().unwrap_or_default().trim().to_string();
        if !name.is_empty() {
            out.push((name, std::mem::take(&mut pending)));
        } else {
            pending.clear();
        }
    }
    out
}

#[test]
fn every_feature_names_its_owner_and_its_removal() {
    let mut checked = 0;
    for manifest in manifests() {
        for (name, comment) in features_in(&manifest) {
            let text = comment.join(" ");
            let owner = text.contains("owner:");
            let removal = text.contains("removal:");
            assert!(
                owner && removal,
                "{}: feature `{}` has no {}. Add a comment above it: \n# owner: <who> | removal: <commit, plan id, or the condition that ends the phase>",
                manifest.display(),
                name,
                if owner { "removal condition" } else { "owner" }
            );
            checked += 1;
        }
    }
    assert!(checked >= 3, "expected the workspace features to be checked, saw {checked}");
}
