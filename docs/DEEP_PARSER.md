# The deep parser

This is the maintainer's description of the deep parser: the CST parser behind
the `parser` feature of `rust-graph`, the one in `crates/`. It is not the
default build's one-pass lexer (`src/lexer.rs`, `src/parser.rs`), which builds
no tree.

Every claim here is checked by a command in the last section, or by a test named
beside it. Numbers are in the measurement table, never in the prose.

## 1. Where the code is

| path | what it is | entry point |
|:--|:--|:--|
| `crates/deep-abi` | the contract: `Node` (20 bytes), `Token`, `Grammar`, `Lexer`, `Recovery`, `Cst`, `Fatal`. `no_std`, no logic. | — |
| `crates/deep-lex` | the hand lexer crate | see its `README.md` |
| `crates/deep-tables` | LR(1)/LALR(1) table generator for the `tables` path | see its `README.md` |
| `crates/rust-graph-deep` | the `grammar!` proc macro (`check.rs`, `decide.rs`, `emit.rs`, `tables/`) | `grammar!` |
| `crates/deep-grammars` | 30 grammars in the `grammar!` DSL, their lexers, and the fixtures | `Language` statics |
| `crates/deep-rt` | the unified runtime: descent driver, GLR driver, arena, recovery, budgets, incremental re-parse | `Workspace::parse` |
| `src/deep` | a path copy of the descent runtime that `crates/deep-grammars` compiles against today (`crates/deep-grammars/src/deep.rs` includes it by `#[path]`) | `Workspace::parse` |
| `src/deep_integration.rs` | the `parser`-feature bridge from the graph build to the parser | `parse_file` |
| `crates/deep-conformance` | scan, summary, gates, diff over a corpus | `deep-conformance` binary |
| `crates/deep-fuzz` | the deterministic fuzzer, shrinker and tree checker | `deep-fuzz` binary |
| `crates/deep-bench` | the alternating 2× benchmark harness | `deep-bench` binary |

`crates/deep-rt/UNIFICATION.md` explains why there are two engines and one
tree. The short version: a `grammar!` grammar is a set of recursive-descent
functions and cannot be turned into LR tables without a second generator, so
the descent engine stays; what was unified is the surface every caller and
grammar sees (kinds, lexer, pair pass, tree, budget, one `Workspace`).

⚠️ `src/deep` and `crates/deep-rt` both contain the descent runtime. They are
copies, not links: `crates/deep-grammars` still compiles against `src/deep`.
A change to `crates/deep-rt/src/{parser,tree,lexer,pairs,syntax,budget}.rs`
must be mirrored into `src/deep/` until the grammar crate's `src/deep.rs`
`#[path]` include is switched to `deep_rt`. `crates/deep-rt/tests/` is what
pins the unified copy; `crates/deep-grammars/tests/` is what pins the copy the
grammars use.

## 2. The pipeline

```text
&[u8]
  │
  ├─ lexer        Cursor over the bytes, one step function per language
  │               -> Tokens { kind[], start[], end[] }, lossless: trivia kept
  │
  ├─ layout       optional, Python/indent languages: zero-width
  │               NEWLINE/INDENT/DEDENT inserted after the last token
  │               of a logical line
  │
  ├─ pair pass    one linear sweep: pair[open] = closer, pair[closer] = open,
  │               `unclosed` and `stray` counters, #if/#else snapshots
  │
  ├─ engine       grammar! languages: descent (Parser over significant tokens,
  │               events, precede/wrap, group firewall, trials)
  │               table languages: LR/GLR driver over a graph-structured stack,
  │               bounded versions, arena of 20-byte ABI nodes
  │
  ├─ tree         events/arena flattened to one preorder Vec<Node> (16 bytes,
  │               u32 indices, subtree_end instead of child pointers)
  │
  └─ Outcome      tokens, steps, deepest, skipped_groups, exhausted,
                  unclosed, stray
```

