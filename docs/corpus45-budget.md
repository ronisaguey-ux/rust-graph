# Per-language parse budget

P6-13 sets the line at 3 ms per file at p95: a language over it is fixed before the phase closes.
This is the measurement, not an estimate — `tools/lang_budget.py` runs the release binary over a
200-file sample of each `corpus45` directory and reads the `micros` the binary already emits for
every source. The first parse of a run is dropped (it pays for the workspace allocation), and a
sample under 20 parsed files is reported as `sample` instead of being judged.

Measured on the box, release build, `x86_64-unknown-linux-musl`, sample 200, 2026-11-23.

| language | p50 µs | p95 µs | max µs | budget |
|---|---:|---:|---:|---|
| awk | 41 | 8063 | 41560 | OVER |
| clojure | 398 | 6412 | 101290 | OVER |
| cpp | 352 | 5493 | 20334 | OVER |
| crystal | 665 | 7090 | 30973 | OVER |
| csharp | 368 | 5297 | 13628 | OVER |
| d | 2482 | 20964 | 81164 | OVER |
| dart | 807 | 6096 | 13689 | OVER |
| elixir | 1640 | 7890 | 29978 | OVER |
| erlang | 368 | 6014 | 24082 | OVER |
| fsharp | 2170 | 18799 | 72766 | OVER |
| go | 143 | 7832 | 36017 | OVER |
| java | 540 | 4016 | 9118 | OVER |
| javascript | 1028 | 5798 | 16012 | OVER |
| julia | 1957 | 8698 | 23273 | OVER |
| nim | 1206 | 7088 | 16463 | OVER |
| objective-c | 404 | 6053 | 9813 | OVER |
| pascal | 2039 | 5070 | 40614 | OVER |
| perl | 589 | 9710 | 26566 | OVER |
| python | 810 | 6998 | 22227 | OVER |
| rust | 1094 | 8018 | 30874 | OVER |
| scala | 789 | 3601 | 13117 | OVER |
| sql | 918 | 7605 | 18814 | OVER |
| swift | 719 | 7577 | 22561 | OVER |
| vbnet | 391 | 14632 | 30398 | OVER |
| zig | 875 | 15581 | 47562 | OVER |
| assembly | 534 | 1680 | 17981 | ok |
| bash | 161 | 2334 | 113789 | ok |
| c | 59 | 1235 | 5220 | ok |
| cobol | 162 | 2715 | 12500 | ok |
| elm | 231 | 1833 | 8479 | ok |
| fortran | 336 | 1622 | 10006 | ok |
| groovy | 408 | 1911 | 2594 | ok |
| haskell | 32 | 972 | 4283 | ok |
| kotlin | 293 | 2195 | 6697 | ok |
| lua | 329 | 2972 | 7649 | ok |
| matlab | 245 | 992 | 11489 | ok |
| ocaml | 288 | 1887 | 8652 | ok |
| php | 164 | 1339 | 4340 | ok |
| powershell | 367 | 2154 | 8003 | ok |
| r | 337 | 2426 | 11671 | ok |
| ruby | 350 | 2203 | 5627 | ok |
| solidity | 134 | 867 | 3526 | ok |
| tcl | 317 | 2500 | 11212 | ok |
| typescript | 69 | 829 | 4627 | ok |

`visual-basic` sampled zero files (the corpus stores it under `vbnet`), so it is reported as a thin
sample rather than as a passing budget.

25 of 45 languages are over the line. The worst are `d` (21 ms), `fsharp` (18.8 ms), `zig` (15.6 ms)
and `vbnet` (14.6 ms) — all above four times the budget, which is a shape problem rather than a
constant factor.

Re-run with:

    python3 tools/lang_budget.py --corpus /tmp/opencode/corpus45 --sample 200

and `--check` to make the exit code the gate.

## Where the time goes (erlang, whole tree, 2026-11-23)

A whole-tree run of `corpus45/erlang` (4,335 files, release, `--no-cache`, per-file `micros` from the
JSON): p50 643 µs, p95 13,404 µs, max 5,458 µs…sec. So the tail is **not** the fallback path — one
file of the 4,335 was parsed by the deep reader, and every other file was the one-pass lexer. The p95
is the lexer's own cost on the corpus's few ~1 MB test files, and that cost is linear:

| file | bytes | µs/byte |
|---|---:|---:|
| median of the tree | — | 0.141 |
| `re_testoutput1_split_test.erl` | 1.82 MB | 0.81 (6× median) |
| `gl.erl` | 556 KB | 0.23 (2× median) |
| `jquery-latest.js` | 78 KB | **69.3 (490× median, 5.46 s)** |

Two findings follow, and they are different work:

1. **The gate is a constant factor, not a loss.** No loss-shaped fix moves a language from 13 ms to
   3 ms: the same one-pass reader costs 0.14 µs/byte for every language, so p95 falls when the reader
   gets faster, not when a language gets cleaner. That is the whole of P6-13's remaining distance.
2. **A pathological case has its own item, and it is quadratic**: the 78 KB jQuery build takes
   3.1–5.4 s. The raw deep parse of it is 104 ms (debug `deep-scan probe`), so the time is in the
   facts walk, not the grammar. Splitting the file in half gives 1.5 s for **both** halves
   (~0.75 s each): half the bytes cost a quarter of the time, which is the classic O(n²). 490× the
   median per byte on a small file is a bug, not a budget line — and because the same walk serves
   every language, fixing it moves p95 for all of them at once, which is what P6-13 actually needs.

### Fixed: the gate's quadratic clone (2026-11-23)

`uncovered` built its "found" pool — every event of the paired symbol plus every extra one — before
looking at `wanted`. For a module symbol that pool is the whole file (61,633 rows for a 78 KB jQuery
build), and it was rebuilt for each of the file's 103 symbol comparisons. Every one of those
comparisons had `wanted` empty, so the pool could not match anything: it was pure cost.

`if wanted.is_empty() { return Vec::new(); }` before the pool is built. Same file, same machine:

| | before | after |
|---|---:|---:|
| `jquery-latest.js`, default (both readers + guard) | 2,070 ms | 476 ms |
| guard only (`compare:` phase) | 1,919 ms | 240 ms |
| peak RSS | 22.2 MB | 7.3 MB |

Losses are byte-identical on `corpus45/erlang` (1,851 rows over 1,819 files, before and after), which
is what the early return promises: no want, no loss. Corpus times on a quiet machine: p50 365 µs,
p95 7.5 ms, 9.3 s for 4,335 files. Earlier percentiles in this document were measured while a test
run was compiling and are optimistic-looking by ~2×; these are the numbers to compare against.

## What the gate now needs: where the 0.1 µs/byte goes

Measured on `otp/lib/stdlib/src/ets.erl` (133,466 bytes, 38,311 tokens, the p95 file of the erlang
tree) with the token loop instrumented: **lexing is 38% of the parse and the statement/fact layer is
62%** (debug: 45.1 ms of 117.4 ms). Per token that is ~390 ns of total work in release, and the gate
(3 ms for this file) is ~78 ns — so P6-13 is a ~5× constant-factor job on both layers, not a
single bug:

| layer | share | note |
|---|---:|---|
| `Lexer::next` | 38% | one pass, no re-reading; the token itself is cheap |
| statements + facts | 62% | `Emit`, event strings, `format!` per value, frame bookkeeping |

The corpus p95 is not a pathological file at all: the file at p95 is 133 KB at 0.116 µs/byte, and the
density of files over 20 KB is p50 0.139 / p95 0.392 / p99 0.502 µs/byte. The tail is size × a
uniform constant, so every language closes at once when the constant falls.

### The base rate is ~100 ns/byte, or ~350 ns/token

Two synthetics separate the cost that scales with *bytes* from the cost that scales with *symbols*:

| synthetic | bytes | statements | time | ns/byte |
|---|---:|---:|---:|---:|
| one function, 16,000 expressions | 66 KB | 16,000 | 18 ms | 273 |
| 16,000 one-line functions | 362 KB | 16,000 | 138 ms | 381 |

Per statement those differ 8× (1.1 µs vs 8.6 µs), but per *byte* they are within 40% of each other,
and a real 133 KB source (`stdlib/src/ets.erl`, ~38,000 tokens) sits at 116 ns/byte — **faster** than
both synthetics. So the gate is not statements, symbols or constructs: it is the base rate of the
one-pass reader, ~350 ns per token, split 38% lexing / 62% statements (above).

The next lever is therefore inside a token, not above it. `perf` is blocked
(`kernel.perf_event_paranoid = 4`, `ptrace_scope = 1`) and `valgrind` is not installed, so the
sub-lexer split has to come from instrumentation in a debug build: time `skip_trivia`, the token
classifier and the token build separately, the same way the lex/statement split above was taken.

### Inside the lexer: trivia is a third of it

Instrumented on the same file (debug, 38,311 tokens; the timing calls themselves inflate the total):

| | ms | share of lex |
|---|---:|---:|
| `Lexer::next` total | 116.4 | 100% |
| `skip_trivia` inside it | 38.1 | 33% |
| classifier + token build | 78.3 | 67% |

So the remaining order of attack, by measured size, is: the statement/facts layer (62% of the parse),
then the token classifier (about 26%), then trivia (about 13%). Trivia alone cannot move the gate,
which is why the next change belongs in the statement layer or the classifier, not in whitespace
skipping.

