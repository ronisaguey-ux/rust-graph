# Unifying the two runtimes

The deep parser shipped two runtimes:

- **A** — the hand-written one in `rust-graph`'s `src/deep`: `syntax`, `lexer`,
  `pairs`, `parser`, `tree`, `budget`. Every `grammar!` grammar is compiled against
  it by path, through `crates/deep-grammars/src/deep.rs`.
- **B** — `crates/deep-rt`: `deep_abi` tables, an LR/GLR driver, a linked arena,
  cascade recovery, a bounded version set.

One parser needs one runtime. The decision below is written down first, then the
tables that justify it, then what was moved, then the tests, then the exact
follow-up.

## The decision

**B's crate is the home; A's grammar-facing surface is ported into it verbatim;
A is then deleted.** Concretely:

- `crates/deep-rt` now owns all six modules of A (`syntax`, `lexer`, `pairs`,
  `parser`, `tree`, `budget`), plus A's hand-written JSON grammar (`json`).
- `crates/deep-rt::Workspace::parse(&[u8], &Language, &Budget) -> Result<Outcome,
  ParseError>` is the one entry point. It lexes, pairs, then picks an engine from
  the language: a language with `Language::tables` runs **B's** GLR driver; a
  language without them (every `grammar!` language) runs **A's** descent driver,
  which is unchanged code now living in `deep-rt`.
- The tree type is **A's** flat preorder `Vec<Node>` with `subtree_end` (16 B a
  node, no child pointers). B's linked arena is an internal build product and is
  flattened into that tree, so there is exactly one tree a caller can see.
- `Parsed`: the brief's suggested `Result<Parsed, Fatal>` would move the tree out
  of the reusable workspace on every file. The runtime keeps the tree in the
  workspace and returns the `Outcome` (`Result<Outcome, ParseError>`); a caller
  reads `workspace.tree` exactly as it does today. This is also what the shipped
  `grammar!` code and its tests already call, so no grammar changes for this.

Why not *one algorithm*: a `grammar!` grammar is a set of recursive descent
functions. LR(1) tables cannot be derived from that without a second generator,
and the runtime cannot invent one at parse time. Merging the two algorithms would
mean rewriting 2,000 lines of shipped grammar into an LR grammar — a different
project, with a different tree. So the two engines stay, and what is unified is
everything a grammar or a caller can see: one crate, one lexer, one kind table,
one pair pass, one tree, one budget, one entry point.

## What each runtime provides

| Concern | A: `src/deep` | B: `crates/deep-rt` before | Result after unification |
|---|---|---|---|
| Lexing | `lexer::Cursor` + `Tokens` (struct-of-arrays, lossless, trivia included); a step function per language | `deep_abi::Lexer` (`next_token(state) -> Option<Token>`), driven by the parser | A's `lexer`, and a thin `Lexer` adapter (`language_tables::TokenStream`) over the same tokens for the GLR driver |
| Layout (NEWLINE/INDENT/DEDENT) | `syntax::Layout` + `lexer::lex_layout`, one pass, zero-width tokens, resets | none | A's, ported in full; tested on its own in `tests/layout.rs` |
| Kind table | `syntax::{Kind, KindInfo, Class, Language}` + reserved kinds (`ERROR`, `SKIPPED`, `WHITESPACE`, `COMMENT`, `UNKNOWN`, `EOF`, `TOMBSTONE`) | none: symbols are plain `u16` from the generated tables | A's, plus `Language::tables` so one type can carry both shapes |
| Delimiters | `pairs::Pairs`: one linear sweep, `SEARCH_WINDOW = 8`, `#if/#else` snapshots, `unclosed`/`stray` counts | bracket pairs only as a recovery hint (`Config::pairs` / source scan) | A's pair pass always runs; its pairs are also what `Config::pairs` is filled from |
| Parser | `parser::Parser`: recursive descent with events, `precede`, `wrap`, `trial`, group firewall, `max_depth` | `driver::parse`: LR/GLR worklist over a graph-structured stack, bounded versions | both, chosen by `Language::tables`; one `Workspace` owns the state of each |
| Tree | `tree::Tree`: preorder `Vec<Node>`, `subtree_end`, `TOKEN`/`HAS_ERROR`/`MISSING`/`UNCLOSED`/`HAS_SKIP` flags | `cst::Arena`: linked child/sibling `deep_abi::Node`, 20 B | A's is the only public tree; B's arena is flattened by `language_tables::flatten` |
| Recovery | grammar-generated: `missing()`, `error_until()`, `separator()`, group firewall, `SKIPPED` leaf past `max_depth` | insert / skip-to / cascade-abandon, with forks and a version cap | both, each on its own engine. The shared invariant is the one that matters: the root survives and an error stays inside the node that broke |
| Budget | `budget::{Budget, Meter, Outcome}`, steps + depth + clock | `Config::max_steps`, a hard work-unit ceiling | A's `Budget` is the public one; B's ceiling is derived from it |
| Output | `Tree::sexp`, `Tree::write_json`, `push_json_string` | none | A's, unchanged |
| Language registry | `deep::language_for(path)` by extension | none | stays in the root package (it needs all the language statics); see follow-up |

