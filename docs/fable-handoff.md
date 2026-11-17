# Hand-off — what this lane left, and what it would not take

Written 2026-11-17 by the single opencode lane that continued this checkout after
fable's usage window closed. Nothing here is undone work that was started: every
item below was either read and refused, or measured and left. The 31 commits that
were already on the branch are untouched — no rebase, no reset, no amend, and no
reader behaviour was changed by this lane.

## What was taken

| commit | what |
| --- | --- |
| `5d69fb8` | `test(audit):` the arm-merge pin named a measuring commit (`8412e5a`) that is not in this history, so `commit_audit.py --check` failed on the measurement rather than the discipline; re-pinned to `426b3a0`, the first commit after the fix whose body reports a corpus count. |
| `3647547` | `fix(tools):` `readers_doc.py` decoded the coverage tables through `unicode_escape`, which is latin-1, so a `…` in a TypeScript row reached `docs/READERS.md` as mojibake. |
| `75272c2` | `chore(constructs):` `construct_report.py --measure` on the release binary — js 0/1854, ts 2/2000, python 0/568, rust 3/1816. |
| `11d9974` | `chore(gate):` `phase4_gate.py` pinned python 8 and rust 48, so it printed `MOVED` on a win; re-pinned. The gate exits 0. |
| `6243e18` | `fix(tools):` the generator decoded `\"` as a quote where `tests/readers_doc.rs` keeps the character itself, and ended a table at the next line starting with `];`, which runs past an empty `KNOWN_GAPS` table into `LINE_ROWS`. Both were caught by the checker. |
| `6b3cd60` | `test(docs):` `LINE_ROWS` is a table of constructs the tests claim, and neither side listed it; the generator writes it and the checker counts it. The checker also read a literal byte-by-byte into chars, so UTF-8 never matched. |
| `e52f486` | `docs(readers):` regenerated, three TypeScript rows added. |

Verified state at `e52f486`: `tools/loss_count.py` js 0 / ts 2 / python 0 / rust 3
(0, 2, 0, 3 losses); `tools/phase4_gate.py` PASSED; `tools/construct_report.py
--check`, `tools/commit_audit.py --check`, `tools/readers_doc.py --check`,
`tools/corpus_table.py --check` all exit 0; `cargo test --release --features "deep
parser"` green apart from the items under "Suite state" below.

## What is left, one line each

Each line names the item, why this lane would not take it, and the file that owns
it. "Judgement" means the two readers are both defensible and choosing one is a
design call; "grammar" means the CST has no node and adding one is a construct
family; "corpus" means the answer is what the corpus should contain, or how
tolerant the guard should be, not a reader fix.

1. **The two RSI files, `CST_MISSING_KIND:call` (rust 2 of 3)** — corpus: the
   source repeats a `fn` header line (`001214_rsi.rs:58` and `:59`, same in
   `001368_rsi.rs`), so the item is in recovery and the call inside it has no node
   to read — either exclude/normalise the two files or decide a recovery policy
   for a duplicated header; owner `docs/MEASURE.md` (corpus) or
   `src/deep_integration.rs` (recovery budget).
2. **`match` named inside a `json!` token tree, `LEX_OVER_EMIT:name` (rust 1)** —
   judgement: the one-pass reader cannot tell a keyword in a token tree from a
   name, and the fix is in the shared value merges that cost Go 959 files, so it
   wants one deliberate decision, not a lane patch; owner `src/expr.rs`.
3. **`000114_Schema.ts` — an object literal the CST read as `m:[0,0,0,0]`
   (`LEX_OVER_EMIT:name`, ts 1 of 2)** — grammar: a 14,878-line generated schema
   whose `const xml = { method(s: string): string { … } }` loses every member;
   owner `src/deep_facts/mod.rs` and the js/ts object kinds in `src/deep/`.
4. **`001681_schemas.d.ts` — a mapped type in a `.d.ts` return type
   (`PAIR_AMBIGUOUS:1`, ts 1)** — judgement: `-readonly [k in keyof T["shape"]]`
   inside a return type writes an event on a line the classifier cannot pair;
   owner `src/pipeline.rs` (classifier) with `src/deep_facts/mod.rs` for the
   mapped-type reading.
