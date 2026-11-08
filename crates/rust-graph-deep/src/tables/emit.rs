//! Writing the result as Rust: the conflict messages a bad grammar gets, and
//! the source a good one becomes.

use std::fmt::Write;

use deep_tables::Conflict;

use super::{build, runtime_error};
use crate::read::Grammar;
use crate::{Error, Spot};

/// A conflict, written in the grammar's own terms: the state, the terminal, and
/// both actions with the productions as the author wrote them.
pub fn conflict_error(rules: &[(String, Vec<String>, Spot)], conflict: Conflict) -> Error {
    // Production 0 is the augmented start rule, so the list is offset by one.
    let text = |production: u16| -> (String, Spot) {
        let at = production as usize;
        if at == 0 {
            return ("the augmented start rule".to_string(), None);
        }
        match rules.get(at - 1) {
            Some((lhs, rhs, spot)) => {
                let body = if rhs.is_empty() { String::new() } else { rhs.join(" ") };
                (format!("`{lhs}: {body}`"), *spot)
            }
            None => (format!("production {production}"), None),
        }
    };
    match conflict {
        Conflict::ShiftReduce { state, terminal_name, shift_to, reduce_by, .. } => {
            let (reduce, spot) = text(reduce_by);
            Error::at(
                spot,
                format!(
                    "shift/reduce conflict in state {state} on the terminal {terminal_name:?}: \
                     shift to state {shift_to}, or reduce by {reduce}\n\
                     the table generator resolves nothing by precedence here: rewrite the rule so one \
                     token decides it, or keep this grammar on the recursive path with `mode = recursive;`"
                ),
            )
        }
        Conflict::ReduceReduce { state, terminal_name, rule_a, rule_b, .. } => {
            let (first, spot) = text(rule_a);
            let (second, _) = text(rule_b);
            Error::at(
                spot,
                format!(
                    "reduce/reduce conflict in state {state} on the terminal {terminal_name:?}: \
                     reduce by {first}, or by {second}\n\
                     two rules accept the same input: make them one rule, or keep this grammar on the \
                     recursive path with `mode = recursive;`"
                ),
            )
        }
        other => Error::at(None, other.to_string()),
    }
}

/// The emitted source: kinds, the symbol names, and the ABI static.
pub fn emit(grammar: &Grammar) -> Result<String, Error> {
    if let Some(error) = runtime_error(grammar) {
        return Err(error);
    }
    let tables = build(grammar)?;
    let name = &grammar.name;
    let compiled = &tables.compiled;
    let runtime = compiled.runtime_tables();
    let mut out = String::new();

    for (constant, symbol) in &tables.kinds {
        if constant == name {
            return Err(Error::at(None, format!("`{constant}` is the name of the generated static")));
        }
        let _ = writeln!(out, "pub const {constant}: u16 = {symbol};");
    }
    let symbol_names: Vec<String> = (0..compiled.symbol_count() as u16)
        .map(|symbol| match compiled.symbol_name(symbol) {
            Some(name) => format!("{name:?}"),
            None => "\"_start\"".to_string(),
        })
        .collect();
    let _ = writeln!(out, "pub static {name}_SYMBOLS: &[&str] = &[{}];", symbol_names.join(", "));
    let hidden: Vec<String> = tables.hidden.iter().map(|symbol| symbol.to_string()).collect();
    let _ = writeln!(out, "pub static {name}_HIDDEN: &[u16] = &[{}];", hidden.join(", "));
    let _ = writeln!(out, "pub const {name}_ROOT: u16 = {};", tables.root);
    let _ = writeln!(out, "static {name}_ACTION_INDEX: &[i32] = &[{}];", numbers(&runtime.action_index));
    let _ = writeln!(out, "static {name}_ACTION_TABLE: &[u32] = &[{}];", numbers(&runtime.action_table));
    let _ = writeln!(out, "static {name}_GOTO_INDEX: &[i32] = &[{}];", numbers(&runtime.goto_index));
    let _ = writeln!(out, "static {name}_GOTO_TABLE: &[u32] = &[{}];", numbers(&runtime.goto_table));
    let _ = writeln!(out, "static {name}_DEFAULT_REDUCE: &[u16] = &[{}];", numbers(&runtime.default_reduce));
    let _ = writeln!(out, "static {name}_PRODUCTION_LEN: &[u8] = &[{}];", numbers(&runtime.production_len));
    let runtime_path = &grammar.runtime;
    let _ = writeln!(
        out,
        "/// `{}` as LALR(1) tables: {} states, {} symbols, {} productions.\n\
         /// The `{name}_HIDDEN` non-terminals make no node.\n\
         pub static {name}: {runtime_path}::Grammar<'static> = {runtime_path}::Grammar {{\n\
         \x20   name: {:?},\n\
         \x20   states: {},\n\
         \x20   symbols: {},\n\
         \x20   action_index: {name}_ACTION_INDEX,\n\
         \x20   action_table: {name}_ACTION_TABLE,\n\
         \x20   goto_index: {name}_GOTO_INDEX,\n\
         \x20   goto_table: {name}_GOTO_TABLE,\n\
         \x20   default_reduce: {name}_DEFAULT_REDUCE,\n\
         \x20   production_len: {name}_PRODUCTION_LEN,\n\
         }};",
        grammar.language,
        runtime.states,
        runtime.symbols,
        compiled.production_count(),
        grammar.language,
        runtime.states,
        runtime.symbols,
    );
    Ok(out)
}

fn numbers<T: std::fmt::Display>(values: &[T]) -> String {
    let mut out = String::new();
    for (at, value) in values.iter().enumerate() {
        if at > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{value}");
    }
    out
}
