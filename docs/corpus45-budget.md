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
