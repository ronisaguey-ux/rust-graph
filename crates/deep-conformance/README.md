# deep-conformance

One command answers two questions about the deep CST parser: **does it parse
real code across the languages this project claims, and how fast** — and a second
command says whether a change made either answer worse.

```text
deep-conformance scan    --root DIR --out report.json
deep-conformance summary --report report.json
deep-conformance gates   --report report.json
deep-conformance diff    --a report.json --b report.json
deep-conformance languages
```

Exit codes: `0` pass, `1` a clean-rate floor, gate or diff regression failed,
`2` usage error or an unreadable report. Nothing else is written to the report
file; `summary`, `gates` and `diff` only read it.

## scan

Walks `--root`, and for every file whose extension maps to one of the project's
42 languages (`crates/deep-conformance/src/registry.rs`, taken from
`src/langs.rs`) writes:

| field | meaning |
|:--|:--|
| `path` | relative to the root, `/` separated |
| `language` | the project's language name |
| `bytes` | source size |
| `outcome` | `clean`, `errors`, `fatal`, `not_measured` or `unreadable` |
| `error_nodes` | ERROR-kind nodes in the tree |
| `error_bytes` | source bytes covered by top-level ERROR nodes (nested counted once) |
| `nodes` | nodes in the tree, root included |
| `max_depth` | greatest nesting depth |
| `ms` | wall time around the parser call, or `null` |
| `rss_bytes` | process peak RSS after the file (`VmHWM`) |
| `reason` | why a `fatal`/`unreadable` file has no numbers |

`ms` and `rss_bytes` are `null` when no parser ran. **A language with no
registered parser is `not_measured`, never `clean`.** A file that cannot be read
is `unreadable`; a parser that panics (in an unwind build) is `fatal`; none of
them stops the run. Symlinks are not followed and directories that cannot be
read increment `walk_errors`.

Cold start is measured from process entry to the scan being ready: binary load,
argument parsing and registry construction, with corpus I/O excluded.

## summary

Prints one row per language: files, measured files, clean files, clean rate with
its denominator, total error bytes, p50/p95/max parse time and nodes per byte.
It exits non-zero when a language with at least **10 files** is under **95%
clean** (nearest-rank percentile). A language at or above the file floor with no
measurement also fails: unmeasured is not clean.

## gates

| gate | definition |
|:--|:--|
| clean rate | every language with at least 10 files is at least 95% clean; the denominator is measured files only |
| speed | aggregate throughput (total measured bytes / total measured parse time) is at least 15 MB/s; the per-file median is printed beside it |
| RSS | no file leaves the process above 2 GiB peak RSS |
| cold start | process entry to scan-ready is under 1 ms |

Each line states the number that decided it (language and rate, MB/s, MB, ms),
and the process exits `1` if any gate fails. An unmeasurable gate — no timed
file, no RSS on the platform, no process age, no language at the file floor —
is a **FAIL**, not a pass.

## diff

Compares two reports per language and exits `1` on a regression:

- clean rate fell by more than 0.5 percentage points, or the language went from
  measured to not measured, or a language with at least 10 files disappeared;
- p50 parse time grew by more than 10%.

The table prints both sides of every number, so a change is judged by the
measurement rather than by memory.

## Wiring a parser in

The registry is the seam and the tool never guesses:

```rust
fn parse_rust(source: &[u8]) -> ParseFacts {
    // call the grammar, then return nodes, depth, error nodes and error bytes
}

let mut registry = Registry::new();
registry.set_parser("Rust", parse_rust);              // one parser for the row
registry.set_extension_parser("Rust", "rs", parse_rust); // only if needed
```

`ParseFacts::fatal("reason")` means no tree. A recovered tree with error nodes
is `ParseFacts::broken(nodes, depth, error_nodes, error_bytes)`. A tree with no
error node is `ParseFacts::clean(nodes, depth)`.

The `deep-grammars` feature does this for the three grammars in this worktree:

```bash
cargo run -p deep-conformance --features deep-grammars -- languages
```

That build reports `3 of 42 languages have a parser`; every other language is
`not measured`.

## A measured run

200 files copied from `/usr/lib/python3.12` (3,095,304 bytes), release build
with `--features deep-grammars`, on the project box:

```text
scanned 200 files (200 measured, 0 not measured, 0 skipped) in 180.2 ms -> report.json

language  files  measured  clean  clean rate               error bytes    p50 ms    p95 ms    max ms   nodes/B
Python      200       200    200  100.0% (200/200)                   0     0.464     2.682     5.078    0.1550

clean rate >= 95% per language (min 10 files)  PASS  Python: 100.0% clean (200/200), floor 95%
median parse >= 15 MB/s (aggregate)            PASS  aggregate 19.85 MB/s over 3095304 bytes (floor 15); per-file median 15.03 MB/s
no file over 2 GiB RSS                         PASS  2.3 MB peak after euc_kr.py (limit 2048.0 MB)
cold start < 1 ms                              PASS  0.068 ms process age when ready to parse
```

Appending an incomplete `def` to five of those files and diffing:

```text
Python  100.0% (200/200) -> 97.5% (195/200)  0.464 -> 0.743  1.60x
        REGRESSION: clean -2.5pp; REGRESSION: p50 1.60x slower
```

## Tests

`cargo test -p deep-conformance` runs unit tests for JSON, percentiles and the
report round trip, plus integration tests that build a synthetic corpus with
known-good and deliberately broken files:

- a read-protected file, a panicking parser and unknown extensions do not stop
  the scan;
- 9/10 clean prints `90.0% (9/10)` and fails summary and the clean gate;
- 19/20 clean prints `95.0% (19/20)` and passes;
- a parser slower than 15 MB/s fails the speed gate;
- a 3 GiB RSS record fails the RSS gate; a 2.5 ms cold start and an unmeasured
  cold start both fail;
- 12 Go files with no registered parser stay `not_measured`, produce no clean
  rate and fail the clean gate;
- a clean-rate drop, a speed regression, a vanished language and an identical
  report each produce the expected diff verdict.
