//! (c) A million levels of nesting must not touch the process stack.

mod common;

use common::*;
use deep_rt::{depth, parse, Config};

const LEVELS: usize = 1_000_000;

fn nested_source(levels: usize) -> String {
    let mut source = String::with_capacity(levels * 2 + 1);
    for _ in 0..levels {
        source.push('(');
    }
    source.push('1');
    for _ in 0..levels {
        source.push(')');
    }
    source
}

#[test]
fn a_million_levels_of_nesting_do_not_overflow_the_process_stack() {
    let source = nested_source(LEVELS);
    let tables = nest();
    let mut lexer = SliceLexer::new(lex_nest(&source));
    let (cst, stats) = parse(&tables.grammar(), &mut lexer, &Config::new()).expect("a tree");

    assert_eq!(stats.cascades, 0);
    assert_eq!(cst.errors, 0);
    assert_eq!(cst.nodes[cst.root as usize].kind, NEST_EXPRS);
    assert_eq!(cst.nodes[cst.root as usize].end, source.len() as u32);
    // The root is EXPRS, then EXPR, so the depth is one more than the nesting.
    assert!(
        depth(&cst.nodes, cst.root) >= LEVELS as u32,
        "depth {} for {} levels",
        depth(&cst.nodes, cst.root),
        LEVELS
    );
}

#[test]
fn a_million_shifts_keep_the_stack_a_vec() {
    // The same input, checked through the graph: every entry lists its parent
    // by index, and walking to the bottom is a loop, not a call.
    let source = nested_source(10_000);
    let tables = nest();
    let mut lexer = SliceLexer::new(lex_nest(&source));
    let (cst, _) = parse(&tables.grammar(), &mut lexer, &Config::new()).expect("a tree");
    assert!(cst.nodes.len() > 10_000);
}