The entry point is `Workspace::parse(src, language, &Budget)`. It lexes, pairs,
picks the engine from the language, and leaves the tree in `workspace.tree`; the
`Outcome` says how much work it did and what ran out. The workspace is reused:
in the steady state a parse allocates no new buffers.

### 2.1 Lexer

`deep::lexer::Cursor` is a byte cursor with `peek`, `peek_at`, `behind`,
`since`, `bump`, `eat`, `eat_while`, `quoted` and `bump_char` (which decodes one
UTF-8 sequence, or one byte on invalid input). A language supplies
`fn(&mut Cursor) -> Kind`; it must advance the cursor. The lexer loop enforces
that: a step that does not advance costs one character as `UNKNOWN`, so lexing
always terminates on hostile input. Runs of `UNKNOWN` merge into one token.
Every byte of the input lands in exactly one token, trivia included.

`Tokens` is a struct of arrays, never a `Vec<Token>`: `kind: Vec<Kind>`,
`start: Vec<u32>`, `end: Vec<u32>`. `deep-abi`'s `Token` is only the table
engine's exchange type.

`MAX_INPUT` is `u32::MAX - 1`; longer input is `TooLarge`, not a panic.

### 2.2 Layout

`Layout { newline, indent, dedent, resets }` is a runtime feature, not a
grammar rule. When a language has one:

- every line break is stored as whitespace, and a **zero-width** `newline` is
  inserted after the last token of each logical line (none inside brackets, on
  blank lines, or on comment-only lines);
- `indent` is zero-width at the first token of a deeper line; `dedent` is
  zero-width right after the `newline` that ends a block; they are an
  `Open`/`Close` pair of one family, so they get the same firewall as brackets;
- end of file closes every open level;
- a `resets` token that starts a line inside brackets ends the bracket there,
  so an unclosed `(` costs one line, not the rest of the file.

### 2.3 Pair pass

`deep::pairs` pairs delimiters before the parser runs its first rule. It is one
linear sweep with a stack of `(token, family)`:

- a closer searches at most `SEARCH_WINDOW` (8) entries down for its family;
  openers above it are closed as unclosed (`{ ( }` pairs the braces);
- a closer that finds nothing in the window is `stray`, not an error node;
- `#if/#else/#endif` take a stack snapshot and restore it in each branch, with
  snapshots skipped past `MAX_SNAPSHOT_DEPTH` (1024) so the pass stays linear;
- the result is `pair[]`, plus `unclosed` and `stray` in the `Outcome`.

The parser uses `pair[open]` to bind a group's body; the pair pass never builds
a tree.

### 2.4 Engines

**Descent** (`crates/deep-rt/src/parser.rs`, `src/deep/parser.rs`): the
`grammar!` macro emits one function per rule, and those functions call
`Parser`. Key operations, all O(1) except where noted:

| operation | what it does |
|:--|:--|
| `bump`/`eat`/`expect` | consume a significant token; `expect` records a missing node |
| `open`/`close`/`abandon` | push a `Start … Finish` event pair |
| `precede`/`wrap` | open a parent around an already-closed node (no event moves) |
| `trial`/`end_trial` | predicate parse: record error count, rewind |
| `group` | parse a balanced group with the body's limit at its closer |
| `checkpoint`/`rewind` | backtrack; cuts forward-parent links made after it |
| `separator` | `x % sep` list recovery |
| `nested` | recursion one level deeper, subject to `max_depth` |

`wrap` caches the outermost wrapper of the last operand, so a chain
`a + b + c + …` is O(1) per operator (`Parser::wrapped`). Events are flattened
to the arena once, after the rule returns; `precede` is a link, not a move.

**Tables** (`crates/deep-rt/src/driver.rs`, `recovery.rs`, `stack.rs`,
`version.rs`): an LR(1)/LALR(1) driver over the flat `deep_abi::Grammar`. The
stack is a `Vec` of `(state, node)` entries and becomes a graph (`Gss`) when
two versions share a prefix; versions are bounded. Recovery is insert /
skip-to / cascade-abandon (`deep_abi::Recovery`); errors and skipped runs are
adopted by the smallest reduction whose span contains them.

