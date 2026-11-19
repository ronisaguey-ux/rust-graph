# Kotlin campaign — corrected record (2026-11-19, 02:00–04:00)

Everything in this section is measured on `/tmp/opencode/sweep-local/kotlin` (7,079 `.kt`
files) with a release build of `--features 'deep parser'`, using `tools/loss_count.py`
(release binary, `RG_LOSSES=1 RG_AUDIT=1`).

| point | files with a loss | loss rows |
|---|---|---|
| 687b3b8 (true baseline, reproduces) | **144** | **160** |
| + multi-dollar strings (`crates/deep-grammars/src/batch_a/kotlin.rs`) | 142 | 158 |
| + trailing-operator continuation (`src/parser.rs`) | **141** | **152** |

`js`, `ts`, `python` and `rust` measure 0 files / 0 rows with the same binary.

## Two traps found the hard way

1. **The `val x = call()` preparser fix is a regression, not a fix.** Letting
   `kotlin_declares_only` heads with a top-level `=` fall through to the assignment path
   took the corpus from 144/160 to **803 files / 1374 rows**: `typealias F = (String) -> Int`,
   `fun f() = e`, `val x: T get() = e` and `val x = lazy { … }` all became invented
   assignments. Reverted. Any lane measuring "before" on a binary that already contains it
   is measuring the regression, not the baseline — two lanes did exactly that and reported
   −120 and −168 deltas that do not exist on the real baseline.
2. **The typealias and getter/destructuring statement guards are no-ops on the true
   baseline.** They only remove rows that the regression above created. Kept out.

## What actually moves kotlin

- **Multi-dollar strings** (`$$"""…"""`, `$$"…"`): Kotlin 2.2 spelled them as a stray `$`
  plus a string that never opened, so every call after one was lost. The flagship repro
  `ktor/…/YamlConfigTestJvm.kt` (`decodeFromString<YamlMap>`) is clean now (0 loss rows).
- **A trailing binary operator continues the statement** (`Pattern.UNIX_LINES or` /
  `Pattern.CASE_INSENSITIVE or` / …, 93 of the 158 rows were this shape). Implemented in
  `on_newline`; `import` statements are excluded because `…/and` and `…/or` are package
  segments (excluding them fixed 5 files the first version broke).
- **Parameter defaults: do not do this naively.** Emitting an expression for every
  `param: T = expr` in a declaration header measured 802 files / 2303 rows — the types and
  defaults are already counted elsewhere. Reverted.

## Remaining shape (141 files / 152 rows)

87 `event`, 55 `call`, 10 `symbol`. The biggest event family is `m:[n:override,n:val …]`
(4 rows) — a declaration with modifiers where the CST writes a value merge and the lexer
writes nothing. The `call` rows are mostly one-per-file (`emptyList`, `assertTrue`,
`arrayOf`, `LoggerFactory.getLogger`).

## Lane discipline that worked

- One worktree + branch per lane, all based on `687b3b8`; a shared
  `CARGO_TARGET_DIR=/home/roni/Roni_workspace/rust-graph-kt/.cargo-target` so three lanes
  serialise on one build instead of thrashing eight cores.
- A lane's report is only accepted with: its own binary, the full-corpus before/after, and
  the reproduction command. A lane that reported a delta against the night3 binary was
  measuring the regression and was wrong.
