//! (a) A valid input builds the expected tree shape, losslessly.

mod common;

use common::*;
use deep_rt::{parse, Config, FLAG_ERROR, FLAG_MISSING, NONE};

#[test]
fn a_valid_input_builds_the_expected_shape() {
    let source = "x = 1 ; def f ( ) : { y = 2 ; }";
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(source));
    let config = Config::new();
    let (cst, stats) = parse(&tables.grammar(), &mut lexer, &config).expect("a tree");

    assert_eq!(cst.errors, 0, "valid input has no error nodes");
    assert_eq!(stats.inserts, 0);
    assert_eq!(stats.skips, 0);
    assert_eq!(stats.cascades, 0);

    let root = cst.nodes[cst.root as usize];
    assert_eq!(root.kind, FILE);
    assert_eq!(root.start, 0);
    assert_eq!(root.end, source.len() as u32, "the root covers the source");
    assert_eq!(root.first_child != NONE, true);

    assert_eq!(
        shape(&cst.nodes, cst.root),
        "(FILE (ITEMS (ITEMS (ITEMS) (ITEM (ASSIGN (NAME) (=) (NUM) (;)))) \
         (ITEM (DEF (DEF) (NAME) (() ()) (:) (BLOCK ({) \
         (ITEMS (ITEMS) (ITEM (ASSIGN (NAME) (=) (NUM) (;)))) (}))))))"
    );

    // Every leaf is a real token, and every token is in the tree exactly once.
    let mut leaves = 0;
    for id in all(&cst.nodes, cst.root) {
        let node = cst.nodes[id as usize];
        assert_eq!(node.flags & (FLAG_ERROR | FLAG_MISSING), 0);
        if node.kind >= DEF && node.kind <= RBRACE {
            leaves += 1;
            assert!(node.start < node.end, "a terminal has width");
        }
    }
    assert_eq!(leaves, lex_mini(source).len());
}

#[test]
fn empty_input_is_a_file_with_no_items() {
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(""));
    let (cst, _) = parse(&tables.grammar(), &mut lexer, &Config::new()).expect("a tree");
    assert_eq!(shape(&cst.nodes, cst.root), "(FILE (ITEMS))");
    assert_eq!(cst.errors, 0);
}

#[test]
fn every_byte_of_the_source_is_inside_the_root() {
    let source = "def f ( ) : { }";
    let tables = mini();
    let mut lexer = SliceLexer::new(lex_mini(source));
    let (cst, _) = parse(&tables.grammar(), &mut lexer, &Config::new()).expect("a tree");
    let root = cst.nodes[cst.root as usize];
    assert_eq!((root.start, root.end), (0, source.len() as u32));
}
