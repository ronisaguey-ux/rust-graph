# Changelog

All measured numbers below are from this worktree on 2026-11-08: Ubuntu 24.04.4
LTS, Intel Core i7-8550U, 8 cores, `x86_64-unknown-linux-musl`, rustc 1.99.1.
The commands that produced them are in `docs/DEEP_PARSER.md` §10.

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
