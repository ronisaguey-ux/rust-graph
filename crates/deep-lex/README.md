# deep-lex

A zero-copy, context-sensitive lexer for the deep parser. It implements
`deep_abi::Lexer`, so a table-driven runtime drives it one terminal at a time:

```rust
use deep_lex::{DeepLexer, Lexer};

let mut lexer = DeepLexer::for_profile(src, &deep_lex::PYTHON);
while let Some(token) = lexer.next_token(state) {
    // token.symbol is the grammar's terminal, token.start..token.end a slice
    // of `src`
}
```

`state` is the parser's current `StateId`. A profile is the only thing that
makes Python, JSON, JavaScript and TypeScript different: which language rules
to apply, a `StateId -> Mode` table, and the bracket pairs. There is one lexer
implementation and no per-language code path in it.

## What is wired

| Piece | Where | What it does |
| --- | --- | --- |
| `deep_abi::Lexer` | `src/run.rs` | `next_token(state)`: consumes pending layout, applies the state's mode, runs the external scanner, then the ordinary rules. `None` only at EOF. |
| mode stack | `src/mode.rs` | fixed 64-frame stack; `with_state_modes` maps a parser state to `Mode::Regex` / `String` / `RawString` / `Template` / `TemplateExpr`. Template `${` / `}` push and pop without the parser. |
| Python layout | `src/layout.rs`, `src/python.rs` | NEWLINE, INDENT and DEDENT as zero-width tokens; suppressed inside brackets and after a backslash; none on blank or comment-only lines. NEWLINE sits on the last token of the logical line and DEDENTs on the last token of the block, matching the old lexer's offsets byte for byte. |
| bracket pairs | `src/brackets.rs`, `src/recovery.rs` | one O(n) pass, one `Vec<u32>`; `pair()` is that slice, `jump(offset)` is the mate (binary search), `PairJump::skip_to_closer` is the Cascade resume point. |
| profiles | `src/profiles.rs` | `PYTHON`, `JSON`, `JAVASCRIPT`, `TYPESCRIPT`, and `symbol_pairs()` to hand `deep_rt::Config::with_pairs`. |
| JSON | `src/kind.rs`, `src/scan.rs` | `Language::Json`: `"` is the only string delimiter, `-12` is one number, comments are trivia as in tree-sitter-json. |

Allocation: one `Vec` for the bracket table, built once per file; nothing per
token, and derived `Clone`/`Copy` structs for the rest. `std` only, no crates.

## Differential result

`tests/differential.rs` lexes 40 inputs (ten per language, including malformed
ones) with the old lexer in `src/deep/lexer.rs` and with `deep-lex`, comparing
`(kind, start, end)` for every non-trivia token and every layout token.
`tests/fixtures/differential/golden.tsv` is the old lexer's recorded output;
`tests/gen/old_lexer_golden.rs` regenerates it.

Result: **40/40 files match exactly**, with two normalised differences:

1. JavaScript `>` in `>>`: the old JS lexer emits one `>joined` per byte so the
   grammar can close `Map<K, V>>`. `deep-lex` does maximal munch (`GtGt`);
   joining generic closers is a grammar decision. The test rejoins the old
   tokens to byte spans and compares those.
2. Unterminated string/template: old `BAD_STR` / `bad_literal`, here `String` /
   template part. Same span; the surface has no per-language bad kind, so the
   grammar checks the closing byte. The test asserts the span equality.

Trivia is filtered: the old lexer stores whitespace and comments so its tokens
concatenate back to the source; `deep-lex` leaves them on the floor.

## Measurements

`cargo run --release -p deep-lex --example measure`, 1 MiB inputs, x86-64 musl,
single thread, best of five runs (the box is shared, so single runs move 2-3x):

| input | tokens | lexing | throughput | allocations |
| --- | ---: | ---: | ---: | ---: |
| Python | 397,998 | 78.7 ms | 5.1M tok/s, 12.7 MiB/s | 1 (367 KB pair table) |
| JavaScript | 384,133 | 50.1 ms | 7.7M tok/s, 20.0 MiB/s | 1 (748 KB) |
| JSON | 391,637 | 23.4 ms | 16.8M tok/s, 42.8 MiB/s | 1 (404 KB) |
| bracket-free | 144,631 | — | — | 0 |

The bracket pass is separate: 8-14 ms per 1 MiB depending on delimiter density.
Allocations per token are 3e-6 (one table per lexer, not one per token). Peak RSS
over the whole run is ~1.9 MiB, of which the pair table is the largest block.
Python is the slowest lane because the layout stack measures indentation on
every logical line; JSON is fastest because its tokens are short.

## Wiring the rest of the tree (files this field does not own)

1. `crates/deep-rt/src/driver.rs`: add `pub pair: Option<&'a [u32]>` to
   `Config` plus `with_pair_table`. `crates/deep-rt/src/recovery.rs`,
   `matching_close`: consult `deep_lex::brackets::jump_in(pair, token.start)`
   before the token/byte scans (the exact patch is in `src/recovery.rs` docs).
2. `crates/deep-grammars/src/deep.rs`: `lexer` is included from
   `src/deep/lexer.rs` by path; replace the `lex(src, language, &mut tokens)`
   call in `src/deep/parser.rs` (`Workspace::parse`) with
   `DeepLexer::for_profile` driven by the LR state. `deep-grammars`' `Language`
   is a recursive-descent runtime with a `lex: fn(&mut Cursor)` field; the
   state-driven path only exists on `deep-rt`, so the swap is
   `Language::lex` -> `(Profile, SymbolMap)` data.
3. `src/deep/syntax.rs`: `Class::Open`/`Close` already carry the family byte the
   pair table uses; a grammar only needs `SymbolMap::set(Kind::LParen, symbol)`
   per delimiter so `symbol_pairs()` speaks its numbering.
4. `crates/rust-graph-deep` (the `grammar!` macro): emits `lex = lex`; it would
   emit a profile reference instead. No grammar rules change.