Which engine runs is a property of the language: a `Language` with `tables`
uses the driver, one without uses descent.

### 2.5 Budget

`Budget { steps_per_token, base_steps, max_depth, time }`, default
`64`, `4096`, `4096`, `Some(10 s)`. `Meter` counts parser steps and reads the
clock every `CLOCK_EVERY` (1024) steps. Running out never panics:

- steps or time: the tokens left become one `ERROR` leaf (`Event::Span`);
- depth past `max_depth`: a group becomes one `SKIPPED` leaf, in O(1);
- `Outcome::complete()` is `exhausted.is_none() && skipped_groups == 0`.

The fuzzer sets the clock to 2 s so a pathological input returns as a
measurable hang instead of spinning.

## 3. EXTRACTED versus inferred

The tree has two kinds of content, and the distinction matters when reading a
node you did not expect:

- **Extracted** — the node or token covers bytes. `start < end`, the lexer
  consumed those bytes through one step function, and the bytes are the only
  source of truth. Every ordinary token and every grammar node that wraps real
  tokens is extracted.
- **Inferred** — the parser or lexer inserted something with no bytes of its
  own:
  - `newline`, `indent`, `dedent` in a layout language (zero-width, derived
    from the line breaks and indentation of real tokens);
  - a `MISSING` `ERROR` node at a position where `expect` wanted a token
    (`start == end`);
  - `ERROR` and `SKIPPED` nodes made by recovery and by the budget; these are
    not derived facts about the source, they are statements that the parse
    could not place a range of tokens.

Inference is never silent and never changes a node's kind to make the tree look
valid: an unplaceable token is inside an `ERROR`, and an `ERROR`/`SKIPPED` node
is flagged, counted, and visible to `Outcome`. **Do not confuse this with the
`EXTRACTED`/`INFERRED` edge tags in `graph.json`.** Those are about symbol
resolution (an import states the edge, or a heuristic chose the target). The
parser has no heuristic resolution step; the only inferred tree content is the
zero-width layout and error structure listed above.

## 4. The arena

`deep::tree::Tree` is one `Vec<Node>` in preorder. A node is 16 bytes:

| field | type | meaning |
|:--|:--|:--|
| `kind` | `u16` | a `Kind`; below 16 is reserved (`ERROR`, `SKIPPED`, whitespace, comment, `UNKNOWN`, `EOF`, `TOMBSTONE`) |
| `flags` | `u16` | `HAS_ERROR` 1, `UNCLOSED` 2, `MISSING` 4, `HAS_SKIP` 8, `TOKEN` 16 |
| `start`, `end` | `u32` | byte range covered by the node's own tokens; the root spans `0..src.len()` |
| `subtree_end` | `u32` | index one past the last descendant |

Consequences a maintainer must keep true:

- node 0 is the root; its `subtree_end` equals `nodes.len()`; its kind is the
  language's root kind;
- a node's children are `index + 1 .. subtree_end`, and each child's range is
  inside its parent's span;
- siblings tile the parent's range with no gap and no overlap;
- a leaf has `subtree_end == index + 1`; a `TOKEN` node is a leaf;
- trivia (whitespace, comments) is not in the tree;
- no walk recurses, so a tree of depth `n` cannot overflow the stack (the
  `max_depth` budget bounds depth anyway).

`crates/deep-fuzz/src/treecheck.rs` implements exactly these rules with an
explicit stack, and the fuzzer runs it on every input. It is the executable
statement of the layout above; if the two disagree, this section is wrong.

The table engine builds a linked `deep_abi::Node` arena (20 bytes: kind, flags,
`start`, `end`, `first_child`, `next_sibling`) and `language_tables::flatten`
converts it into the preorder tree above, so callers never see two shapes.