## What the grammars actually call

`crates/rust-graph-deep/src/emit.rs` writes the calls; this is the whole surface a
`grammar!` grammar can touch, and every item of it now exists in `deep-rt` with the
same signature and the same behaviour.

| Called | Emitted at | Provided by |
|---|---|---|
| `rt::syntax::{Kind, KindInfo, Class, Language, Layout, FIRST_LANGUAGE_KIND}` | `emit.rs:98,104,115,132,133,137` | `syntax` |
| `rt::parser::Parser` | `emit.rs:137` | `parser` |
| `Language::new(name, root, kinds, lex, parse)` / `.with_layout(&Layout)` | `emit.rs:115` | `syntax` |
| `Parser::{current, at, at_any, at_end, nth, bump, eat, expect, missing, error_one, error_until, separator, separator_trailing, open, close, group, nested, precede, wrap, event_count, checkpoint, rewind, abandon, trial, end_trial, errors, exhausted}` | `emit.rs` (`p.` calls, 17 to 22 uses each) | `parser` |
| `Cursor` (`peek`, `peek_at`, `behind`, `since`, `rest`, `starts_with`, `bump`, `bump_n`, `bump_char`, `eat`, `eat_while`, `quoted`, `pos`, `at_end`, `previous`) | `python.rs:30`, `javascript.rs:50` | `lexer` |
| `syntax::{WHITESPACE, COMMENT, UNKNOWN}` | `python.rs:31`, `javascript.rs:51` | `syntax` |
| `Workspace::parse(&src, &Language, &Budget)` | `tests/fixtures.rs:19`, `tests/lexers.rs:9` | `parser` |
| `tree::{HAS_ERROR, TOKEN}` and `tree::Node` fields | `tests/fixtures.rs:18` | `tree` |

Nothing else in the grammar crate names the runtime: no module path other than
`crate::deep`. That is what makes the switch mechanical.

## What was ported

