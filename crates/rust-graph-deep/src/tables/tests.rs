//! The table path's own tests: the JSON grammar, the conflicts it must report,
//! what it refuses, and what the emitted source costs.

use super::*;
use crate::read::grammar as read;
use crate::tests::toks;

/// The JSON grammar of `deep-grammars`, unchanged: the same DSL text the
/// recursive path compiles must compile here too.
const JSON: &str = r#"JSON {
    name = "json";
    lex = lex;
    mode = tables;
    runtime = deep_rt;
    tokens {
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        COLON = ":",
        COMMA = ",",
        STRING = "string",
        NUMBER = "number",
        TRUE = "true",
        FALSE = "false",
        NULL = "null",
        BAD_STRING = "bad_string" bad,
        WORD = "word" bad,
    }
    document: _value*,
    _value: object | array | STRING | NUMBER | TRUE | FALSE | NULL,
    object: "{" (pair % ",") "}",
    array: "[" (_value % ",") "]",
    pair: STRING ":" _value,
}"#;

fn json() -> Grammar {
    read(toks(JSON)).expect("the JSON grammar reads")
}

fn error(source: &str) -> String {
    match read(toks(source)) {
        Ok(_) => panic!("accepted: {source}"),
        Err(error) => error.message,
    }
}

#[test]
fn json_lowers_to_a_conflict_free_table_grammar() {
    let tables = build(&json()).expect("the JSON grammar compiles to tables");
    // "{" "}" "[" "]" ":" "," string number true false null bad_string word
    assert_eq!(tables.compiled.terminal_count(), 14, "thirteen tokens and end of input");
    assert_eq!(tables.compiled.symbol("document"), Some(tables.root));
    assert!(tables.compiled.states() > 10);
    // `_value` and the three anonymous non-terminals make no node.
    assert_eq!(tables.hidden.len(), 4, "hidden: {:?}", tables.hidden);
    assert!(tables.kinds.iter().any(|(constant, _)| constant == "DOCUMENT"));
    assert!(tables.kinds.iter().any(|(constant, _)| constant == "PAIR"));
    assert!(tables.kinds.iter().any(|(constant, _)| constant == "BAD_STRING"));
    assert!(!tables.kinds.iter().any(|(constant, _)| constant == "_VALUE"));
    assert!(tables.compiled.runtime_tables().ambiguous_states.is_empty());
}

#[test]
fn repetition_becomes_an_anonymous_non_terminal() {
    let tables = build(&json()).expect("compiles");
    let productions: Vec<String> = (0..tables.compiled.production_count() as u16)
        .map(|id| {
            let production = tables.compiled.production(id);
            let lhs = tables.compiled.symbol_name(production.lhs).unwrap_or("?");
            let rhs: Vec<&str> = production
                .rhs
                .iter()
                .map(|symbol| tables.compiled.symbol_name(*symbol).unwrap_or("?"))
                .collect();
            format!("{lhs}: {}", rhs.join(" "))
        })
        .collect();
    assert!(productions.iter().any(|p| p.starts_with("_gen_star_") && p.ends_with(": ")), "an empty loop: {productions:?}");
    assert!(
        productions.iter().any(|p| p.contains("_gen_star_") && p.contains(" _value")),
        "the loop takes a value: {productions:?}"
    );
    assert!(
        productions.iter().any(|p| p.contains("_gen_sep_") && p.contains(" , pair")),
        "a separated list: {productions:?}"
    );
    // The root is the first rule the builder saw, so it is the start symbol.
    assert_eq!(tables.compiled.production(1).lhs, tables.compiled.start());
}

#[test]
fn the_emitted_source_holds_the_abi_static_and_the_kinds() {
    let source = emit(&json()).expect("emits");
    assert!(source.contains("pub const LBRACE: u16 = 1;"), "{source}");
    assert!(source.contains("pub static JSON_SYMBOLS: &[&str]"), "{source}");
    assert!(source.contains("pub static JSON_HIDDEN: &[u16]"), "{source}");
    assert!(source.contains("pub static JSON: deep_rt::Grammar<'static>"), "{source}");
    assert!(source.contains("action_index: JSON_ACTION_INDEX"), "{source}");
    assert!(source.contains("production_len: JSON_PRODUCTION_LEN"), "{source}");
    // Every symbol the runtime may look up is inside a row of its own.
    let tables = build(&json()).unwrap();
    let runtime = tables.compiled.runtime_tables();
    assert_eq!(runtime.action_table.len(), runtime.states as usize * (runtime.symbols as usize + 1));
}

#[test]
fn an_ambiguous_grammar_fails_with_the_state_symbol_and_both_actions() {
    let grammar = read(toks(
        r#"AMBIG {
            name = "ambig";
            lex = lex;
            mode = tables;
            runtime = deep_rt;
            tokens { NUM = "number", PLUS = "+", }
            expr: expr "+" expr | NUM,
        }"#,
    ))
    .expect("reads");
    let error = emit(&grammar).expect_err("the grammar is ambiguous").message;
    assert!(error.contains("shift/reduce conflict in state"), "{error}");
    assert!(error.contains("on the terminal \"+\""), "{error}");
    assert!(error.contains("shift to state"), "{error}");
    assert!(error.contains("reduce by `expr: expr + expr`"), "the competing rule is named: {error}");
    assert!(error.contains("mode = recursive"), "{error}");
}

#[test]
fn a_reduce_reduce_conflict_names_both_rules() {
    let grammar = read(toks(
        r#"RR {
            name = "rr";
            lex = lex;
            mode = tables;
            runtime = deep_rt;
            tokens { A = "aa", B = "bb", }
            top: x,
            x: a | b,
            a: A,
            b: A,
        }"#,
    ))
    .expect("reads");
    let error = emit(&grammar).expect_err("two rules accept `a`").message;
    assert!(error.contains("reduce/reduce conflict in state"), "{error}");
    assert!(error.contains("`a: aa`") || error.contains("`b: aa`"), "{error}");
    assert!(error.contains(" or by `"), "{error}");
}

