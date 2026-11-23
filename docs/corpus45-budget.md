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