## 5. Recovery rules

The contract is one sentence: **the root survives, and a broken construct costs
its own subtree, not the file.** The mechanisms:

| mechanism | where | behaviour |
|:--|:--|:--|
| missing token | `Parser::expect` / `missing` | zero-width `ERROR` with `MISSING`, parse continues |
| separator recovery | `Parser::separator`, `separator_trailing` | in `x % sep`, junk up to the next separator becomes one `ERROR`; a trailing separator becomes an error node |
| error run | `Parser::error_until(stop)` | wraps tokens in one `ERROR` until a `stop` token or the group's end; always consumes at least one token, so a loop cannot stall; balanced groups inside are taken whole so a `stop` inside brackets does not end the error early |
| group firewall | `Parser::group` | the body's visible limit is the paired closer (`pair[open]`), so no error inside can consume the closer or leak past it |
| unclosed group | `Parser::group` | an opener with no closer ends at the enclosing limit (or the line end in a layout language), gets a `UNCLOSED` flag, and its content stays inside the node |
| leftover tokens | `Parser::take_rest` | whatever a rule left before its limit becomes one `ERROR` |
| budget | `Parser::group` / `nested` / `take_rest` | depth past `max_depth` becomes one `SKIPPED` leaf; exhausted steps/time become one `ERROR` span. Both are O(1) |
| failed predicate | `Parser::trial` | a trial's first error ends its input, then `rewind` restores events, position and error count |
| table engine | `deep-rt/src/recovery.rs` | insert the expected terminal, fork a skip, or abandon the subtree at a matching bracket; errors/skips are adopted by the smallest containing reduction; versions bounded |

No recovery path can panic: every index is bounded by `tokens.len()`, the pair
table, or a group limit; recursion depth is bounded by `max_depth`; the lexer
cannot stall; the events array is truncated only at a recorded checkpoint.

The fuzzer checks the observable half of this: no panic, no hang over 2 s, no
tree outside the layout rules. The grammar tests check the semantic half: a
malformed fixture keeps its root, spans the file, and carries its error inside
the root (`crates/deep-grammars/tests/fixtures.rs`).

## 6. Incremental re-parse

`crates/deep-rt/src/incremental.rs`, entry point
`reparse(grammar, old_cst, old_len, &Edit, tokens, &Config) -> (Cst, Report)`.

The algorithm re-parses from the smallest old node covering the edit and splices
the rest:

1. **Prepare.** Walk the old tree in build order recording, per node, the LR
   state and stack height at its first byte, its token ordinals, and the old
   leaf stream. Find the smallest node whose span covers the edit.
2. **Replay.** Replay the prefix. If the token stream at the window no longer
   matches (an edit can delete a `DEDENT` *before* the window), widen to the
   parent and retry, up to 6 levels; the root (empty prefix) is the last resort.
3. **Run.** Parse the window from the recorded stack.
4. **Splice.** A suffix subtree is reused iff the new parse reaches the same
   byte (shifted by `delta`), the same top state and the same stack height, and
   its bytes are untouched. Then re-parsing those bytes would reproduce the
   node exactly, so the old subtree is copied with offsets shifted and pushed as
   one pre-reduced stack entry.
5. **Fallback.** Any inconsistency — or an old tree with error nodes — is a
   full parse (`Report::fell_back`). The result is then guaranteed identical to
   `parse`.

The differential test `crates/deep-rt/tests/incremental.rs` asserts the
incremental tree is byte-identical to a full parse on real Python files, edits
included, and (in a release build, where the timing assertion is not skipped)
that it is faster; that test is the contract, not the prose.

Limits, explicitly:

- **Error trees are not reused.** Missing/error nodes are inserted or adopted
  rather than shifted, so the recorded states would be guesses; `reparse`
  full-parses instead.
