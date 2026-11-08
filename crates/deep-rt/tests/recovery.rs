//! (b) Recovery: an ERROR child inside the function node, the file continues,
//! and a failing balanced group is one cascade instead of a cascade of errors.

mod common;

use common::*;
use deep_rt::{parse, Config, FLAG_ERROR, FLAG_MISSING};

fn error_nodes(nodes: &[deep_rt::Node], root: deep_rt::NodeId) -> Vec<deep_rt::NodeId> {
    all(nodes, root)
        .into_iter()
        .filter(|&id| nodes[id as usize].flags & FLAG_ERROR != 0)
        .collect()
}

#[test]
fn a_bad_def_keeps_the_function_node_and_the_file_going() {
    // `def 1 ( ) : ...` — a number where the name belongs. The parser inserts
    // the name it expected, then skips what it cannot place, and the following
    // assignment still parses.
    let source = "def 1 ( ) : { x = 1 ; } y = 2 ;";
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(source));
    let (cst, stats) = parse(&tables.grammar(), &mut lexer, &Config::new()).expect("a tree");

    assert_eq!(cst.nodes[cst.root as usize].kind, FILE);
    assert!(cst.errors >= 2, "a missing name and an error node");
    assert!(stats.inserts >= 1, "the expected name was inserted");

    // The def node exists, contains an ERROR child, and still has its block.
    let mut defns = Vec::new();
    let mut items = 0;
    for id in all(&cst.nodes, cst.root) {
        if cst.nodes[id as usize].kind == DEFN {
            defns.push(id);
        }
        if cst.nodes[id as usize].kind == ITEM {
            items += 1;
        }
    }
    assert_eq!(defns.len(), 1, "one function node");
    // Two assignments: the one inside the block and the one after the def.
    assert_eq!(items, 3, "the def and both assignments");
    assert!(
        all(&cst.nodes, cst.root)
            .into_iter()
            .any(|id| cst.nodes[id as usize].kind == ASSIGN && cst.nodes[id as usize].start == 24),
        "the assignment after the def parsed: {}",
        shape(&cst.nodes, cst.root)
    );
    let defn = defns[0];
    let kids = children(&cst.nodes, defn);
    assert!(
        kids.iter()
            .any(|&id| cst.nodes[id as usize].flags & FLAG_ERROR != 0),
        "the function node contains an ERROR child: {}",
        shape(&cst.nodes, defn)
    );
    assert!(
        kids.iter()
            .any(|&id| cst.nodes[id as usize].flags & FLAG_MISSING != 0),
        "the function node contains the inserted name"
    );
    assert!(
        kids.iter().any(|&id| cst.nodes[id as usize].kind == BLOCK),
        "the block still parsed"
    );
}

#[test]
fn a_failing_balanced_group_is_one_cascade() {
    // At `= {` the parser wants a number. The group is balanced, so the whole
    // `{ a = 1 ; }` becomes a single ERROR node and `y = 2 ;` still parses.
    let source = "x = { a = 1 ; } 2 ; y = 3 ;";
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(source));
    let pairs = [(LBRACE, RBRACE), (LP, RP)];
    let config = Config::new().with_pairs(&pairs).with_max_inserts(0);
    let (cst, stats) = parse(&tables.grammar(), &mut lexer, &config).expect("a tree");

    assert_eq!(stats.cascades, 1, "one balanced group, one cascade");
    assert!(stats.skips >= 5, "the whole group was skipped at once");
    let errors = error_nodes(&cst.nodes, cst.root);
    assert_eq!(errors.len(), 1);
    let group = cst.nodes[errors[0] as usize];
    assert_eq!((group.start, group.end), (4, 15), "the group's exact bytes");
    assert_eq!(group.kind, deep_rt::ERROR_SYMBOL);

    // The file continued: two items, and the last one is the assignment.
    let items: Vec<_> = all(&cst.nodes, cst.root)
        .into_iter()
        .filter(|&id| cst.nodes[id as usize].kind == ITEM)
        .collect();
    assert_eq!(items.len(), 2, "the assignment after the group parsed");
    let root = cst.nodes[cst.root as usize];
    assert_eq!((root.start, root.end), (0, source.len() as u32));
}

#[test]
fn the_cascade_finds_the_bracket_without_a_pair_table() {
    // No pairs, no grammar knowledge: the driver scans the source bytes.
    let source = "x = { a = 1 ; } 2 ; y = 3 ;";
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(source));
    let config = Config::new().with_source(source.as_bytes()).with_max_inserts(0);
    let (cst, stats) = parse(&tables.grammar(), &mut lexer, &config).expect("a tree");

    assert_eq!(stats.cascades, 1);
    let errors = error_nodes(&cst.nodes, cst.root);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        (cst.nodes[errors[0] as usize].start, cst.nodes[errors[0] as usize].end),
        (4, 15)
    );
}

#[test]
fn a_lone_closing_bracket_is_an_error_but_not_a_cascade() {
    let source = "x = 1 ; } y = 2 ;";
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(source));
    let pairs = [(LBRACE, RBRACE)];
    let config = Config::new().with_pairs(&pairs).with_max_inserts(0);
    let (cst, stats) = parse(&tables.grammar(), &mut lexer, &config).expect("a tree");
    assert_eq!(stats.cascades, 0);
    assert!(stats.skips >= 1);
    assert!(cst.errors >= 1);
    assert_eq!(cst.nodes[cst.root as usize].kind, FILE);
}