#[test]
fn what_a_table_cannot_express_is_refused_by_name() {
    let predicate = read(toks(
        r#"P {
            name = "p";
            lex = lex;
            tokens { A = "a", B = "b", }
            top: x*, x: &(A) A | B,
        }"#,
    ))
    .unwrap();
    let refused = unsupported(&predicate).expect("a predicate is not a table").message;
    assert!(refused.contains("predicate"), "{refused}");

    let table = read(toks(
        r#"T {
            name = "t";
            lex = lex;
            tokens { A = "a", PLUS = "+", LP = "(" open p, RP = ")" close p, }
            top: _e*, _e: precedence A { left "+" => sum, }, args: "(" ")" ,
        }"#,
    ))
    .unwrap();
    let refused = unsupported(&table).expect("a precedence table is not a table grammar").message;
    assert!(refused.contains("precedence"), "{refused}");

    let layout = read(toks(
        r#"L {
            name = "l";
            lex = lex;
            layout = NL IN DE;
            tokens { A = "a", NL = "newline", IN = "indent" open b, DE = "dedent" close b, }
            top: A NL,
        }"#,
    ))
    .unwrap();
    let refused = unsupported(&layout).expect("layout is the lexer's").message;
    assert!(refused.contains("layout"), "{refused}");

    // A grammar that names no table runtime cannot be driven by one.
    let mut runtime_free = json();
    runtime_free.runtime = DEFAULT_RUNTIME.to_string();
    let refused = unsupported(&runtime_free).expect("the default runtime has no tables").message;
    assert!(refused.contains("reads no tables"), "{refused}");
}

#[test]
fn left_recursion_is_accepted_where_the_recursive_path_rejects_it() {
    let grammar = read(toks(
        r#"LR {
            name = "lr";
            lex = lex;
            mode = tables;
            runtime = deep_rt;
            tokens { NUM = "number", PLUS = "+", STAR = "*", }
            expr: expr "+" expr | expr "*" expr | NUM,
        }"#,
    ))
    .unwrap();
    let error = emit(&grammar).expect_err("ambiguous without a precedence table").message;
    assert!(error.contains("shift/reduce conflict"), "{error}");

    let grammar = read(toks(
        r#"LR {
            name = "lr";
            lex = lex;
            mode = tables;
            runtime = deep_rt;
            tokens { NUM = "number", PLUS = "+", STAR = "*", }
            expr: expr "+" term | term,
            term: term "*" factor | factor,
            factor: NUM,
        }"#,
    ))
    .unwrap();
    let tables = build(&grammar).expect("an unambiguous left-recursive grammar compiles");
    assert!(tables.compiled.states() > 5);
}

#[test]
fn the_default_mode_compiles_expressible_grammars_to_tables() {
    let no_mode = JSON.replace("mode = tables;", "");
    let grammar = read(toks(&no_mode)).expect("reads");
    let source = crate::compile(grammar).expect("compiles");
    assert!(source.contains("deep_rt::Grammar<'static>"), "the default is tables: {source}");
    assert!(!source.contains("parse_document"), "no parser functions are emitted: {source}");

    // A grammar the generator cannot express falls back, so the grammars that
    // still use predicates keep building.
    let grammar = read(toks(
        r#"P {
            name = "p";
            lex = lex;
            tokens { A = "a", B = "b", }
            top: x*, x: &(A) A | B,
        }"#,
    ))
    .unwrap();
    let source = crate::compile(grammar).expect("compiles the recursive way");
    assert!(source.contains("parse_top"), "the fallback emits parser functions: {source}");
    assert!(!source.contains("Grammar<'static>"));
}

#[test]
fn the_json_tables_cost_what_they_cost() {
    use std::time::Instant;
    let tables = build(&json()).expect("compiles");
    let runtime = tables.compiled.runtime_tables();
    let source = emit(&json()).expect("emits");
    eprintln!(
        "json: {} states, {} symbols, {} productions; {} bytes of tables \
         (action {} cells, goto {} cells); {} bytes of generated source",
        runtime.states,
        runtime.symbols,
        tables.compiled.production_count(),
        tables.bytes(),
        runtime.action_table.len(),
        runtime.goto_table.len(),
        source.len()
    );
    assert_eq!(
        tables.bytes(),
        runtime.action_index.len() * 4
            + runtime.action_table.len() * 4
            + runtime.goto_index.len() * 4
            + runtime.goto_table.len() * 4
            + runtime.default_reduce.len() * 2
            + runtime.production_len.len()
    );

    let start = Instant::now();
    for _ in 0..20 {
        build(&json()).expect("compiles");
    }
    eprintln!("json: {:?} per LALR(1) build", start.elapsed() / 20);
}

#[test]
fn the_mode_setting_is_read() {
    for (source, want) in [
        ("G { lex = l; tokens { A = \"a\", } a: A, }", crate::read::Mode::Auto),
        ("G { lex = l; mode = tables; tokens { A = \"a\", } a: A, }", crate::read::Mode::Tables),
        ("G { lex = l; mode = recursive; tokens { A = \"a\", } a: A, }", crate::read::Mode::Recursive),
    ] {
        let grammar = read(toks(source)).expect("reads");
        assert_eq!(grammar.mode, want, "{source}");
    }
    let message = error("G { lex = l; mode = magic; tokens { A = \"a\", } a: A, }");
    assert!(message.contains("`mode` is `auto`, `tables` or `recursive`"), "{message}");
}
