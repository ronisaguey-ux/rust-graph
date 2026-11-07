//! The deep scan: a real parser behind the `parser` feature. The default build
//! never compiles this module.

/// Bumped whenever parser output changes. Part of the `facts.pack` key from
/// Slice 4, so a new parser never reads facts cached by an old one.
pub const PARSER_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    rust_graph_deep::grammar! { Python {
        func_def: "def" IDENT "(" [params] ")" ":" block,
        call: IDENT "(" [args] ")",
    } }

    #[test]
    fn the_macro_reads_the_grammar_shape() {
        assert_eq!(Python::NAME, "Python");
        assert_eq!(Python::RULES, ["func_def", "call"]);
    }
}
