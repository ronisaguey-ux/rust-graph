# The CST reader's budget

What the deep reader promises, and the number each promise is measured by. The
guard's own cost lives in `MEASURE.md`; this file is about the parser alone.

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
