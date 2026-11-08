//! Table-driven parses through the flat ABI slices.

mod common;

use common::{names, parse, productions, render, Tree};
use deep_tables::{grammars, Compiled};

fn build() -> Compiled {
    grammars::expr().build().expect("the expression grammar compiles")
}

fn ids(c: &Compiled, tokens: &[&str]) -> Vec<u16> {
    tokens
        .iter()
        .map(|t| c.symbol(t).unwrap_or_else(|| panic!("unknown symbol {t}")))
        .collect()
}

fn accepts(c: &Compiled, tokens: &[&str]) -> bool {
    let g = c.as_grammar();
    let prods = productions(c);
    parse(&g, &prods, &ids(c, tokens)).is_some()
}

#[test]
fn accepts_valid_expressions() {
    let c = build();
    let cases: &[&[&str]] = &[
        &["NUM"],
        &["NAME"],
        &["NUM", "PLUS", "NUM"],
        &["NUM", "MINUS", "NUM"],
        &["NUM", "STAR", "NUM"],
        &["NUM", "SLASH", "NUM"],
        &["NUM", "PLUS", "NUM", "STAR", "NUM"],
        &["NUM", "STAR", "NUM", "PLUS", "NUM"],
        &["LP", "NUM", "RP"],
        &["LP", "NUM", "PLUS", "NAME", "RP", "STAR", "NUM"],
        &["MINUS", "NUM"],
        &["MINUS", "LP", "NUM", "PLUS", "NUM", "RP"],
        &["NAME", "SLASH", "NUM", "STAR", "NUM"],
        &["NUM", "PLUS", "NUM", "MINUS", "NUM", "STAR", "NUM", "SLASH", "NAME"],
    ];
    for case in cases {
        assert!(accepts(&c, case), "should accept {case:?}");
    }
}

#[test]
fn rejects_invalid_expressions() {
    let c = build();
    let cases: &[&[&str]] = &[
        &[],
        &["PLUS", "NUM"],
        &["NUM", "NUM"],
        &["NUM", "PLUS"],
        &["LP", "NUM"],
        &["NUM", "RP"],
        &["STAR", "NUM"],
        &["NUM", "PLUS", "STAR", "NUM"],
        &["LP", "RP"],
        &["NUM", "MINUS", "MINUS"],
    ];
    for case in cases {
        assert!(!accepts(&c, case), "should reject {case:?}");
    }
}

#[test]
fn builds_the_precedence_tree() {
    let c = build();
    let g = c.as_grammar();
    let prods = productions(&c);
    let tokens = ids(&c, &["NUM", "PLUS", "NUM", "STAR", "NUM"]);
    let tree = parse(&g, &prods, &tokens).expect("accepted");
    let got = render(&names(&c), &tree);
    let want = "expr(expr(term(factor(atom(NUM)))),PLUS,\
                term(term(factor(atom(NUM))),STAR,factor(atom(NUM))))";
    assert_eq!(got, want);
}

#[test]
fn addition_is_left_associative() {
    let c = build();
    let g = c.as_grammar();
    let prods = productions(&c);
    let tokens = ids(&c, &["NUM", "PLUS", "NUM", "PLUS", "NUM"]);
    let tree = parse(&g, &prods, &tokens).expect("accepted");
    let names = names(&c);
    let expr = c.symbol("expr").unwrap();
    match tree {
        Tree::Node(sym, children)
            if sym == expr
                && children.len() == 3
                && matches!(&children[0], Tree::Node(s, _) if *s == expr) =>
        {
            assert_eq!(render(&names, &children[0]), "expr(expr(term(factor(atom(NUM)))),PLUS,term(factor(atom(NUM))))");
        }
        other => panic!("expected a left-nested expr, got {other:?}"),
    }
}

#[test]
fn parses_bracketed_arithmetic() {
    let c = build();
    let g = c.as_grammar();
    let prods = productions(&c);
    let tokens = ids(&c, &["LP", "NUM", "PLUS", "NAME", "RP", "STAR", "NUM"]);
    let tree = parse(&g, &prods, &tokens).expect("accepted");
    assert_eq!(
        render(&names(&c), &tree),
        "expr(term(term(factor(atom(LP,expr(expr(term(factor(atom(NUM)))),PLUS,term(factor(atom(NAME)))),RP))),STAR,factor(atom(NUM))))"
    );
}

#[test]
fn parses_a_json_document() {
    let c = grammars::json().build().expect("json compiles");
    let g = c.as_grammar();
    let prods = productions(&c);
    let tokens = ids(
        &c,
        &[
            "LBRACE", "STRING", "COLON", "LBRACKET", "NUMBER", "COMMA", "TRUE", "RBRACKET",
            "COMMA", "STRING", "COLON", "NULL", "RBRACE",
        ],
    );
    assert!(parse(&g, &prods, &tokens).is_some());
    let broken = ids(
        &c,
        &["LBRACE", "STRING", "NUMBER", "RBRACE"],
    );
    assert!(parse(&g, &prods, &broken).is_none(), "a member needs its colon");
}

#[test]
fn parses_a_python_function() {
    let c = grammars::python_subset().build().expect("python subset compiles");
    let g = c.as_grammar();
    let prods = productions(&c);
    let tokens = ids(
        &c,
        &[
            "DEF", "NAME", "LP", "NAME", "COMMA", "NAME", "RP", "COLON", "NEWLINE", "INDENT",
            "RETURN", "NAME", "PLUS", "NUMBER", "NEWLINE", "DEDENT",
        ],
    );
    assert!(parse(&g, &prods, &tokens).is_some());
    let if_stmt = ids(
        &c,
        &[
            "IF", "NAME", "COLON", "NEWLINE", "INDENT", "NAME", "EQUALS", "NUMBER", "NEWLINE",
            "DEDENT", "ELSE", "COLON", "NEWLINE", "INDENT", "PASS", "NEWLINE", "DEDENT",
        ],
    );
    assert!(parse(&g, &prods, &if_stmt).is_some(), "if/else with suites");
    let missing_suite = ids(&c, &["WHILE", "NAME", "COLON"]);
    assert!(parse(&g, &prods, &missing_suite).is_none());
}
