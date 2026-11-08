//! Compression round-trips: the packed and ABI forms must answer exactly like
//! the uncompressed reference.

mod common;

use common::{abi_action, abi_goto};
use deep_tables::{grammars, Width};

fn all_symbols(c: &deep_tables::Compiled) -> Vec<u16> {
    (0..c.symbol_count() as u16).collect()
}

#[test]
fn compressed_tables_answer_like_the_raw_ones() {
    for builder in [grammars::expr(), grammars::json(), grammars::python_subset()] {
        let c = builder.build().expect("grammar compiles");
        let t = c.tables();
        for state in 0..c.states() as u32 {
            for sym in all_symbols(&c) {
                assert_eq!(
                    t.action(state, sym),
                    c.raw().action(state, sym),
                    "action mismatch in state {state} on symbol {sym}"
                );
            }
        }
    }
}

#[test]
fn gotos_answer_like_the_raw_ones() {
    let c = grammars::python_subset().build().expect("grammar compiles");
    let t = c.tables();
    let nonterm_start = c.terminal_count() as u16;
    for state in 0..c.states() as u32 {
        let expected = &c.raw().goto[state as usize];
        for sym in nonterm_start..c.symbol_count() as u16 {
            let want = expected
                .iter()
                .find(|(s, _)| *s == sym)
                .map(|(_, target)| *target);
            assert_eq!(t.goto(state, sym), want, "goto mismatch in state {state} on {sym}");
        }
    }
}

#[test]
fn packed_tables_decode_losslessly() {
    for builder in [grammars::expr(), grammars::python_subset()] {
        let c = builder.build().expect("grammar compiles");
        let original = c.tables();
        let packed = original.packed();
        let restored = packed.decode();
        assert_eq!(restored.states, original.states);
        assert_eq!(restored.symbols, original.symbols);
        assert_eq!(restored.action_index, original.action_index);
        assert_eq!(restored.action_table, original.action_table);
        assert_eq!(restored.goto_index, original.goto_index);
        assert_eq!(restored.goto_table, original.goto_table);
        assert_eq!(restored.default_reduce, original.default_reduce);
        assert_eq!(restored.production_len, original.production_len);
        for state in 0..c.states() as u32 {
            for sym in all_symbols(&c) {
                assert_eq!(
                    restored.action(state, sym),
                    original.action(state, sym),
                    "packed decode changed state {state} on {sym}"
                );
            }
        }
    }
}

#[test]
fn the_abi_slices_answer_like_the_crate() {
    for builder in [grammars::expr(), grammars::json(), grammars::python_subset()] {
        let c = builder.build().expect("grammar compiles");
        let g = c.as_grammar();
        let t = c.tables();
        for state in 0..c.states() as u32 {
            for sym in all_symbols(&c) {
                assert_eq!(abi_action(&g, state, sym), t.action(state, sym));
                assert_eq!(abi_goto(&g, state, sym), t.goto(state, sym));
            }
        }
    }
}

#[test]
fn compression_beats_the_dense_matrix() {
    for builder in [grammars::expr(), grammars::json(), grammars::python_subset()] {
        let c = builder.build().expect("grammar compiles");
        let r = c.report();
        assert!(r.packed_bytes < r.raw_bytes, "packed must beat raw");
        assert!(r.packed_bytes <= r.abi_bytes);
        assert!(r.ratio() > 1.5, "expected a real win, got {:.2}x", r.ratio());
        assert!(r.cells < r.states * r.terminals, "rows should be sparse");
    }
}

#[test]
fn small_grammars_narrow_to_u16_cells() {
    let c = grammars::expr().build().expect("grammar compiles");
    assert_eq!(c.tables().width, Width::Half);
    assert_eq!(c.report().cell_bytes(), 2);
    let packed = c.tables().packed();
    assert_eq!(packed.idx_width, 1, "offsets fit a byte here");
}

#[test]
fn default_reductions_are_used() {
    let c = grammars::json().build().expect("grammar compiles");
    let states_with_default = (0..c.states() as u32)
        .filter(|s| c.default_reduce(*s).is_some())
        .count();
    assert!(states_with_default > 0, "some state should default to a reduction");
    for state in 0..c.states() as u32 {
        let explicit = c.tables().action_index[state as usize * 2 + 1];
        assert!(explicit >= 0);
    }
}

#[test]
fn explicit_errors_survive_the_default_reduction() {
    // Non-associative LT puts an explicit error into the table; the default
    // reduction must not paper over it.
    let g = deep_tables::GrammarBuilder::new()
        .name("cmp")
        .terminal("NUM")
        .terminal("LT")
        .precedence(1, deep_tables::Associativity::NonAssoc, &["LT"])
        .rule("c", &["c", "LT", "c"])
        .rule("c", &["NUM"])
        .build()
        .expect("compiles");
    let t = g.tables();
    let lt = g.symbol("LT").unwrap();
    let mut blocked = 0;
    for state in 0..g.states() as u32 {
        let has_default = g.default_reduce(state).is_some();
        if has_default && t.action(state, lt) == deep_tables::Action::Error {
            blocked += 1;
        }
    }
    assert!(blocked > 0, "an explicit error must override a default reduction");
}
