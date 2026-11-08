//! Every language the fuzzer knows, driven through the real parser.
//!
//! Two invariants:
//!
//! 1. every seed fixture parses and yields a well-formed tree — the fuzzer's
//!    checker must not be stricter than a shipped grammar;
//! 2. a short deterministic mutation run over every language finds nothing.
//!    The second is a regression net, not a proof: 30 s per language is the
//!    fuzzing run, and it is reported in `docs/DEEP_PARSER.md`.
//!
//! If a language is added to the registry without fixtures that load, the
//! first test fails rather than skipping it.

use deep_fuzz::harness::{check, LanguageTarget};
use deep_fuzz::mutate::{mutate, Rng};
use deep_fuzz::registry::{seeds_in, ENTRIES};

use std::path::Path;

#[test]
fn every_seed_fixture_parses_into_a_reachable_tree() {
    let root = deep_fuzz::fixtures_root();
    for entry in ENTRIES {
        let seeds = seeds_in(&root, entry).unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        let mut target = LanguageTarget::new(entry);
        for seed in &seeds {
            let report = check(&mut target, &seed.bytes);
            assert!(report.failure.is_none(), "{}: {}: {:?}", entry.name, seed.origin, report.failure);
        }
    }
}

#[test]
fn a_short_mutation_run_finds_nothing_in_any_language() {
    let root = deep_fuzz::fixtures_root();
    for entry in ENTRIES {
        let seeds = seeds_in(&root, entry).unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        let mut target = LanguageTarget::new(entry);
        let mut rng = Rng::new(0xF022_2026 ^ entry.name.len() as u64);
        for iteration in 0..250 {
            let input = mutate(&mut rng, &seeds[iteration % seeds.len()].bytes);
            let report = check(&mut target, &input);
            assert!(report.failure.is_none(), "{}: iteration {iteration}: {:?}", entry.name, report.failure);
        }
    }
}

#[test]
fn every_language_has_a_loadable_seed_set() {
    let root = deep_fuzz::fixtures_root();
    for entry in ENTRIES {
        let seeds = seeds_in(&root, entry).unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        assert!(!seeds.is_empty(), "{} has no seeds", entry.name);
        // The shared hostile corpus is appended to every language.
        assert!(
            seeds.iter().any(|seed| seed.origin.contains("/deep-fuzz/corpus/seeds/")),
            "{} did not load the hostile corpus",
            entry.name
        );
    }
}

#[test]
fn the_fixture_tree_is_the_one_the_registry_names() {
    let root = deep_fuzz::fixtures_root();
    assert!(Path::new(&root).join("python").is_dir(), "{root:?} is not the fixtures root");
}