1. **The six modules** — `budget`, `lexer`, `pairs`, `syntax`, `parser`, `tree` —
   moved with no behavioural change. Only `super::` became `crate::`, and the one
   error type `TooLarge` became `parser::ParseError` (which also carries the GLR
   driver's `deep_abi::Fatal` reasons). The descent body of `Workspace::parse` is
   the old body moved into `Workspace::parse_descent`.
2. **Layout tokens**, inferred by the runtime (`lexer::lex_layout`): a zero-width
   `newline` after the last token of a logical line, `indent` before the first
   token of a deeper line, `dedent` immediately after the `newline` that ends a
   block, `dedent`s for every level at end of file, none inside brackets or on
   blank/comment-only lines, tabs to the next multiple of 8, form feed back to 0,
   and a `resets` token at the start of a line ending the bracket. `indent`/`dedent`
   are an `Open`/`Close` pair of one family, so `Parser::group` jumps a block whole
   with the same firewall as `{...}`.
3. **A hand-written grammar path** (`deep_rt::json`): JSON's lexer, kind table and
   descent parser, with no tables and no macro, so a caller with no generator still
   parses a real language through the same `Workspace::parse`.
4. **The GLR bridge** (`language_tables`): `TableGrammar` (tables + `Kind`↔symbol
   maps + bracket pairs), a `Lexer` adapter over the tokens the runtime already
   lexed, and `flatten`, which turns the arena into the one public `Tree`
   (preorder, `subtree_end`, flags, `TOKEN` inferred from "childless with bytes",
   runtime `ERROR` kind preserved rather than mapped through the language).

## Tests

`cargo test -p deep-rt` — 43 tests, all green:

| File | What it proves |
|---|---|
| `tests/grammar_parity.rs` (5) | The shipped `python.rs`, `javascript.rs`, `typescript.rs` compiled **unmodified** against `deep-rt`, and all **43 fixtures** (10 python, 10 javascript, 11 typescript, 12 malformed) parsed by both runtimes. Compared on four renderings, all byte-for-byte equal: the `Outcome` description, the named S-expression, the raw JSON tree (kinds, ranges, flags, token text), and every token by kind and byte range. |
| `tests/layout.rs` (9) | NEWLINE/INDENT/DEDENT inferred by the runtime, on a hand-written four-line language: placement, blank and comment-only lines, brackets, resets, tabs, form feed, bad dedent, block pairing, 300 nested levels, lossless tokens. |
| `tests/json_reference.rs` (8) | The hand-written JSON path: the five #5910 trees `src/deep/tests.rs` pins, nesting, comments, bad tokens, missing closer, missing value, budget exhaustion, plus the runtime's structural invariants. |
| `tests/table_language.rs` (7) | A `Language` carrying `TableGrammar` parses on the GLR engine through `Workspace::parse`; both engines see the same tokens in the same order; a broken item keeps a root over the whole file; a kind outside the grammar becomes an `ERROR` node, never a gap; the step ceiling reports exhaustion. |
| `tests/{tree,depth,recovery,branches}.rs` (13) | The existing GLR tests, unchanged. |
| `src/lib.rs` unit test (1) | The ABI layout is still pinned. |

## Differences, measured and expected

These are engine differences, not port differences. Every one is covered by a test.

| Difference | Why | Where it is asserted |
|---|---|---|
| Node shape for a table grammar | The driver shifts a node per terminal and reduces one per production; the descent parser emits a leaf only where a grammar says `bump` | `table_language.rs`: the leaves agree, the internal nodes do not |
| `Outcome::steps` | The driver counts shifts + reductions + versions; the descent parser counts token looks | `language_tables` docs; `table_language.rs` |
| `Outcome::exhausted` | The driver has no clock; exhaustion means its work units reached the ceiling derived from `Budget` | `table_language.rs::the_step_ceiling_is_reported_as_exhaustion` |
| `Outcome::skipped_groups` | Always 0 for a table grammar: the driver's cascades are counted as `Stats::cascades` instead | documented in `language_tables` |
| `TOKEN` on a table node | Inferred as "childless with a non-empty range", since the arena does not record which nodes came from the lexer | `table_language.rs::the_shape_of_a_terminal_is_a_leaf_with_its_range` |
| A `Kind` the tables do not name | It is still handed to the driver, which has no action for it and recovers, so it becomes an `ERROR` node rather than an unaccounted byte | `table_language.rs::a_kind_outside_the_grammar_becomes_an_error_node_not_a_gap` |

There is no difference in the *descent* engine: the 43-fixture comparison is
byte-identical, which is the point of the port.

## Follow-up: the exact changes in `crates/deep-grammars`

Both routes keep the grammar sources untouched. The recommended one is two edits.

### Recommended: keep `crate::deep` as a re-export shim (2 files)

1. `crates/deep-grammars/Cargo.toml:9` — add
   `deep-rt = { path = "../deep-rt" }` beside `rust-graph-deep`.
2. `crates/deep-grammars/src/deep.rs:1-29` — delete the six `#[path = "..."]`
   includes and their `pub use`s, and re-export instead:

   ```rust
   pub use deep_rt::{budget, lexer, pairs, parser, syntax, tree};
   pub use deep_rt::{Budget, ParseError, Workspace};
   ```

   (Keep the module doc, updated: the runtime is `deep-rt`, not a copy of
   `src/deep`.)

Nothing else changes. Every `crate::deep::...` path in the grammar modules and
every `deep_grammars::deep::...` path in the tests resolves through the shim:

- `src/python.rs:30`, `src/python.rs:31`
- `src/javascript.rs:50`, `src/javascript.rs:51`, `src/javascript.rs:621`
- `src/lib.rs:9-18` — doc comment only, says the runtime is compiled by path
- `tests/fixtures.rs:17`, `:18`, `:19`
- `tests/lexers.rs:8`, `:9`

### If the shim is dropped instead (7 sites)

Replace `crate::deep` with `deep_rt` at:
`src/python.rs:30`, `src/python.rs:31`, `src/javascript.rs:50`,
`src/javascript.rs:51`, `src/javascript.rs:621`, and `tests/fixtures.rs:17,18,19`
plus `tests/lexers.rs:8,9` as `deep_rt::` paths. The `grammar!` default runtime
(`crates/rust-graph-deep/src/read.rs:137`, `runtime: "crate::deep"`) would then
need `runtime = deep_rt;` in each of the three grammar invocations:
`src/python.rs:33`, `src/javascript.rs:53`.

Recommendation: the shim. It is one file, and it keeps the macro's default valid.

### Deleting `src/deep` (after the two edits above)

- Delete `src/deep/` (15 files, about 8,000 lines).
- `src/deep_integration.rs:25,27,105,106,107` reference `crate::deep`; that file is
  not declared in `src/main.rs` and is dead. Either delete it with `src/deep` or
  repoint it at `deep_rt`.
- `Cargo.toml:13` (`deep = ["dep:deep-rt", "dep:deep-lex", "dep:deep-grammars"]`)
  already names `deep-rt`, so no change; `src/deep` was never a crate.
- Keep `deep::language_for(path)` (extension → `&Language`) somewhere: it is the
  only thing in `src/deep/mod.rs` that is not a runtime module. Move it next to
  the language statics in the package that owns them (`crates/rust-graph-deep` is
  the natural home, since it also names the languages).

## Ground rules for the future

- Do not add a second tree type, a second lexer or a second budget. New work goes
  into the existing module.
- A new engine is a new `Language` field plus a new arm in `Workspace::parse`, in
  this crate — not a new runtime crate.
- Anything a grammar can see is frozen by `grammar_parity.rs`: if a change alters
  one token, one byte range or one flag of a fixture tree, it is a breaking change
  to every grammar and must be argued for, not merged.
