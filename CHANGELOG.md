# Changelog

All measured numbers below are from this worktree on 2026-11-08 and 2026-11-09:
Ubuntu 24.04.4 LTS, Intel Core i7-8550U, 8 cores, `x86_64-unknown-linux-musl`,
rustc 1.99.1. The commands that produced them are in `docs/DEEP_PARSER.md`
§10 and §11.

## [0.2.2] - 2026-11-09

### Fixed

- **A chained call keeps its receiver's arguments.** `Path(path).resolve()`
  wrote the receiver as `["c",0,0,[]]` — the call without its arguments — so a
  reader had the outer call and a receiver that looked empty. The receiver is
  now the inner call's own form. Measured on the Felicity corpus: the lost-call
  fallbacks on Go's chain-heavy files fell from 57 to 23, and the Python shape
  `A(x).b()` stopped producing a difference at all.
- **Python keyword arguments carry their value.** `f(default=True)` wrote `0`
  for every argument after the first: the grammar spells the `=` as a child of
  the keyword argument, and `True`, `False` and `None` are keywords to the
  parser while the one-pass lexer reads them as names. Both are read now, so
  the argument, its name and its value reach the record.
- **A bare annotation is an event.** `rule_id: str` in a class body binds
  nothing and produced no record, while the lexer files an event for it; every
  dataclass-shaped class lost that event and forced a fallback.
- **`fn default()` has a name.** `default` is a contextual keyword in Rust and
  the definition came out anonymous.
- **A method is paired by its owner, and its binders compare as a set.** Every
  class has an `__init__`, so the guard compared one class's events against
  another's and blamed the file for an event that was in the record. Binder
  lists also compared as raw text, which the languages spell differently
  (`["name","incoming"]` against `["incoming","name"]`).
- **A pattern is not a call.** `match x { Some(t) => … }` and
  `if let Some(t) = …` made the lexer file a call to `Some`; the CST records a
  pattern. The line settles it, and the difference is counted as its own class
  instead of forcing a fallback.

- **A name the source does not declare is not a loss.** `pub(crate) struct
  Meter` made the lexer file a symbol named `pub`, and the guard read the line
  as declaring it because it also declares `Meter`. A claimed name now has to
  follow its declaration keyword or be written before an `=`.

## [0.2.1] - 2026-11-09

### Fixed

- **An f-string's `{...}` field is code, not a string.** The Python grammar now
  opens a field mode with its own brace depth, so `f"{len(x)}"` contains a
  `call` node, `f"{ {'a': 1}['a'] }"` closes at its own brace, `!r` and `:>{w}`
  specs parse, a nested f-string restores the outer quote, and `f"{}"` is an
  error exactly as Python says. A broken field abandons the whole string with
  one `pop FSTRING` unwind instead of swallowing the rest of the file.
- **Template substitutions in JavaScript, TypeScript, TSX** get the same
  treatment: `${f(x)}` is an expression, a nested template keeps its own frames,
  a tagged template is a call with the literal as its argument.
- **`x: T = f()` lost its call**: the walk had no case for an annotated
  assignment, so the value was never read.
- **`x = lambda k: f(k)` lost its body**: the grammar writes the binding as a
  definition node and nothing read the lambda inside it.
- **`assert a(x), b(y)` dropped everything after the comma**; it now reads every
  value-bearing child, as the lexer does.
- **The guard compared event counts, not values.** A record whose `return` lost
  a value could still replace the lexer's. It now compares the shape of each
  event (targets and the names/calls/constants in the value) and keeps the
  lexer's record when anything differs. It also stops counting the lexer's own
  artefacts (`.call`, `assert` read as a call) as losses, and compares the class
  bindings.
- **Two Python grammar copies drifted**: `src/deep/python.rs` was a stale fork of
  the crate's grammar, so the facts path and the harnesses disagreed. One file
  is compiled by both now, and the same is true for JavaScript.

### Added

- **Fact tables for Rust, Go and Java**, alongside the existing Python and
  JavaScript/TypeScript ones. `lang_for` answers for `RS`, `GO` and `JAVA`, so
  those files can take the CST path; JSON deliberately stays out.
- `--compare`: parses every file both ways and prints symbols, calls and imports
  per side without choosing. The per-file choice is now named in the JSON
  (`path_choice`) and counted (`paths`), so a tree's split is auditable.
