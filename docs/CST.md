# The CST reader

What the deep reader promises, the number each promise is measured by, and what
adding a language costs. The guard's own cost lives in `MEASURE.md`; this file is
about the parser alone.

## Node kinds

A kind is a `u16` index into a table the language owns (`deep::syntax`). Nine of
those values are the runtime's — `ERROR`, `SKIPPED`, `EOF`, and the trivia
classes — and everything from `FIRST_LANGUAGE_KIND` (16) up is the language's own
`KindInfo { name, class }`. The arena and the guard never name a kind: the record
is 16 bytes of kind, flags, and byte range, and the guard reads the fact tags
(`a`, `r`, `x`), not the grammar's vocabulary. That is why a new kind is a table
entry in the grammar crate and nothing else.

Two rules the tests pin:

- every entry is addressable by its index and the record stays 16 bytes
  (`deep_facts::a_kind_is_a_table_index_and_the_arena_does_not_grow`);
- every node's byte range reconstructs its slice, nests inside its parent and the
  root spans the file (`deep_facts::every_node_range_reconstructs_its_source_slice`).

Names are not required to be unique: a grammar may write two rules that produce
the same node name, and that is the language's decision, not the arena's.

## Recovery policy

The parser closes the smallest open node, wraps the tokens it could not place in
exactly one `ERROR`, and resumes at the next statement boundary — one error node
per recovery, not one per token (`deep_facts::one_bad_statement_is_one_error_and_the_rest_parses`).

Recovery has a budget. `Cst::degraded()` is the one place the bound is written: a
tree that needed an error node for more than one node in a hundred is refused
under `auto` with the reason code `CST_DEGRADED:recovery`, and the lexer's record
is emitted instead. `--deep` obeys the user and parses anyway, `--compare` reports
both, so a harness can still see a bad tree rather than only its rejection. A fuzz
pass over a thousand mutated inputs requires a tree or a clean refusal and never a
panic (`tests/fuzz_recovery.rs`).

## Adding a language

One entry plus one file, per reader. For the CST:

1. write the grammar module under `src/deep/` (or `crates/deep-grammars/`) with
   its `KINDS` table and a `pub static X: Language = Language::new(name, ROOT,
   KINDS, lex, parse)` — and `with_layout(..)` if indentation is syntax;
2. add one arm to `deep::language_for` in `src/deep/mod.rs`;
3. add a fixture and one claim to `tests/registry.rs`;
4. run `rust-graph-deep-scan conformance` on a corpus and check that the kinds
   seen against declared are not a rounding error.

For the one-pass lexer the same shape applies to `LANGUAGES` in `src/langs.rs`.
Neither reader registers in the other's table, and `tests/layering.rs` fails if a
module ever names the other side.

## Parse time

Budget: **every file under 2 ms at the 95th percentile**, per language, on the
development laptop. Measured with the release build of the harness, three parses
per file, best time counting:

```bash
B=/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph-deep-scan
$B conformance /tmp/opencode/sweep-local/<corpus> --reps 3
```

| corpus | files | p50 ms | p95 ms | mean ms | max ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| js | 2000 | 0.021 | **0.624** | 0.515 | 200.2 |
| ts | 2000 | 0.008 | **0.274** | 0.080 | 7.9 |
| python | 568 | 0.169 | **1.896** | 0.441 | 6.2 |
| rust | 1816 | 0.175 | **1.063** | 0.366 | 15.4 |

Measured 2026-11-13 on the four local corpora. Python is the one to watch: 1.9 ms
at the 95th percentile is inside the budget and a few tenths from it, while js and
ts sit at a third of it. The maxima are single files — the 9 MB `typescript.js`
bundle, a 15 ms rust file — and the budget is deliberately about the percentile,
not the outlier: a 9 MB file is not a parse the editor waits on.

## Memory

Budget: **a 2,000-file corpus run stays under 300 MB peak RSS**. Measured with
the bench harness, one parse per file, the caller holding every source at once:

```bash
$B bench /tmp/js2000.list --reps 1     # prints rss_kb at the end
```

| corpus | files | bytes | nodes | peak RSS |
| --- | ---: | ---: | ---: | ---: |
| js | 2000 | 47.3 MB | 10,979,991 | **129 MB** |
| ts | 2000 | 12.0 MB | 1,830,042 | **17.9 MB** |
| python | 568 | 10.6 MB | 1,860,245 | **14.4 MB** |
| rust | 1816 | 33.7 MB | 8,108,679 | **45.2 MB** |

Measured 2026-11-13. The arena is reused across files, so peak memory tracks the
largest single tree plus the sources the harness holds — not the corpus size.
js is the largest because its corpus is 47 MB of source and the biggest file is a
9 MB bundle; a 2,000-file ts run allocates a seventh of the budget.

## What is not in this number

- Reading the file and building the record. `conformance` measures
  `Workspace::parse` only.
- The guard. It runs on both readers' records and is the larger cost
  (`docs/MEASURE.md`, "The guard's cost").
- Caches. Every measurement is `--reps` parses of the same bytes, so a warm
  cache would be invisible; the corpus files are read once and parsed repeatedly
  on purpose.

## Coverage and recovery live beside it

The same command prints `kinds=seen/declared` and `errors` per file, which is how
a grammar's reach is checked: a corpus that never produces a kind has not
exercised that part of the grammar, and a file with more than one error node in a
hundred is refused outright (`Cst::degraded`, `docs/EVENT-TAXONOMY.md`).