- **Tokenisation is not incremental.** The caller passes the whole new token
  stream. Lexing is linear and cheap, and a context-sensitive lexer (Python
  layout, mode stacks) cannot in general restart at a byte offset without
  replaying anyway.
- **Context changes widen the window.** An edit that changes how tokens before
  it reduce (indentation is the loud case: one dedent moves every later
  `DEDENT`) fails the state comparison and is rebuilt level by level.
- **It is the table engine's API.** `reparse` takes a `deep_abi::Grammar` and a
  `Cst`; the descent runtime's `grammar!` tree is not an input. Wiring the
  deeper engines together is a separate change, not a flag.
- **Memory is O(old nodes)** for the recorded states, heights and ordinals. The
  tree is not copied until the splice; the fallback allocates a new arena.

## 7. Adding a language

### 7.1 The `grammar!` route (what all 30 grammars here use)

1. **Write `crates/deep-grammars/src/<language>.rs`.** A `grammar! { NAME { … } }`
   block: `name`, `lex = <step fn>`, optional `layout = NEWLINE INDENT DEDENT`,
   `tokens { … }` with `Class::Open/Close` families implied by the delimiter
   pair, then rules; the first rule is the root. Write the hand lexer in the same
   file. The DSL is documented in `crates/rust-graph-deep/src/lib.rs`; what it
   cannot express is listed in `crates/deep-grammars/README.md` §"What the DSL
   could not express" (layout is runtime; no f-string fields; a choice sees one
   to three tokens; one kind per rule).
2. **Register it** in `crates/deep-grammars/src/lib.rs` (`pub mod <language>;`)
   and, for batches, in the batch `mod.rs`.
3. **Add fixtures**: `crates/deep-grammars/fixtures/<language>/` with real-shaped
   valid files and a `malformed/` (or `broken/`) directory. Add the expectation
   tables to the fixture test, naming every file: an unlisted fixture fails the
   test instead of being skipped.
4. **Add it to the fuzzer**: one `entry!(…)` line in
   `crates/deep-fuzz/src/registry.rs` with the extension and fixture
   directories. Then `deep-fuzz fuzz --lang <name> --seconds 30` must be clean.
5. **Run** `cargo test -p deep-grammars`, `cargo test -p deep-fuzz`, and
   `deep-conformance scan/summary/gates` over a corpus in that language.

### 7.2 The table route

Produce a `deep_abi::Grammar` (flat `action_index`/`action_table`,
`goto_index`/`goto_table`, `default_reduce`, `production_len`) for the language,
wrap it in a `Language` that carries it (`Language::tables`), and run
`deep-rt`'s driver tests. `crates/deep-tables` is the generator; `deep-rt`'s
`table.rs` documents the cell packing. This route exists for languages whose
grammar is generated, not for the `grammar!` set.

## 8. Running the gates

| gate | command | passes when |
|:--|:--|:--|
| grammars | `cargo test -p deep-grammars` | every fixture matches its expectation; lexers match |
| runtime | `cargo test -p deep-rt` | descent/GLR parity, recovery, incremental differential, tree invariants |
| grammar generation | `cargo test -p rust-graph-deep -p deep-tables` | the macro and table generator tests |
| fuzz | `cargo test -p deep-fuzz` | harness, shrinker and tree checker tests |
| fuzz run | `deep-fuzz corpus --dir crates/deep-grammars/fixtures` | every fixture passes the four checks |
| fuzz run | `deep-fuzz fuzz --lang all --seconds 30 [--seed N]` | 0 panics, 0 hangs, 0 unreachable trees, 0 memory breaches |
| conformance | `cargo run -p deep-conformance --features deep-grammars -- scan --root DIR --out report.json`, then `gates --report report.json` | clean rate ≥ 95 % per language (≥ 10 files), aggregate ≥ 15 MB/s, no file over 2 GiB RSS, cold start < 1 ms |
| speed | `cargo test --release --features parser --test parser_gate` | ≥ 2× Tree-sitter per corpus; needs a C compiler and network for `bench/build-tsdrv.sh` |

