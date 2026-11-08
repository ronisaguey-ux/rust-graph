//! Conflict detection and precedence resolution.

mod common;

use common::{parse, productions};
use deep_tables::{grammars, Associativity, Conflict, GrammarBuilder};

fn ambiguous() -> GrammarBuilder {
    GrammarBuilder::new()
        .name("ambiguous")
        .terminal("NUM")
        .terminal("PLUS")
        .terminal("LT")
        .rule("s", &["s", "PLUS", "s"])
        .rule("s", &["NUM"])
        .rule("s", &["s", "LT", "s"])
}

#[test]
fn reports_a_shift_reduce_conflict_with_state_symbol_and_actions() {
    let err = ambiguous().build().expect_err("ambiguous without precedence");
    match err {
        Conflict::ShiftReduce { state, terminal, terminal_name, shift_to, reduce_by } => {
            assert_eq!(terminal_name, "PLUS", "the first clash is on PLUS");
            assert_eq!(terminal, 2, "NUM is 1, PLUS is 2, LT is 3");
            assert!(shift_to > 0, "the shift goes somewhere");
            assert_eq!(reduce_by, 1, "the clashing production is s -> s PLUS s");
            assert!(state > 0, "the clash is not in the start state");
        }
        other => panic!("expected a shift/reduce conflict, got {other:?}"),
    }
}

#[test]
fn reports_a_reduce_reduce_conflict() {
    let err = GrammarBuilder::new()
        .name("rr")
        .terminal("NUM")
        .rule("s", &["a"])
        .rule("s", &["b"])
        .rule("a", &["NUM"])
        .rule("b", &["NUM"])
        .build()
        .expect_err("NUM reduces to both a and b");
    match err {
        Conflict::ReduceReduce { state, terminal, terminal_name, rule_a, rule_b } => {
            assert_eq!(terminal_name, "$", "the clash shows up at end of input");
            assert_eq!(terminal, 0);
            assert_eq!((rule_a, rule_b), (3, 4), "the two competing productions");
            assert!(state > 0);
        }
        other => panic!("expected a reduce/reduce conflict, got {other:?}"),
    }
}

#[test]
fn precedence_resolves_the_conflict() {
    let g = ambiguous()
        .precedence(1, Associativity::Left, &["PLUS"])
        .precedence(2, Associativity::Left, &["LT"])
        .build()
        .expect("precedence resolves both operators");
    let grammar = g.as_grammar();
    let prods = productions(&g);
    let num = g.symbol("NUM").unwrap();
    let plus = g.symbol("PLUS").unwrap();
    assert!(parse(&grammar, &prods, &[num, plus, num, plus, num]).is_some());
}

#[test]
fn non_associativity_makes_chaining_an_error() {
    let g = GrammarBuilder::new()
        .name("cmp")
        .terminal("NUM")
        .terminal("LT")
        .precedence(1, Associativity::NonAssoc, &["LT"])
        .rule("c", &["c", "LT", "c"])
        .rule("c", &["NUM"])
        .build()
        .expect("non-assoc resolves the conflict to an error entry");
    let grammar = g.as_grammar();
    let prods = productions(&g);
    let num = g.symbol("NUM").unwrap();
    let lt = g.symbol("LT").unwrap();
    assert!(parse(&grammar, &prods, &[num, lt, num]).is_some(), "a single comparison parses");
    assert!(
        parse(&grammar, &prods, &[num, lt, num, lt, num]).is_none(),
        "a chained comparison is an error"
    );
}

#[test]
fn precedence_levels_decide_shift_versus_reduce() {
    let g = GrammarBuilder::new()
        .name("prec")
        .terminal("NUM")
        .terminal("PLUS")
        .terminal("STAR")
        .precedence(1, Associativity::Left, &["PLUS"])
        .precedence(2, Associativity::Left, &["STAR"])
        .rule("e", &["e", "PLUS", "e"])
        .rule("e", &["e", "STAR", "e"])
        .rule("e", &["NUM"])
        .build()
        .expect("two levels resolve the grammar");
    let grammar = g.as_grammar();
    let prods = productions(&g);
    let num = g.symbol("NUM").unwrap();
    let plus = g.symbol("PLUS").unwrap();
    let star = g.symbol("STAR").unwrap();
    assert!(parse(&grammar, &prods, &[num, plus, num, star, num]).is_some());
}

#[test]
fn reports_undefined_symbols() {
    let err = GrammarBuilder::new()
        .terminal("NUM")
        .rule("s", &["NUM", "MISSING"])
        .build()
        .expect_err("MISSING is neither a terminal nor a rule");
    assert_eq!(err, Conflict::UndefinedSymbol("MISSING".to_string()));
}

#[test]
fn json_and_python_grammars_are_conflict_free() {
    grammars::json().build().expect("json has no conflicts");
    grammars::python_subset().build().expect("the python subset has no conflicts");
}