## The scoreboard after the 2026-11-23 fixes

Re-taken on a quiet box (the runaway lane processes were killed first: eight `rust-graph build`
runs had been burning every core for 6-13 hours, all of them stale binaries with an Objective-C hang
that is already fixed - the same corpora parse in 5 s with the current one).

**26 of 44 judged languages are over the 3 ms line** (visual-basic has no sample). Under it: c, cobol,
kotlin, php, powershell, ruby, r, tcl, typescript, elm, groovy, haskell, matlab, ocaml, solidity,
fortran, assembly, bash. Over it: d 20.3, fsharp 18.3, zig 15.9, vbnet 14.9, perl 9.7, julia 8.6,
rust 8.2, awk 8.2, elixir 8.1, swift 7.9, sql 7.9, go 7.7, erlang 7.4, nim 7.4, python 7.1,
crystal 6.6, dart 6.1, objective-c 6.1, clojure 5.9, javascript 5.9, cpp 5.5, csharp 5.4, pascal 5.0,
java 4.1, scala 3.7, lua 3.1 (ms, p95).

The tail is **size × the base rate**, not a pathological construct: awk's five slowest files are its
biggest (273 KB at 0.056 µs/byte, 258 KB of `.awk` at 0.057), and every language's p95 sits at
0.05-0.12 µs/byte. P6-13 is therefore one global constant-factor problem, and it closes for many
languages at once when the rate falls.

### Where a statement's time goes: calls are 7× a bare value

One expression per statement is a fair unit. 16,000 single-expression statements, `--lexer`:

| statement | time | per statement |
|---|---:|---:|
| a number | 8 ms | 0.50 µs |
| an atom | 18 ms | 1.13 µs |
| an assignment | 24 ms | 1.50 µs |
| a call (`f(1)`) | 60 ms | 3.75 µs |

A call costs 7.5× a constant, and calls are what a real source is made of (`ets.erl` is ~2,000 of
them ≈ 7.5 ms of its 9 ms). The per-call work is `word()` returning a `String` (an allocation), the
`Call.path` (a second), then `format!("[\"c\",{id},0,[{args}]]")` (a third, plus formatting). That is
the next place to look, and it is inside the 62% statement share rather than the 38% lexer share.

### Where the parse actually spends its time (measured, not guessed)

`perf` is unusable on this machine (`kernel.perf_event_paranoid=4`, `ptrace_scope=1`) and valgrind is
not installed, so the binary samples itself. `RG_SAMPLE=1` (or a path) turns the sampler on: a thread
reads a global "current function" tag every 200 µs and rewrites a table of counts, every 100 ms, to
`$RG_SAMPLE` or `/tmp/opencode/rg-profile.txt`. The tag is restored when the function returns, which
matters — without the restore every sample after a nested call was credited to the child, and the
parent (the statement preamble) looked five times cheaper than it is. With the sampler off, each
call site costs one relaxed load and the parse is the tree's own.

```
RG_SAMPLE=1 rust-graph build corpus45/erlang --no-cache     # then read the table
```

4335 files / 423 MB / 15.0 s on a quiet box:

| tag | share of the run | share of the parse |
|---|---:|---:|
| pipeline (read, sha256, facts → json) | 37.6 % | — |
| `next_token` (the lexer) | 18.2 % | 29 % |
| `chain` (postfix loop in the value walk) | 12.5 % | 20 % |
| `mix` (one value, combined) | 9.3 % | 15 % |
| `lexeme` (a word, as a `String`) | 9.0 % | 14 % |
| `ident` | 4.5 % | 7 % |
| `end_statement` bookkeeping | 3.4 % | 5 % |
| `parse` body (statement loop) | 3.7 % | 6 % |
| `list`, `values`, `argument`, `event`, `with_emit` | 2.0 % | 3 % |

The pipeline share does not enter the budget: `note_micros` times `parse_source` only.

Allocation traffic in the same run, counted with a temporary counter in the pool: **53.2 M blocks,
4.4 GB** over 4335 files — about one allocation per token, which is `lexeme` (a fresh `String` per
word, with a UTF-8 validation pass) plus `Call.path`. The counter is not in the tree: the sampler is
the tool that ships, and a counter is three lines when a run needs one.

So there is no single hot spot left. The 3-5× cannot come from one fix: the value walk
(`chain` + `mix` + `lexeme` + `ident`) is 56 % of the parse and is token-at-a-time with a `String`
per word, and the lexer is the other 29 %. That is a rewrite of both, in steps, with the sampler
pointing at each step.

The languages that cannot reach the line under this constant went to `docs/HANDOFF-ARGON.md` (P6-15),
one row each: p95, the factor it needs, the construct that blocks it, what has already been measured
and reverted, and the code the next attempt has to touch. No row there is an estimate.
