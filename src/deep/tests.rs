//! Slice 1 gates and the runtime's invariants.
//!
//! Gates: every #5910 fixture keeps its `document` root; 1M-deep input survives;
//! the fuzz invariants hold (termination, lossless tokens, nesting ranges, and the
//! cascade test: an edit inside one group leaves its siblings' subtrees unchanged).

use std::time::{Duration, Instant};

use super::budget::{Budget, Exhausted, Outcome};
use super::json::{self, ARRAY, DOCUMENT, JSON, OBJECT};
use super::lexer::{Cursor, Tokens};
use super::pairs::{Pairs, NONE, SEARCH_WINDOW};
use super::parser::{Parser, Workspace};
use super::syntax::{Class, Kind, KindInfo, Language, ERROR, FIRST_LANGUAGE_KIND, SKIPPED, UNKNOWN, WHITESPACE};
use super::tree::{Event, Tree, HAS_ERROR, HAS_SKIP, MISSING, TOKEN, UNCLOSED};

pub(super) fn parse_with(src: &[u8], budget: &Budget) -> (Workspace, Outcome) {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src, &JSON, budget).unwrap();
    check(&workspace, src, &JSON);
    (workspace, outcome)
}

fn parse(src: &str) -> (Workspace, Outcome) {
    parse_with(src.as_bytes(), &Budget::default())
}

fn named(src: &str) -> String {
    parse(src).0.tree.sexp(&JSON, true)
}

/// Deep inputs need more stack than a test thread has in a debug build. Release
/// stack use is measured by the deep-scan binary's `stack` probe, not here.
pub(super) fn on_big_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(256 << 20).spawn(work).unwrap().join().unwrap()
}

/// The invariants every parse must keep, whatever the input.
pub(super) fn check(workspace: &Workspace, src: &[u8], language: &Language) {
    let tokens = &workspace.tokens;
    let nodes = &workspace.tree.nodes;

    // Lossless: tokens tile the input, so their texts concatenate back to it.
    let mut joined = Vec::with_capacity(src.len());
    for index in 0..tokens.len() {
        assert!(tokens.start(index) < tokens.end[index], "empty token {index}");
        joined.extend_from_slice(tokens.text(src, index));
    }
    assert_eq!(joined, src, "tokens are not lossless");

    // The root is the language's root, over the whole input, owning every node.
    assert!(!nodes.is_empty());
    assert_eq!(nodes[0].kind, language.root, "the root collapsed");
    assert_eq!((nodes[0].start, nodes[0].end), (0, src.len() as u32));
    assert_eq!(nodes[0].subtree_end as usize, nodes.len());

    // Ranges nest and are monotonic; subtree ends nest.
    let mut stack: Vec<usize> = Vec::new();
    let mut previous_end_in: Vec<u32> = vec![0];
    for (index, node) in nodes.iter().enumerate() {
        while stack.last().is_some_and(|&open| nodes[open].subtree_end as usize <= index) {
            stack.pop();
            previous_end_in.pop();
        }
        assert!(node.start <= node.end, "node {index} runs backwards");
        assert!(node.subtree_end as usize > index && node.subtree_end as usize <= nodes.len());
        if let Some(&parent) = stack.last() {
            let parent = &nodes[parent];
            assert!(parent.start <= node.start && node.end <= parent.end, "node {index} escapes its parent");
            assert!(node.subtree_end <= parent.subtree_end);
            let previous = previous_end_in.last_mut().unwrap();
            assert!(*previous <= node.start, "node {index} overlaps its previous sibling");
            *previous = node.end;
        }
        stack.push(index);
        previous_end_in.push(node.start);
    }

    // No token is lost: every significant token lies inside exactly one leaf.
    let leaves: Vec<_> = nodes.iter().enumerate().filter(|(index, node)| node.is_leaf(*index as u32) && *index > 0).map(|(_, node)| node).collect();
    let mut leaf = 0;
    for index in 0..tokens.len() {
        if language.is_trivia(tokens.kind[index]) {
            continue;
        }
        let (start, end) = (tokens.start(index), tokens.end[index]);
        while leaf < leaves.len() && leaves[leaf].end <= start {
            leaf += 1;
        }
        assert!(leaf < leaves.len(), "token {index} at {start} is in no leaf");
        assert!(leaves[leaf].start <= start && end <= leaves[leaf].end, "token {index} at {start} is in no leaf");
        if leaves[leaf].flags & TOKEN != 0 {
            assert_eq!(leaves[leaf].kind, tokens.kind[index]);
        }
    }

    // Error and skip flags say exactly what is below.
    for index in (0..nodes.len()).rev() {
        let node = &nodes[index];
        let mut want = 0;
        if node.kind == ERROR || (node.flags & TOKEN != 0 && language.class(node.kind) == Class::BadToken) {
            want |= HAS_ERROR;
        }
        if node.kind == SKIPPED {
            want |= HAS_SKIP;
        }
        let mut child = index + 1;
        while child < node.subtree_end as usize {
            want |= nodes[child].flags & (HAS_ERROR | HAS_SKIP);
            child = nodes[child].subtree_end as usize;
        }
        assert_eq!(node.flags & (HAS_ERROR | HAS_SKIP), want, "flags of node {index}");
    }
}

// ---- The #5910 gate -------------------------------------------------------------