The conformance gate definitions and their exit codes are in
`crates/deep-conformance/README.md`; the speed gate's corpora and reproduction
steps are in the repo `README.md`.

## 9. Fuzzing

`crates/deep-fuzz` mutates seed fixtures deterministically and checks the four
properties in §5. It reads the fixtures of `crates/deep-grammars` plus its own
`corpus/seeds/` (deep nesting, unterminated strings, invalid UTF-8, NUL bytes,
delimiter soup, repeated tokens). Mutations are bit flips, truncation, range
deletion, delimiter/keyword insertion, token and range doubling, UTF-8 boundary
damage, and invalid UTF-8 insertion; all are pure functions of the PRNG state,
so `(seed, iteration)` names an input.

```bash
deep-fuzz fuzz   --lang all --seconds 30 --seed 20261108
deep-fuzz shrink --input crashes/<file>.bin [--lang NAME]
deep-fuzz corpus --dir crates/deep-grammars/fixtures
```

A failure is a panic, a parse over 2 s, peak RSS over 512 MB, or a tree the
checker rejects. Panics are caught with `catch_unwind`, so the fuzzer must be
built with unwinding panics (`cargo run -p deep-fuzz`, the default). A
`--release` build inherits the workspace's `panic = "abort"`; a panic then ends
the process after saving the current input, which is the same verdict with less
evidence.

Proof that the harness can fail: `crates/deep-fuzz/src/harness.rs` contains
`an_injected_broken_grammar_is_caught_through_the_real_runtime`, which builds a
real `Language` whose parse function panics and drives it through the real
workspace and lexer, plus targets that return a truncated `subtree_end` and a
node whose span escapes its parent. `crates/deep-fuzz/src/shrink.rs` reduces a
27-byte input to the exact 5-byte trigger in `shrink_reduces_to_the_trigger`,
and `shrink_drives_the_real_runtime_to_the_trigger_byte` builds a real
`Language` whose lexer panics on one byte and shrinks a 40-byte input to that
single byte through the workspace, the lexer and the parser.

## 10. Measurements

All numbers below are from this worktree on the development box, in a debug
build unless stated. "Inputs" is mutated inputs checked, not fixtures.

### 10.1 Fuzz run

Command:

```bash
cargo build -p deep-fuzz
for l in $(target/x86_64-unknown-linux-musl/debug/deep-fuzz languages | cut -f1); do
  target/x86_64-unknown-linux-musl/debug/deep-fuzz fuzz --lang "$l" --seconds 30 --seed 20261108
done
```

One process per language (so a crash would not stop the rest); `--lang all`
walks the same registry with the same per-language streams and produces the same
verdicts. The seed is folded with the language name
(`main.rs::mix`), so a language's stream depends on `--seed` and its name, not
on which other languages ran.

Seed fixtures: every valid and malformed fixture in
`crates/deep-grammars/fixtures/` for the language, up to 8 per language, plus
the 7 hostile seeds in `crates/deep-fuzz/corpus/seeds/`.