5. **`CST_DEGRADED:recovery` (C 137 of 295, Go 46)** — corpus/tolerance: this is
   the CVD parser's recovery, not a reader disagreement, and the only honest knob
   is the error-node threshold; owner `src/deep_integration.rs`.
6. **`LEX_OVER_EMIT:call` (C 13)** — judgement: `__visibility__("default")` and
   `hidden void __vm_wait(void)` are read as calls of the declared name; which
   side is right depends on whether a decorated prototype is a declaration with a
   macro word or a call; owner `src/expr.rs` with the C declaration arm in
   `src/deep_facts/c_family.rs`.
7. **`CST_SHAPE_MISMATCH:targets` (Go 828, C 5)** — judgement: the two readers
   agree on the line and disagree on the target list, and neither is wrong on its
   own, so this is a reader/guard agreement rather than a fix; owner
   `src/parser.rs`.
8. **`CST_MISSING_KIND:symbol` / `constant` (Go 941 / 22, C 9 / 51)** — grammar:
   the language table and walk arm have no node for the symbol the lexer names;
   owner `src/deep_facts/go.rs`, `src/deep_facts/c_family.rs` and their `src/deep/`
   kinds.
9. **C declarations that begin with a user type and a pointer
   (`CST_MISSING_KIND:var`, 34 rows)** — grammar: `State *p = G(L);` is parsed as
   `expression_statement (assignment_expression (binary_expression identifier
   identifier) …)`, because the grammar cannot tell a typedef from a value, so the
   CST sees a multiplication and there is no binding to compare; a fix is either a
   typedef table or a new declaration production, not a reader patch. Repro:
   `/tmp/rgg1` is Go, the C repro is `typedef struct State State; … State *p =
   G(L);` in one function. Owner `src/deep/` (C grammar) and
   `src/deep_facts/c_family.rs`.
10. **Go's remaining call rows (7,015 of 15,612 files forced)** — continuation of
    the class fable was closing (`ee81b7d`, `af88f33`); each remaining shape is a
    walk arm in `src/deep_facts/go.rs`, and the next one needs the measurement of
    the shape before it, so it is a lane of its own rather than a leftover.
11. **The direction the `call` spelling takes (`af88f33`)** — judgement, and the
    only red suite test this lane could not close: `Find(k) (Item, error)` writes
    `symbol.calls` entry `Find().call` on the CST side and `.call` on the lexer
    side, so the two readers disagree *and* `no_c_family_fact_names_a_thing_the_
    source_does_not_contain` fails, because `call` is a word the source never
    writes. Whoever owns that commit has to pick one spelling and say whether a
    synthesised result-list call is a fact at all; owner `src/deep_facts/go.rs`
    and `tests/facts_c_family_fixtures.rs`.

## Suite state at `e52f486`

`cargo test --release --features "deep parser" --no-fail-fast`, 43 binaries:
everything passes except

- `the_binary_is_small_and_static` (5,621,984 bytes) and
  `a_thousand_files_build_inside_the_limit` (3.18 s) in `tests/benchmark.rs`, and
  `the_deep_scan_beats_tree_sitter_by_the_margin` in `tests/parser_gate.rs` — the
  three gates that were already red on trunk (binary size, thousand-file build,
  missing tsdrv);
- `no_c_family_fact_names_a_thing_the_source_does_not_contain`, item 11 above;
- `deep::tests::the_delimiter_pass_is_linear_on_hostile_input`, which passes when
  run alone (75 ms against 17 ms on a loaded box) and is a timing ratio, not a
  fact about the reader.

## How to measure again

    cargo build --release --features "deep parser"
    python3 tools/loss_count.py --binary target/x86_64-unknown-linux-musl/release/rust-graph
    python3 tools/auto_audit.py c go --binary target/x86_64-unknown-linux-musl/release/rust-graph

Corpora are at `/tmp/opencode/sweep-local/{js,ts,python,rust,c,go}`. A `cargo
test` in the same target directory overwrites the binary the measurements use;
rebuild before believing a number. `phase4_gate.py --save` and
`construct_report.py --measure` re-pin the records this lane just re-pinned, and
`docs/unsolved.md` now carries the numbers this sweep produced.