/// Tree-sitter at HEAD and v0.26.13 makes `root=ERROR` of n1, n2 and n5 (measured in
/// Phase 1 with target/ts-baseline/tsdrv). These are the v0.26.12 trees, which kept
/// the root, minus `string_content` and field names.
pub(super) const FIXTURES_5910: &[(&str, &str)] = &[
    (r#"{"a":{"b" "c"}}"#, "(document (object (pair string (object (ERROR string string)))))"),
    (r#"{"one":{"bar" "baz"},"two":"bar"}"#, "(document (object (pair string (object (ERROR string string))) (pair string string)))"),
    (r#"{"b" "c"}"#, "(document (object (ERROR string string)))"),
    ("[1, 2 3, 4]", "(document (array number number (ERROR number) number))"),
    (
        r#"{"k":[{"a":1},{"b" "c"},{"d":2}],"z":{"ok":true}}"#,
        "(document (object (pair string (array (object (pair string number)) (object (ERROR string string)) (object (pair string number)))) (pair string (object (pair string true)))))",
    ),
];

#[test]
fn every_5910_fixture_keeps_its_root_and_confines_the_error() {
    for (src, want) in FIXTURES_5910 {
        let (workspace, outcome) = parse(src);
        assert_eq!(workspace.tree.root().kind, DOCUMENT, "{src}");
        assert_eq!(workspace.tree.sexp(&JSON, true), *want, "{src}");
        assert!(outcome.complete(), "{src}: {outcome:?}");
    }
}

#[test]
fn the_fixtures_match_the_files_phase_1_measured() {
    let probes = concat!(env!("CARGO_MANIFEST_DIR"), "/target/ts-baseline/probes");
    for (number, (src, _)) in FIXTURES_5910.iter().enumerate() {
        let path = format!("{probes}/n{}.json", number + 1);
        // The probes live in the untracked baseline directory; skip quietly without it.
        if let Ok(text) = std::fs::read_to_string(&path) {
            assert_eq!(text.trim_end(), *src, "{path}");
        }
    }
}

#[test]
fn an_error_never_reaches_past_its_group() {
    // The broken member is fenced by its braces: siblings before and after survive.
    // (An unterminated string is not here: it runs to the end of its line by design.)
    for broken in [r#"{"b" "c"}"#, r#"{"b":}"#, r#"{"b" 1 2 3}"#, r#"{:::}"#, r#"{"a":[1 2}"#, r#"{"x": tru}"#, r#"{"b":"c" "d"}"#, "{[}"] {
        let src = format!(r#"[{{"before":1}},{broken},{{"after":2}}]"#);
        let named = named(&src);
        assert!(named.starts_with("(document (array (object (pair string number)) (object "), "{src}: {named}");
        assert!(named.ends_with(" (object (pair string number))))"), "{src}: {named}");
    }
}

#[test]
fn a_closer_of_an_outer_family_ends_the_outer_group_early() {
    // A known limit of one-pass pairing: in `[{]}` the `]` finds the `[` within the
    // search window and pairs with it, taking `{` as the one left open. That is the
    // right call for a missing `}` (`[{]`) and the wrong one for an extra `]` (`{]}`);
    // the pass cannot tell them apart without indentation, which later slices add
    // for languages where it is reliable. The damage stays inside the outer group.
    let src = r#"[{"before":1},{]},{"after":2}]"#;
    assert_eq!(
        named(src),
        "(document (array (object (pair string number)) (object UNCLOSED (ERROR MISSING))) (ERROR) (object (pair string number)) (ERROR))"
    );
}

#[test]
fn valid_json_is_clean_and_complete() {
    let src = r#"{"a": [1, -2.5e+3, true, false, null, {"b": "c\"d"}], "e": {}, "f": []} // done"#;
    let (workspace, outcome) = parse(src);
    assert_eq!(workspace.tree.root().flags & HAS_ERROR, 0, "{}", workspace.tree.sexp(&JSON, false));
    assert!(outcome.complete());
    assert_eq!(
        workspace.tree.sexp(&JSON, true),
        "(document (object (pair string (array number number true false null (object (pair string string)))) (pair string (object)) (pair string (array))))"
    );
}

#[test]
fn invalid_json_always_yields_an_error() {
    // M5: a permissive grammar would pass these.
    for src in ["{,}", "[1,]", r#"{"a":1,}"#, "[1 2]", r#"{"a" 1}"#, r#"{"a":}"#, "[", "]", "{", r#""open"#, "nul", "-", "@", "[1,,2]", r#"{"a":1 "b":2}"#] {
        let (workspace, _) = parse(src);
        assert_ne!(workspace.tree.root().flags & HAS_ERROR, 0, "{src}: {}", workspace.tree.sexp(&JSON, false));
    }
}

#[test]
fn a_missing_value_is_an_empty_error_flagged_missing() {
    let (workspace, _) = parse(r#"{"a":}"#);
    let tree = &workspace.tree;
    assert_eq!(tree.sexp(&JSON, true), "(document (object (pair string (ERROR MISSING))))");
    let missing = tree.nodes.iter().find(|node| node.flags & MISSING != 0).unwrap();
    assert_eq!((missing.start, missing.end), (5, 5));
}

#[test]
fn an_unclosed_group_runs_to_its_parents_end_and_says_so() {
    let (workspace, outcome) = parse(r#"[{"a":1, [2]"#);
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (array UNCLOSED (object UNCLOSED (pair string number) (ERROR number) (ERROR MISSING)) (ERROR MISSING)))");
    assert_eq!(outcome.unclosed, 2);
    let (workspace, _) = parse(r#"[{"a":1]"#);
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (array (object UNCLOSED (pair string number) (ERROR MISSING))))");
}

#[test]
fn a_stray_closer_is_an_error_where_it_stands() {
    let (workspace, outcome) = parse(r#"{"a":1}}]"#);
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (object (pair string number)) (ERROR))");
    assert_eq!(outcome.stray, 2);
}

#[test]
fn an_unterminated_string_stops_at_its_line() {
    // The `,` and the `2` are not swallowed by the string, and `]` still closes.
    let (workspace, _) = parse("[\"open,\n 2\n]");
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (array (ERROR bad_string number)))");
}

#[test]
fn trivia_and_unknown_bytes_are_kept() {
    let src = "/* c */ [1 /* x */, @@ 2] // end\n";
    let (workspace, _) = parse(src);
    let kinds: Vec<Kind> = workspace.tokens.kind.clone();
    assert!(kinds.contains(&super::syntax::COMMENT));
    assert_eq!(kinds.iter().filter(|kind| **kind == UNKNOWN).count(), 1, "one token per unknown run");
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (array number (ERROR UNKNOWN number)))");
}

// ---- The depth gate -------------------------------------------------------------

pub(super) const MILLION: usize = 1_000_000;

#[test]
fn a_million_deep_array_survives_and_is_skipped_past_the_depth_budget() {
    let (outcome, depth, nodes, elapsed) = on_big_stack(|| {
        let src = format!("{}{}\n", "[".repeat(MILLION), "]".repeat(MILLION));
        let start = Instant::now();
        let (workspace, outcome) = parse_with(src.as_bytes(), &Budget::default());
        let elapsed = start.elapsed();
        let tree = &workspace.tree;
        assert_eq!(tree.root().kind, DOCUMENT);
        let mut json = String::new();
        tree.write_json(&JSON, src.as_bytes(), &mut json);
        assert!(json.starts_with(r#"{"kind":"document","start":0,"end":2000001,"flags":["skipped"],"children":[{"kind":"array""#));
        (outcome, tree.depth(), tree.len(), elapsed)
    });
    let max = Budget::default().max_depth;
    assert_eq!(outcome.deepest, max);
    assert_eq!(outcome.skipped_groups, 1);
    assert_eq!(outcome.exhausted, None);
    // document > max arrays > one SKIPPED leaf; each array also holds its `[` and `]`.
    assert_eq!(depth, max + 1);
    assert_eq!(nodes as u32, 1 + 3 * max + 1);
    eprintln!("1M-deep balanced: {elapsed:?}, {outcome:?}");
}

#[test]
fn a_million_unclosed_openers_survive() {
    let (outcome, sexp_head) = on_big_stack(|| {
        let src = "[".repeat(MILLION);
        let (workspace, outcome) = parse_with(src.as_bytes(), &Budget::default());
        let sexp = workspace.tree.sexp(&JSON, true);
        (outcome, sexp[..41].to_string())
    });
    assert_eq!(outcome.unclosed, MILLION as u32);
    assert_eq!(outcome.skipped_groups, 1);
    assert_eq!(sexp_head, "(document (array UNCLOSED (array UNCLOSED");
}

#[test]
fn a_million_stray_closers_survive_in_linear_time() {
    let src = "]".repeat(MILLION);
    let start = Instant::now();
    let (workspace, outcome) = parse(&src);
    assert!(start.elapsed() < Duration::from_secs(20));
    assert_eq!(outcome.stray, MILLION as u32);
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (ERROR))");
}

#[test]
fn a_million_deep_object_chain_survives() {
    let outcome = on_big_stack(|| {
        let src = format!("{}1{}", r#"{"a":"#.repeat(MILLION), "}".repeat(MILLION));
        parse_with(src.as_bytes(), &Budget::default()).1
    });
    assert_eq!(outcome.skipped_groups, 1);
    assert!(outcome.exhausted.is_none());
}

#[test]
fn every_tree_walk_handles_a_million_deep_tree() {
    // Built straight from events: the tree builder and every walk are loops.
    let mut events = vec![Event::Start { kind: DOCUMENT, forward_parent: 0 }];
    events.extend(std::iter::repeat_n(Event::Start { kind: ARRAY, forward_parent: 0 }, MILLION));
    events.extend(std::iter::repeat_n(Event::Finish, MILLION + 1));
    let mut tree = Tree::default();
    tree.build(&mut events, &Tokens::default(), &JSON, 0);
    assert_eq!(tree.len(), MILLION + 1);
    assert_eq!(tree.depth(), MILLION as u32);
    let sexp = tree.sexp(&JSON, true);
    assert_eq!(sexp.len(), "(document)".len() + MILLION * "(array)".len() + MILLION);
    let mut json = String::new();
    tree.write_json(&JSON, b"", &mut json);
    // The innermost array is a leaf; the root and every other array close a list.
    assert!(json.ends_with(&format!("{{\"kind\":\"array\",\"start\":0,\"end\":0}}{}", "]}".repeat(MILLION))));
    let mut at = 0;
    let mut hops = 0;
    while let Some(child) = tree.children(at).next() {
        at = child;
        hops += 1;
    }
    assert_eq!(hops, MILLION);
}

// ---- Budgets --------------------------------------------------------------------

pub(super) fn big_document(objects: usize) -> String {
    let mut src = String::from("[\n");
    for index in 0..objects {
        if index > 0 {
            src.push_str(",\n");
        }
        src.push_str(&format!(r#"{{"id": {index}, "name": "item {index}", "tags": ["a", "b", null], "ok": true, "nested": {{"x": -1.5e3, "y": [1, 2, {{"z": false}}]}}}}"#));
    }
    src.push_str("\n]\n");
    src
}

#[test]
fn the_step_budget_degrades_the_file_and_keeps_the_tree_valid() {
    let src = big_document(200);
    let tight = Budget { steps_per_token: 0, base_steps: 500, ..Budget::default() };
    let (workspace, outcome) = parse_with(src.as_bytes(), &tight);
    assert_eq!(outcome.exhausted, Some(Exhausted::Steps));
    assert!(outcome.steps <= 501);
    assert_eq!(workspace.tree.root().kind, DOCUMENT);
    assert_ne!(workspace.tree.root().flags & HAS_ERROR, 0);
}

#[test]
fn the_time_budget_degrades_the_file() {
    let src = big_document(50);
    let (workspace, outcome) = parse_with(src.as_bytes(), &Budget { time: Some(Duration::ZERO), ..Budget::default() });
    assert_eq!(outcome.exhausted, Some(Exhausted::Time));
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (ERROR))");
}

#[test]
fn a_correct_parse_uses_a_small_constant_of_steps_per_token() {
    let src = big_document(500);
    let (_, outcome) = parse(&src);
    assert!(outcome.complete());
    let per_token = outcome.steps as f64 / outcome.tokens as f64;
    assert!(per_token < 4.0, "{per_token} steps per token");
}

#[test]
fn a_smaller_depth_budget_skips_sooner() {
    let (workspace, outcome) = parse_with(b"[[[[1]]], 2]", &Budget { max_depth: 2, ..Budget::default() });
    assert_eq!(workspace.tree.sexp(&JSON, true), "(document (array (array (SKIPPED)) number))");
    assert_eq!(outcome.skipped_groups, 1);
}

// ---- Arena reuse ----------------------------------------------------------------

#[test]
fn a_reused_workspace_does_not_allocate_again() {
    let src = big_document(300);
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), &JSON, &Budget::default()).unwrap();
    let before = (workspace.tokens.kind.as_ptr(), workspace.tokens.end.as_ptr(), workspace.pairs.pair.as_ptr(), workspace.events.as_ptr(), workspace.tree.nodes.as_ptr());
    let first = workspace.tree.sexp(&JSON, false);
    workspace.parse(br#"{"small": 1}"#, &JSON, &Budget::default()).unwrap();
    workspace.parse(src.as_bytes(), &JSON, &Budget::default()).unwrap();
    let after = (workspace.tokens.kind.as_ptr(), workspace.tokens.end.as_ptr(), workspace.pairs.pair.as_ptr(), workspace.events.as_ptr(), workspace.tree.nodes.as_ptr());
    assert_eq!(before, after);
    assert_eq!(first, workspace.tree.sexp(&JSON, false));
}

// ---- Fuzz invariants ------------------------------------------------------------

/// xorshift64*: deterministic, std only.
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub(super) fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn pick(&mut self, alphabet: &[u8]) -> u8 {
        alphabet[self.below(alphabet.len())]
    }
}

pub(super) const ANY_JSON_BYTE: &[u8] = b"{}[]:,\"\\0123456789 \n\t-+.eEtrufalsn/*x@\xc3\xa9\xff";

/// One random edit within `from..to`. Bytes in `keep` are never deleted or
/// overwritten, so an edit can be made delimiter-neutral.
pub(super) fn mutate(rng: &mut Rng, src: &mut Vec<u8>, alphabet: &[u8], keep: &[u8], from: usize, to: usize) {
    let span = to - from;
    match rng.below(4) {
        0 => {
            let at = from + rng.below(span + 1);
            src.insert(at, rng.pick(alphabet));
        }
        1 if span > 0 => {
            let at = from + rng.below(span);
            let length = 1 + rng.below((to - at).min(8));
            let mut cut = at;
            for _ in 0..length {
                if cut >= src.len() {
                    break;
                }
                if keep.contains(&src[cut]) {
                    cut += 1;
                } else {
                    src.remove(cut);
                }
            }
        }
        2 if span > 0 => {
            let at = from + rng.below(span);
            if !keep.contains(&src[at]) {
                src[at] = rng.pick(alphabet);
            }
        }
        _ => {
            let at = from + rng.below(span + 1);
            let count = 1 + rng.below(4);
            for _ in 0..count {
                src.insert(at, rng.pick(alphabet));
            }
        }
    }
}

#[test]
fn fuzzed_input_terminates_and_keeps_every_invariant() {
    let mut seeds: Vec<Vec<u8>> = FIXTURES_5910.iter().map(|(src, _)| src.as_bytes().to_vec()).collect();
    seeds.push(big_document(3).into_bytes());
    seeds.push(br#"{"a": [1, 2, {"b": null}], "c": "d"} // x"#.to_vec());
    seeds.push(b"[[[[[[]]]]]]".to_vec());
    let mut rng = Rng(0x5910_5910_dead_beef);
    let mut workspace = Workspace::default();
    let budget = Budget::default();
    let mut erroneous = 0;
    const ROUNDS: usize = 4000;
    for round in 0..ROUNDS {
        let mut src = seeds[round % seeds.len()].clone();
        for _ in 0..1 + rng.below(6) {
            let length = src.len();
            mutate(&mut rng, &mut src, ANY_JSON_BYTE, b"", 0, length);
        }
        let outcome = workspace.parse(&src, &JSON, &budget).unwrap();
        assert!(outcome.exhausted.is_none(), "budget ran out on {:?}", String::from_utf8_lossy(&src));
        check(&workspace, &src, &JSON);
        if workspace.tree.root().flags & HAS_ERROR != 0 {
            erroneous += 1;
        }
    }
    // M5 in the small: most random edits must be reported as errors.
    assert!(erroneous > ROUNDS / 2, "{erroneous} of {ROUNDS}");
}

/// Delimiter-neutral edits: no brackets, no `/` (an unclosed `/*` comment runs to
/// the end of the file by design, as in Tree-sitter), and no quote or backslash,
/// which could hide a delimiter inside a string. Those edits change the pairing
/// itself; the general fuzz test above covers them.
const INSIDE_GROUP_BYTE: &[u8] = b":,0123456789 \t-+.eEtrufalsnx@";
const DELIMITING_BYTES: &[u8] = b"{}[]\"\\\n";

/// (kind, flags, start, end, size) of each node of a subtree, positions relative to it.
fn shape(tree: &Tree, root: u32) -> Vec<(Kind, u16, u32, u32, u32)> {
    let base = tree.nodes[root as usize].start;
    (root..tree.nodes[root as usize].subtree_end)
        .map(|index| {
            let node = tree.nodes[index as usize];
            (node.kind, node.flags, node.start - base, node.end - base, node.subtree_end - index)
        })
        .collect()
}

fn elements(tree: &Tree) -> Vec<u32> {
    let array = tree.children(0).find(|child| tree.nodes[*child as usize].kind == ARRAY).unwrap();
    tree.children(array).collect()
}

#[test]
fn an_edit_inside_one_group_leaves_every_sibling_subtree_unchanged() {
    let mut rng = Rng(0xcafe_f00d_0000_5910);
    let mut workspace = Workspace::default();
    let budget = Budget::default();
    for round in 0..1500 {
        let objects = 3 + rng.below(5);
        let mut src = String::from("[\n");
        for index in 0..objects {
            src.push_str(&format!("{{\"id\": {index}, \"v\": [1, {{\"w\": \"s{round}\"}}], \"t\": true}},"));
        }
        src.push_str("{}]");
        // One token per line.
        let src = String::from_utf8(src.into_bytes()).unwrap().replace(", ", ",").replace(": ", ":");
        let src: Vec<u8> = src.bytes().flat_map(|byte| match byte {
            b'{' | b'}' | b'[' | b']' | b',' | b':' => vec![b'\n', byte, b'\n'],
            _ => vec![byte],
        }).collect();
        workspace.parse(&src, &JSON, &budget).unwrap();
        let tree = &workspace.tree;
        let before_elements = elements(tree);
        let before: Vec<_> = before_elements.iter().map(|element| shape(tree, *element)).collect();
        // Pick one object and edit strictly between its braces.
        let objects_at: Vec<usize> = before_elements.iter().enumerate().filter(|(_, e)| tree.nodes[**e as usize].kind == OBJECT).map(|(at, _)| at).collect();
        let target = objects_at[rng.below(objects_at.len() - 1)];
        let node = tree.nodes[before_elements[target] as usize];
        let (from, to) = (node.start as usize + 1, node.end as usize - 1);
        let mut edited = src.clone();
        let mut to_now = to;
        for _ in 0..1 + rng.below(5) {
            let length = edited.len();
            mutate(&mut rng, &mut edited, INSIDE_GROUP_BYTE, DELIMITING_BYTES, from, to_now);
            to_now = (to_now + edited.len()).saturating_sub(length).max(from);
        }
        workspace.parse(&edited, &JSON, &budget).unwrap();
        check(&workspace, &edited, &JSON);
        let tree = &workspace.tree;
        let after_elements = elements(tree);
        assert_eq!(after_elements.len(), before.len(), "the array lost or gained a member: {}", String::from_utf8_lossy(&edited));
        for (at, element) in after_elements.iter().enumerate() {
            if at != target {
                assert_eq!(shape(tree, *element), before[at], "sibling {at} changed after an edit in {target}: {}", String::from_utf8_lossy(&edited));
            }
        }
    }
}

// ---- The delimiter pass ---------------------------------------------------------

/// A test language for pairing: `{}` and `()`, `i`/`e`/`n` as `#if`/`#else`/`#endif`.
mod pp {
    use super::*;
    pub const LB: Kind = FIRST_LANGUAGE_KIND;
    pub const RB: Kind = LB + 1;
    pub const LP: Kind = LB + 2;
    pub const RP: Kind = LB + 3;
    pub const IF: Kind = LB + 4;
    pub const ELSE: Kind = LB + 5;
    pub const END: Kind = LB + 6;
    pub const X: Kind = LB + 7;
    pub const ROOT: Kind = LB + 8;
    const KINDS: &[KindInfo] = &[
        KindInfo { name: "{", class: Class::Open(0) },
        KindInfo { name: "}", class: Class::Close(0) },
        KindInfo { name: "(", class: Class::Open(1) },
        KindInfo { name: ")", class: Class::Close(1) },
        KindInfo { name: "#if", class: Class::BranchIf },
        KindInfo { name: "#else", class: Class::BranchElse },
        KindInfo { name: "#endif", class: Class::BranchEnd },
        KindInfo { name: "x", class: Class::Token },
        KindInfo { name: "root", class: Class::Node },
    ];
    fn lex(cursor: &mut Cursor) -> Kind {
        let kind = match cursor.peek() {
            b'{' => LB,
            b'}' => RB,
            b'(' => LP,
            b')' => RP,
            b'i' => IF,
            b'e' => ELSE,
            b'n' => END,
            b'x' => X,
            b' ' => WHITESPACE,
            _ => return UNKNOWN,
        };
        cursor.bump();
        kind
    }
    fn parse(p: &mut Parser) {
        while !p.at_end() {
            p.bump();
        }
    }
    pub static PP: Language = Language::new("pp", ROOT, KINDS, lex, parse);
}

/// Pairs over the non-space characters of `src`, as (opener at, closer at) char indices.
fn pairs_of(src: &str) -> (Vec<(usize, usize)>, Pairs) {
    let mut tokens = Tokens::default();
    super::lexer::lex(src.as_bytes(), &pp::PP, &mut tokens);
    let mut pairs = Pairs::default();
    pairs.build(&tokens, &pp::PP);
    check_pairs(&pairs, &tokens);
    let found = (0..tokens.len())
        .filter(|&index| matches!(pp::PP.class(tokens.kind[index]), Class::Open(_)) && pairs.pair[index] != NONE)
        .map(|index| (tokens.start(index) as usize, tokens.start(pairs.pair[index] as usize) as usize))
        .collect();
    (found, pairs)
}

/// `pair` is symmetric, pairs an opener with a later closer of its family, and nests.
fn check_pairs(pairs: &Pairs, tokens: &Tokens) {
    let mut stack: Vec<u32> = Vec::new();
    for index in 0..tokens.len() as u32 {
        let partner = pairs.pair[index as usize];
        if partner == NONE {
            continue;
        }
        assert_eq!(pairs.pair[partner as usize], index, "pair is not symmetric at {index}");
        let (open, close) = (index.min(partner), index.max(partner));
        match (pp::PP.class(tokens.kind[open as usize]), pp::PP.class(tokens.kind[close as usize])) {
            (Class::Open(a), Class::Close(b)) => assert_eq!(a, b),
            other => panic!("{other:?} paired"),
        }
        if index == open {
            while stack.last().is_some_and(|&top| pairs.pair[top as usize] < index) {
                stack.pop();
            }
            if let Some(&top) = stack.last() {
                assert!(close < pairs.pair[top as usize], "pairs cross at {index}");
            }
            stack.push(index);
        }
    }
}

#[test]
fn a_closer_closes_the_openers_above_its_partner() {
    let (found, pairs) = pairs_of("{(}");
    assert_eq!(found, [(0, 2)]);
    assert_eq!((pairs.unclosed, pairs.stray), (1, 0));
}

#[test]
fn a_closer_with_no_partner_in_reach_is_stray() {
    let (found, pairs) = pairs_of("{)}");
    assert_eq!(found, [(0, 2)]);
    assert_eq!(pairs.stray, 1);
    let deep = format!("{{{}}}", "(".repeat(SEARCH_WINDOW));
    let (found, pairs) = pairs_of(&deep);
    assert!(found.is_empty(), "the brace is below the search window");
    assert_eq!((pairs.stray, pairs.unclosed), (1, SEARCH_WINDOW as u32 + 1));
}

#[test]
fn braces_split_across_branches_still_pair() {
    // drm.h: `#ifdef __cplusplus / extern "C" { / #endif ... #ifdef __cplusplus / } / #endif`
    let (found, pairs) = pairs_of("i{n x i}n");
    assert_eq!(found, [(1, 7)]);
    assert_eq!((pairs.unclosed, pairs.stray), (0, 0));
}

#[test]
fn each_branch_starts_from_the_stack_at_its_if() {
    // `f(a, #if X b) #else c) #endif`: the later closer wins, the earlier is stray.
    let (found, pairs) = pairs_of("(ix)ex)n");
    assert_eq!(found, [(0, 6)]);
    assert_eq!((pairs.unclosed, pairs.stray), (0, 1));
    // `#if X f( #else g( #endif x)`: the opener abandoned by `#else` stays unclosed.
    let (found, pairs) = pairs_of("i(e(n)");
    assert_eq!(found, [(3, 5)]);
    assert_eq!((pairs.unclosed, pairs.stray), (1, 0));
}

#[test]
fn random_pairing_input_keeps_pair_symmetric_and_nested() {
    let mut rng = Rng(0x0dd_ba11);
    for _ in 0..3000 {
        let length = rng.below(60);
        let src: String = (0..length).map(|_| rng.pick(b"{}(){}()ienx ") as char).collect();
        pairs_of(&src);
    }
}

#[test]
fn the_delimiter_pass_is_linear_on_hostile_input() {
    let src = format!("{}{}", "(".repeat(MILLION / 2), "}".repeat(MILLION / 2));
    let start = Instant::now();
    let (found, pairs) = pairs_of(&src);
    assert!(found.is_empty());
    assert_eq!(pairs.stray, MILLION as u32 / 2);
    assert!(start.elapsed() < Duration::from_secs(20));
    let many_ifs = format!("{}{}", "{".repeat(2000), "ixen".repeat(100_000));
    let start = Instant::now();
    pairs_of(&many_ifs);
    assert!(start.elapsed() < Duration::from_secs(20), "snapshots over the depth cap are skipped");
}

// ---- Events: precede, abandon, rewind -------------------------------------------

/// A test language with a Pratt loop: numbers, `+`, `*`, prefix `-`, parentheses.
mod calc {
    use super::*;
    pub const NUM: Kind = FIRST_LANGUAGE_KIND;
    pub const PLUS: Kind = NUM + 1;
    pub const STAR: Kind = NUM + 2;
    pub const MINUS: Kind = NUM + 3;
    pub const LP: Kind = NUM + 4;
    pub const RP: Kind = NUM + 5;
    pub const ROOT: Kind = NUM + 6;
    pub const LIT: Kind = NUM + 7;
    pub const BIN: Kind = NUM + 8;
    pub const NEG: Kind = NUM + 9;
    pub const PAREN: Kind = NUM + 10;
    const KINDS: &[KindInfo] = &[
        KindInfo { name: "num", class: Class::Token },
        KindInfo { name: "+", class: Class::Token },
        KindInfo { name: "*", class: Class::Token },
        KindInfo { name: "-", class: Class::Token },
        KindInfo { name: "(", class: Class::Open(0) },
        KindInfo { name: ")", class: Class::Close(0) },
        KindInfo { name: "root", class: Class::Node },
        KindInfo { name: "lit", class: Class::Node },
        KindInfo { name: "bin", class: Class::Node },
        KindInfo { name: "neg", class: Class::Node },
        KindInfo { name: "paren", class: Class::Node },
    ];
    fn lex(cursor: &mut Cursor) -> Kind {
        let kind = match cursor.peek() {
            b'0'..=b'9' => {
                cursor.eat_while(|byte| byte.is_ascii_digit());
                return NUM;
            }
            b'+' => PLUS,
            b'*' => STAR,
            b'-' => MINUS,
            b'(' => LP,
            b')' => RP,
            b' ' => WHITESPACE,
            _ => return UNKNOWN,
        };
        cursor.bump();
        kind
    }

    pub fn expr(p: &mut Parser, min: u8) -> Option<super::super::parser::Closed> {
        let mut left = match p.current() {
            NUM => {
                let marker = p.open();
                p.bump();
                p.close(marker, LIT)
            }
            LP => p.group(PAREN, |p| {
                expr(p, 0);
            })?,
            MINUS => {
                let marker = p.open();
                p.bump();
                p.nested(|p| expr(p, 5));
                p.close(marker, NEG)
            }
            _ => return None,
        };
        loop {
            let (left_power, right_power) = match p.current() {
                PLUS => (1, 2),
                STAR => (3, 4),
                _ => break,
            };
            if left_power < min {
                break;
            }
            let marker = p.precede(left);
            p.bump();
            if expr(p, right_power).is_none() {
                p.missing();
            }
            left = p.close(marker, BIN);
        }
        Some(left)
    }

    fn parse(p: &mut Parser) {
        while !p.at_end() {
            if expr(p, 0).is_none() {
                p.error_one();
            }
        }
    }

    pub static CALC: Language = Language::new("calc", ROOT, KINDS, lex, parse);
}

fn calc(src: &str) -> (Workspace, Outcome) {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src.as_bytes(), &calc::CALC, &Budget::default()).unwrap();
    check(&workspace, src.as_bytes(), &calc::CALC);
    (workspace, outcome)
}

#[test]
fn precede_wraps_a_finished_node_without_moving_events() {
    assert_eq!(calc("1 + 2 * 3").0.tree.sexp(&calc::CALC, true), "(root (bin (lit num) (bin (lit num) (lit num))))");
    assert_eq!(calc("1 * 2 + 3").0.tree.sexp(&calc::CALC, true), "(root (bin (bin (lit num) (lit num)) (lit num)))");
    assert_eq!(calc("-(1 + 2) * 3").0.tree.sexp(&calc::CALC, true), "(root (bin (neg (paren (bin (lit num) (lit num)))) (lit num)))");
}

#[test]
fn a_long_left_associative_chain_is_a_loop_not_recursion() {
    // 1+1+...: 200k nested `bin`s on the left, from one chain of forward parents.
    let count = 200_000;
    let src = format!("1{}", "+1".repeat(count));
    let (workspace, outcome) = calc(&src);
    assert!(outcome.complete());
    assert_eq!(workspace.tree.depth(), count as u32 + 2);
}

#[test]
fn deep_prefix_recursion_is_cut_by_the_depth_budget() {
    let (workspace, outcome) = on_big_stack(|| calc(&format!("{}1", "-".repeat(MILLION))));
    assert_eq!(outcome.skipped_groups, 1);
    assert_eq!(workspace.tree.depth(), Budget::default().max_depth + 2);
}

#[test]
fn abandon_and_rewind_leave_no_trace() {
    fn rule(p: &mut Parser) {
        // An abandoned marker at the end, and one with events after it.
        let marker = p.open();
        p.abandon(marker);
        let outer = p.open();
        let lit = p.open();
        p.bump();
        let lit = p.close(lit, calc::LIT);
        // Speculate: wrap the literal, consume `+`, then give it all back.
        let checkpoint = p.checkpoint();
        let wrap = p.precede(lit);
        p.bump();
        p.close(wrap, calc::BIN);
        p.rewind(checkpoint);
        p.abandon(outer);
        while !p.at_end() {
            p.bump();
        }
    }
    static SPECULATE: Language = Language::new("speculate", calc::ROOT, calc::CALC.kinds, calc::CALC.lex, rule);
    let mut workspace = Workspace::default();
    workspace.parse(b"1 + 2", &SPECULATE, &Budget::default()).unwrap();
    check(&workspace, b"1 + 2", &SPECULATE);
    assert_eq!(workspace.tree.sexp(&SPECULATE, false), "(root (lit num) + num)");
}

#[test]
fn json_kinds_are_listed_in_order() {
    // The kind constants index the table; a slip would rename every node.
    for (kind, name) in [(json::LBRACE, "{"), (json::STRING, "string"), (json::WORD, "word"), (DOCUMENT, "document"), (json::PAIR, "pair")] {
        assert_eq!(JSON.name(kind), name);
    }
    assert_eq!(JSON.kinds.len() as Kind, json::PAIR - FIRST_LANGUAGE_KIND + 1);
}

#[test]
fn flags_name_what_happened() {
    let (workspace, _) = parse("[");
    assert_ne!(workspace.tree.nodes[1].flags & UNCLOSED, 0);
    assert_ne!(workspace.tree.nodes[1].flags & HAS_ERROR, 0, "an unclosed group is an error");
}

// ---- grammar! end to end --------------------------------------------------------

/// A small language through `grammar!`. Prefix `-` recurses outside any group, so
/// the macro sends that call through the depth budget.
mod prefix {
    use super::*;

    rust_graph_deep::grammar! { PREFIX {
        lex = lex;
        tokens {
            MINUS = "-",
            NUM = "num",
            COMMA = ",",
            LP = "(" open parens,
            RP = ")" close parens,
        }
        items: _item*,
        _item: neg | NUM | tuple,
        neg: "-" _item,
        tuple: "(" (_item % ",") ")",
    } }

    fn lex(cursor: &mut Cursor) -> Kind {
        let kind = match cursor.peek() {
            b'0'..=b'9' => {
                cursor.eat_while(|byte| byte.is_ascii_digit());
                return NUM;
            }
            b'-' => MINUS,
            b',' => COMMA,
            b'(' => LP,
            b')' => RP,
            b' ' => WHITESPACE,
            _ => return UNKNOWN,
        };
        cursor.bump();
        kind
    }
}

fn prefix(src: &str) -> (Workspace, Outcome) {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src.as_bytes(), &prefix::PREFIX, &Budget::default()).unwrap();
    check(&workspace, src.as_bytes(), &prefix::PREFIX);
    (workspace, outcome)
}

#[test]
fn a_macro_grammar_defines_its_kinds_in_order() {
    use prefix::*;
    assert_eq!((PREFIX.name, PREFIX.root), ("prefix", ITEMS));
    assert_eq!([MINUS, NUM, COMMA, LP, RP, ITEMS, NEG, TUPLE], [16, 17, 18, 19, 20, 21, 22, 23]);
    let rows: Vec<(&str, Class)> = PREFIX.kinds.iter().map(|info| (info.name, info.class)).collect();
    assert_eq!(rows[3..], [("(", Class::Open(0)), (")", Class::Close(0)), ("items", Class::Node), ("neg", Class::Node), ("tuple", Class::Node)]);
}

#[test]
fn a_macro_grammar_parses_and_recovers() {
    let sexp = |src: &str| prefix(src).0.tree.sexp(&prefix::PREFIX, true);
    assert_eq!(sexp("-1 (2, -(3)) 4"), "(items (neg num) (tuple num (neg (tuple num))) num)");
    // A required item that is absent is MISSING; an extra item is an ERROR.
    assert_eq!(sexp("-"), "(items (neg (ERROR MISSING)))");
    assert_eq!(sexp("(1 2, 3)"), "(items (tuple num (ERROR num) num))");
    // The firewall holds for generated groups too.
    assert_eq!(sexp("(1, (2 -), 3) 4"), "(items (tuple num (tuple num (ERROR)) num) num)");
    assert_eq!(sexp("(1, (2, -), 3) 4"), "(items (tuple num (tuple num (neg (ERROR MISSING))) num) num)");
}

#[test]
fn macro_recursion_outside_a_group_is_cut_by_the_depth_budget() {
    let (outcome, sexp_head) = on_big_stack(|| {
        let src = format!("{}1", "-".repeat(MILLION));
        let (workspace, outcome) = prefix(&src);
        (outcome, workspace.tree.sexp(&prefix::PREFIX, true)[..21].to_string())
    });
    assert_eq!(outcome.deepest, Budget::default().max_depth);
    assert_eq!(outcome.skipped_groups, 1);
    assert_eq!(sexp_head, "(items (neg (neg (neg");
}