| language | inputs | panics | hangs | unreachable | memory | worst ms | max nodes |
|:--|--:|--:|--:|--:|--:|--:|--:|
| awk | 127,862 | 0 | 0 | 0 | 0 | 4 | 2843 |
| c | 71,528 | 0 | 0 | 0 | 0 | 45 | 1668 |
| clojure | 95,111 | 0 | 0 | 0 | 0 | 14 | 2152 |
| cpp | 33,874 | 0 | 0 | 0 | 0 | 52 | 2547 |
| csharp | 46,376 | 0 | 0 | 0 | 0 | 31 | 1685 |
| css | 70,057 | 0 | 0 | 0 | 0 | 22 | 2345 |
| elixir | 90,165 | 0 | 0 | 0 | 0 | 6 | 3449 |
| erlang | 117,882 | 0 | 0 | 0 | 0 | 10 | 2083 |
| go | 105,549 | 0 | 0 | 0 | 0 | 13 | 1694 |
| html | 279,644 | 0 | 0 | 0 | 0 | 9 | 377 |
| java | 69,643 | 0 | 0 | 0 | 0 | 36 | 2384 |
| javascript | 61,866 | 0 | 0 | 0 | 0 | 26 | 2431 |
| json | 85,215 | 0 | 0 | 0 | 0 | 36 | 788 |
| julia | 71,263 | 0 | 0 | 0 | 0 | 44 | 3067 |
| kotlin | 52,340 | 0 | 0 | 0 | 0 | 37 | 1669 |
| lua | 46,903 | 0 | 0 | 0 | 0 | 46 | 3330 |
| objc | 72,368 | 0 | 0 | 0 | 0 | 14 | 2553 |
| perl | 93,956 | 0 | 0 | 0 | 0 | 5 | 2710 |
| php | 108,853 | 0 | 0 | 0 | 0 | 20 | 2443 |
| python | 35,368 | 0 | 0 | 0 | 0 | 25 | 2537 |
| r | 76,866 | 0 | 0 | 0 | 0 | 65 | 2880 |
| ruby | 101,124 | 0 | 0 | 0 | 0 | 12 | 2876 |
| scala | 127,476 | 0 | 0 | 0 | 0 | 4 | 2436 |
| shell | 122,926 | 0 | 0 | 0 | 0 | 4 | 2317 |
| solidity | 134,192 | 0 | 0 | 0 | 0 | 4 | 1675 |
| sql | 87,063 | 0 | 0 | 0 | 0 | 60 | 1669 |
| swift | 42,448 | 0 | 0 | 0 | 0 | 31 | 2441 |
| tcl | 307,218 | 0 | 0 | 0 | 0 | 3 | 267 |
| typescript | 72,408 | 0 | 0 | 0 | 0 | 13 | 2491 |
| yaml | 143,383 | 0 | 0 | 0 | 0 | 15 | 1933 |
| zig | 63,094 | 0 | 0 | 0 | 0 | 31 | 2169 |
| **total (31)** | **3,014,021** | **0** | **0** | **0** | **0** | **65** | **3,449** |

Peak RSS for the whole run: 3.8 MB (the process peak; the cap is 512 MB).
The worst input took 65 ms; the parser's clock budget is 2 s, so the slowest
input was 3.3% of the hang threshold.

### 10.2 Corpus check

Command:

```bash
target/x86_64-unknown-linux-musl/debug/deep-fuzz corpus --dir crates/deep-grammars/fixtures
```

| measure | value |
|:--|--:|
| files checked | 233 |
| files skipped (unknown extension) | 0 |
| worst parse | 5 ms (debug build) |
| exit status | 0 |

### 10.3 Repository checks

| measure | command | value |
|:--|:--|--:|
| fuzzer tests | `cargo test -p deep-fuzz` | 30 tests: 26 unit and 4 integration, all pass |
| fixture files checked | `find crates/deep-grammars/fixtures -type f | wc -l` | 233 |
| fuzzable languages | `deep-fuzz languages | wc -l` | 31 |
| `deep-fuzz` debug binary | `ls -l target/x86_64-unknown-linux-musl/debug/deep-fuzz` | 15,454,688 bytes (14.7 MiB, debug) |

### 10.4 Numbers measured elsewhere

| number | where it comes from | command |
|:--|:--|:--|
| 8.7× / 8.6× faster than Tree-sitter (python-a / python-b) | repo `README.md` parser gate table, 2026-11-08 | `cargo test --release --features parser --test parser_gate` |
| 100.0 % clean on 200 Python stdlib files | `crates/deep-conformance/README.md` measured run | `deep-conformance scan/summary/gates` |
| 19.85 MB/s aggregate Python | same | same |

Those three are quoted, not re-measured here; they need a corpus, a release
build and (for the speed gate) a C compiler and network.