- Kotlin, Scala, Swift and Dart corpora: every sample parses clean, with
  interpolation parsed as expressions.
- C and C++: 84 → 185 and 92 → 169 clean files on a 331-file real corpus.
- PHP: 436/1553 real files clean → 5048/5053 across eight projects.
- Ruby: interpolation fields, command calls and heredocs.

### Measured

| tree | files | calls deep / lexer | imports deep / lexer |
|:--|--:|--:|--:|
| Felicity (Python) | 68 | 5638 / 5547 | 710 / 710 |
| rust-graph itself (Rust) | 29 | 7841 / 7589 | 304 / 295 |
| webchat_harness runtime (JS) | 11 | 547 / 541 | — |

Grammar corpora after this release: PHP 5048/5053 real files clean, shell
297/300 (two of the three are not shell), C 185/229, C++ 169/249, CSS 250/256,
HTML 336/383, Kotlin and Scala 8/8 each on their fixtures, Swift 9/9 and Dart
8/8 on their corpora.

Suite: 1442 tests with `--features "deep parser"`, 488 without. Deep release
binary 2,796,192 B; default build unchanged.

## [0.2.0] - 2026-11-08

### Added

- The deep parser behind `--features "deep parser"`: a lossless CST parser with
  error nodes, recovery and a step/time/depth budget. `rust-graph build` gains
  `--deep` (force the CST), `--lexer` (force the one-pass lexer) and an
  automatic mode that uses the CST where a grammar exists.
- `rust-graph-deep-scan` (`--features parser`): `tree`, `sexp`, `probe`,
  `bench` and `compare`, for inspecting a tree or timing a corpus.
- `crates/deep-grammars`: 43 `grammar!` blocks with hand lexers and fixtures,
  covering 40 of the project's 42 languages in that crate. The shipped
  `--deep` path wires 6 of the 42 languages (Python, Rust, JavaScript,
  TypeScript, Go, Java) plus JSON; every other language falls back to the
  one-pass lexer and the fallback is reported.
- `deep-conformance`: scan, summary, gates and diff over a corpus.
- `deep-fuzz`: deterministic mutation fuzzer, shrinker and tree checker.
- `crates/deep-bench`: corpus verification, the alternating 2× comparison and
  peak-RSS measurement.
- `docs/DEEP_PARSER.md` (design, recovery, incremental re-parse, measurements)
  and `docs/RELEASE.md` (the release gate sequence).

### Measured

| gate | value |
|:--|--:|
| `cargo test --workspace` | 416 passed, 0 failed, 19 ignored, 86.15 s |
| `cargo test --workspace --features "deep parser"` | 718 passed, 0 failed, 20 ignored, 110.68 s |
| default release binary | 837,496 bytes, unchanged sha256 |
| release binary with `deep parser` | 1,577,280 bytes (1.88×) |
| `rust-graph-deep-scan` release | 2,089,096 bytes |
| graph build, Python stdlib, `--lexer` | 2.10 s median, 7,940 kB peak RSS |
| graph build, Python stdlib, `--deep` | 4.03 s median, 7,888 kB peak RSS (1.92× wall) |
| one 1 MiB Python file, deep parse + extract | 258.6 ms vs 230.6 ms lexer; 29,428 kB vs 4,096 kB peak |
| deep scan, 568 files / 10.6 MB | 349.1 ms, 30.4 MB/s, 568/568 clean, 14,004 kB peak RSS |
| fuzz, 6 languages, seed 20261108 | 320,822 inputs, 0 panics, 0 hangs, 0 bad trees, 0 over 512 MB |
| conformance, Python 3.12 stdlib | 568/568 clean (100.0 %), 18.71 MB/s aggregate, cold start 0.066 ms |

### Not measured

- The 2× Tree-sitter gate. Tree-sitter is not installed on this machine and
  there is no C compiler (`cc`, `gcc`, `clang`, `pkg-config` all missing), so
  `bench/build-tsdrv.sh` cannot run and `crates/deep-bench compare` exits `2`
  without a ratio. No ratio is claimed in this release.

### Changed

- The crate version is now `0.2.0`. The default build is byte-identical with
  the features off.
