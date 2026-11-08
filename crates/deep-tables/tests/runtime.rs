//! The encoded tables read through `deep-rt` itself, not through a copy of its
//! lookup: a cell this crate writes must decode to the action its raw table
//! holds, in the runtime that will consume it.

use deep_abi::Grammar;
use deep_tables::{abi, grammars, Compiled};

/// The encoded tables as the ABI value, borrowed from `tables`.
fn grammar<'a>(compiled: &'a Compiled, tables: &'a abi::RuntimeTables) -> Grammar<'a> {
    Grammar {
        name: compiled.name(),
        states: tables.states,
        symbols: tables.symbols,
        action_index: &tables.action_index,
        action_table: &tables.action_table,
        goto_index: &tables.goto_index,
        goto_table: &tables.goto_table,
        default_reduce: &tables.default_reduce,
        production_len: &tables.production_len,
    }
}

/// `deep-rt` decodes into its own `Action`; this crate's is the one the raw
/// tables carry, so compare in that vocabulary.
fn same(action: deep_rt::Action) -> deep_tables::Action {
    match action {
        deep_rt::Action::None => deep_tables::Action::Error,
        deep_rt::Action::Shift(state) => deep_tables::Action::Shift(state),
        deep_rt::Action::Reduce(production) => deep_tables::Action::Reduce(production as u16),
        deep_rt::Action::Accept => deep_tables::Action::Accept,
    }
}

fn all() -> Vec<(&'static str, Compiled)> {
    [
        ("expr", grammars::expr()),
        ("json", grammars::json()),
        ("python-subset", grammars::python_subset()),
    ]
    .into_iter()
    .map(|(name, builder)| (name, builder.build().expect("compiles")))
    .collect()
}

#[test]
fn the_runtime_reads_the_same_action_everywhere() {
    for (name, compiled) in all() {
        let tables = compiled.runtime_tables();
        let g = grammar(&compiled, &tables);
        let raw = compiled.raw();
        for state in 0..compiled.states() {
            for symbol in 0..compiled.symbol_count() as u16 {
                let want = raw.action(state as u32, symbol);
                // This crate numbers end of input as 0; the runtime asks for it
                // as `EOF_SYMBOL` and the encoder maps it to the column past
                // the last symbol.
                let lookahead = if symbol == 0 { deep_abi::EOF_SYMBOL } else { symbol };
                let got = same(deep_rt::action(&g, state as u16, lookahead));
                assert_eq!(got, want, "{name}: state {state}, symbol {symbol}");
            }
        }
    }
}

#[test]
fn the_runtime_reads_the_same_goto_everywhere() {
    for (name, compiled) in all() {
        let tables = compiled.runtime_tables();
        let g = grammar(&compiled, &tables);
        let raw = compiled.raw();
        for state in 0..compiled.states() {
            for symbol in 0..compiled.symbol_count() as u16 {
                let want = raw.goto[state]
                    .iter()
                    .find(|(sym, _)| *sym == symbol)
                    .map(|(_, target)| *target);
                assert_eq!(deep_rt::goto(&g, state as u16, symbol), want, "{name}: state {state}, symbol {symbol}");
            }
        }
    }
}

#[test]
fn the_end_of_input_uses_the_column_past_the_last_symbol() {
    let json = grammars::json().build().expect("json compiles");
    let tables = json.runtime_tables();
    let g = grammar(&json, &tables);
    // The document production reduces on end of input, which the runtime asks
    // for as `EOF_SYMBOL`; the action must be there and not on a real column.
    let accept = (0..json.states())
        .filter(|state| deep_rt::action(&g, *state as u16, deep_abi::EOF_SYMBOL) != deep_rt::Action::None)
        .count();
    assert!(accept > 0, "end of input has actions");
    assert_eq!(tables.action_table.len() % (tables.symbols as usize + 1), 0, "every row spans the full width");
}

#[test]
fn a_state_names_the_nonterminal_it_reduces_by() {
    for (name, compiled) in all() {
        let tables = compiled.runtime_tables();
        let g = grammar(&compiled, &tables);
        for state in 0..compiled.states() {
            let Some(production) = compiled.raw().default_reduce[state] else { continue };
            let want = compiled.production(production).lhs;
            assert_eq!(
                deep_rt::lhs(&g, state as u16, u32::from(production)),
                want,
                "{name}: state {state} reduces by production {production}"
            );
            assert_eq!(deep_rt::production_len(&g, u32::from(production)) as usize, compiled.production(production).rhs.len(), "{name}: production {production}");
        }
    }
}

#[test]
fn the_encoding_says_which_states_it_cannot_name() {
    for (name, compiled) in all() {
        let tables = compiled.runtime_tables();
        assert!(tables.ambiguous_states.is_empty(), "{name}: {tables:?}", tables = tables.ambiguous_states);
    }
}
