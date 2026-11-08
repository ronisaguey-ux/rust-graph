//! (d) The caps: eight branches per stack entry, ten versions in flight.

mod common;

use common::*;
use deep_rt::version::{Set, Version, MAX_LIVE, MAX_VERSIONS};
use deep_rt::{parse, Config, Gss, MAX_BRANCHES, NONE};

#[test]
fn the_version_set_keeps_six_live_and_four_overflow() {
    let mut set = Set::new();
    for cost in 0..12u32 {
        let mut version = Version::new(Vec::new(), cost as usize);
        version.cost = cost;
        set.push(version);
    }
    assert_eq!(set.len(), MAX_VERSIONS);
    assert_eq!(MAX_VERSIONS, MAX_LIVE + 4);
    assert_eq!(set.drops(), 2, "the two most expensive versions were dropped");
    assert!(set.overflow_events() > 0, "the set went past six");
    assert_eq!(set.peak(), MAX_VERSIONS);
    // The cheapest six plus the four next cheapest survive; 10 and 11 do not.
    let costs: Vec<u32> = set.iter().map(|version| version.cost).collect();
    assert!(!costs.contains(&10) && !costs.contains(&11));
    assert_eq!(costs.len(), 10);
}

#[test]
fn a_stack_entry_keeps_at_most_eight_parents() {
    let mut gss = Gss::new();
    let mut bottoms = Vec::new();
    for _ in 0..10 {
        bottoms.push(gss.push(0, NONE, None));
    }
    let top = gss.push(1, 7, Some(bottoms[0]));
    for &bottom in &bottoms[1..] {
        gss.link_parent(top, bottom);
    }
    assert_eq!(gss.parents(top).len(), MAX_BRANCHES);
    assert_eq!(MAX_BRANCHES, 8);
    assert_eq!(gss.branch_drops(), 2, "the ninth and tenth branch were refused");
}

#[test]
fn a_fork_storm_is_capped_and_reported() {
    // Every bad token offers the parser two readings: insert and skip. Left
    // alone that doubles forever, so the version cap has to bite.
    let mut source = String::from("x = ");
    for _ in 0..24 {
        source.push('?');
    }
    source.push_str(" ; y = 1 ;");

    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(&source));
    let (cst, stats) = parse(&tables.grammar(), &mut lexer, &Config::new()).expect("a tree");

    assert_eq!(cst.nodes[cst.root as usize].kind, FILE);
    assert!(
        stats.peak_versions <= MAX_VERSIONS as u32,
        "peak {} over the cap",
        stats.peak_versions
    );
    assert!(stats.peak_versions > MAX_LIVE as u32, "the overflow was used");
    assert!(stats.version_drops > 0, "drops are counted: {}", stats.version_drops);
    assert!(stats.overflow_events > 0);
    assert!(cst.errors > 0, "the storm is still reported as errors");
}

#[test]
fn a_clean_parse_never_forks() {
    let source = "x = 1 ;";
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(source));
    let (_, stats) = parse(&tables.grammar(), &mut lexer, &Config::new()).expect("a tree");
    assert_eq!(stats.versions, 1);
    assert_eq!(stats.version_drops, 0);
    assert_eq!(stats.branch_drops, 0);
    assert_eq!(stats.peak_versions, 1);
}
