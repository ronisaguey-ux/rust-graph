//! JSON through `grammar!`: the Slice 2 gate. The generated parser must produce
//! the same events, tree and outcome as the hand-written one in `json.rs`, which
//! stays as the reference. Both use the same lexer, so this compares parsers only.

rust_graph_deep::grammar! { JSON {
    name = "json";
    lex = super::json::lex;

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
        /// A string with no closing quote on its line.
        BAD_STRING = "bad_string" bad,
        /// A bare word that is not `true`, `false` or `null`.
        WORD = "word" bad,
    }

    document: _value*,
    _value: object | array | STRING | NUMBER | TRUE | FALSE | NULL,
    object: "{" (pair % ",") "}",
    array: "[" (_value % ",") "]",
    pair: STRING ":" _value,
} }

/// The shared lexer returns `json.rs`'s kinds, so every one must be the same number.
const _: () = {
    use super::json as hand;
    let same = [
        (LBRACE, hand::LBRACE),
        (RBRACE, hand::RBRACE),
        (LBRACKET, hand::LBRACKET),
        (RBRACKET, hand::RBRACKET),
        (COLON, hand::COLON),
        (COMMA, hand::COMMA),
        (STRING, hand::STRING),
        (NUMBER, hand::NUMBER),
        (TRUE, hand::TRUE),
        (FALSE, hand::FALSE),
        (NULL, hand::NULL),
        (BAD_STRING, hand::BAD_STRING),
        (WORD, hand::WORD),
        (DOCUMENT, hand::DOCUMENT),
        (OBJECT, hand::OBJECT),
        (ARRAY, hand::ARRAY),
        (PAIR, hand::PAIR),
    ];
    let mut at = 0;
    while at < same.len() {
        assert!(same[at].0 == same[at].1, "a generated JSON kind differs from json.rs");
        at += 1;
    }
};

#[cfg(test)]
mod gate {
    use std::time::Duration;

    use super::super::budget::{Budget, Outcome};
    use super::super::json;
    use super::super::parser::Workspace;
    use super::super::tests::{big_document, check, mutate, on_big_stack, Rng, ANY_JSON_BYTE, FIXTURES_5910, MILLION};
    use super::JSON;

    /// Parses `src` with both parsers and requires the same events, tree and
    /// outcome, steps included: the generated code makes the same runtime calls.
    fn same(hand: &mut Workspace, generated: &mut Workspace, src: &[u8], budget: &Budget) -> Outcome {
        let want = hand.parse(src, &json::JSON, budget).unwrap();
        let got = generated.parse(src, &JSON, budget).unwrap();
        let shown = || String::from_utf8_lossy(&src[..src.len().min(200)]).into_owned();
        assert_eq!(got, want, "outcome: {}", shown());
        assert!(generated.events == hand.events, "events differ: {}", shown());
        assert!(generated.tree.nodes == hand.tree.nodes, "trees differ: {}", shown());
        check(generated, src, &JSON);
        want
    }

    fn same_default(src: &[u8]) -> Outcome {
        same(&mut Workspace::default(), &mut Workspace::default(), src, &Budget::default())
    }

    #[test]
    fn the_kind_tables_are_identical() {
        assert_eq!((JSON.name, JSON.root), (json::JSON.name, json::JSON.root));
        let rows = |kinds: &[super::super::syntax::KindInfo]| kinds.iter().map(|info| (info.name, info.class)).collect::<Vec<_>>();
        assert_eq!(rows(JSON.kinds), rows(json::JSON.kinds));
    }

    /// Every input the Slice 1 JSON tests parse.
    const SLICE_1_INPUTS: &[&str] = &[
        r#"{"a": [1, -2.5e+3, true, false, null, {"b": "c\"d"}], "e": {}, "f": []} // done"#,
        "{,}",
        "[1,]",
        r#"{"a":1,}"#,
        "[1 2]",
        r#"{"a" 1}"#,
        r#"{"a":}"#,
        "[",
        "]",
        "{",
        r#""open"#,
        "nul",
        "-",
        "@",
        "[1,,2]",
        r#"{"a":1 "b":2}"#,
        r#"[{"a":1, [2]"#,
        r#"[{"a":1]"#,
        r#"{"a":1}}]"#,
        "[\"open,\n 2\n]",
        "/* c */ [1 /* x */, @@ 2] // end\n",
        r#"[{"before":1},{]},{"after":2}]"#,
        "[[[[1]]], 2]",
        "",
        "   ",
        "// only a comment",
        "/* unclosed comment",
    ];

    #[test]
    fn the_fixtures_give_the_same_events() {
        for (src, _) in FIXTURES_5910 {
            assert!(same_default(src.as_bytes()).complete());
        }
        for src in SLICE_1_INPUTS {
            same_default(src.as_bytes());
        }
        for broken in [r#"{"b" "c"}"#, r#"{"b":}"#, r#"{"b" 1 2 3}"#, r#"{:::}"#, r#"{"a":[1 2}"#, r#"{"x": tru}"#, r#"{"b":"c" "d"}"#, "{[}"] {
            same_default(format!(r#"[{{"before":1}},{broken},{{"after":2}}]"#).as_bytes());
        }
    }

    #[test]
    fn every_budget_degrades_both_the_same_way() {
        let src = big_document(200);
        let (mut hand, mut generated) = (Workspace::default(), Workspace::default());
        for budget in [
            Budget::default(),
            Budget { steps_per_token: 0, base_steps: 500, ..Budget::default() },
            Budget { time: Some(Duration::ZERO), ..Budget::default() },
            Budget { max_depth: 2, ..Budget::default() },
            Budget { max_depth: 0, ..Budget::default() },
        ] {
            same(&mut hand, &mut generated, src.as_bytes(), &budget);
        }
    }

    #[test]
    fn million_deep_inputs_give_the_same_events() {
        on_big_stack(|| {
            for src in [
                format!("{}{}\n", "[".repeat(MILLION), "]".repeat(MILLION)),
                "[".repeat(MILLION),
                "]".repeat(MILLION),
                format!("{}1{}", r#"{"a":"#.repeat(MILLION), "}".repeat(MILLION)),
            ] {
                same_default(src.as_bytes());
            }
        });
    }

    #[test]
    fn fuzzed_input_gives_the_same_events() {
        let mut seeds: Vec<Vec<u8>> = FIXTURES_5910.iter().map(|(src, _)| src.as_bytes().to_vec()).collect();
        seeds.extend(SLICE_1_INPUTS.iter().map(|src| src.as_bytes().to_vec()));
        seeds.push(big_document(3).into_bytes());
        let mut rng = Rng(0x5100_2a77_a5ed);
        let (mut hand, mut generated) = (Workspace::default(), Workspace::default());
        let budget = Budget::default();
        for round in 0..20_000 {
            let mut src = seeds[round % seeds.len()].clone();
            for _ in 0..1 + rng.below(8) {
                let length = src.len();
                mutate(&mut rng, &mut src, ANY_JSON_BYTE, b"", 0, length);
            }
            same(&mut hand, &mut generated, &src, &budget);
        }
    }
}
