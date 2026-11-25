# FRCE — the Fallback Root-Cause Engine

A fallback count says how many files the CST record lost. It does not say why, and
"why" is the only thing that turns into a fix. FRCE classifies every refusal the
comparison guard makes, counts it, keeps a small sample, and ranks the result by
LOC affected — so a corpus run ends with a ranked cause list instead of a number.

## Using it

```console
$ rust-graph build corpus45/awk --no-cache --frce-report
$ rust-graph build corpus45/tcl --no-cache --frce-report --frce-dump /tmp/tcl-frce.jsonl
```

`--frce-report` prints the table after the run:

```
FRCE — fallback root causes (ranked by LOC affected)
  316 fallbacks, 12453 LOC affected, 6 distinct reasons

code  reason                             files  share  loc_lost  channel  what a fix does
  51  DifferentialNameValue                330  34.5%      9120  event    the lexer's value names a name the CST value lacks
  15  MissingStatementTerminator           305  31.9%      7011  event    statement boundary both readers can mean
  52  DifferentialCallValue                110  11.5%      2660  event    the lexer's value calls or constants what the CST value lacks
  55  DifferentialMissingFact                42   4.4%       980  call     a fact the lexer recorded is absent from the CST record
  53  DifferentialConstantZero               92   9.6%       812  event    the CST wrote the value as the constant 0
  54  DifferentialTargetsDiffer               8   0.8%       203  event    the statement's assignment targets differ
```

Numbers above are illustrative of the shape, not a recorded run.

Ranking is LOC affected, then files, then code — deterministic, so two runs on the
same tree print the same order.

`--frce-dump <file>` writes one JSON object per kept sample (max 3 per reason),
with `schema`, `code`, `reason`, `channel`, `file`, `line`, `col`, `loc_affected`
and the offending source line, so a sample can be turned into a fixture without
re-running the parser:

```json
{"schema":1,"code":53,"reason":"DifferentialConstantZero","channel":"event","file":"corpus45/awk/x.awk","line":92,"col":0,"loc_affected":410,"files_in_reason":92,"loc_in_reason":812,"source":"  print arr[i] ..."}
```

## Codes

Codes 1–50 are the syntax taxonomy from the FRCE specification (lexer and
encoding 1–10, delimiters and indentation 11–18, expressions 19–27, declarations
28–37, imports and metaprogramming 38–45, guardrails 46–50).

Codes 51–55 are this engine's differential classes, and they are what the corpora
actually produce, because a fallback here is a disagreement between two readers
about a *record*, not an illegal token:

| code | class | meaning |
|---:|---|---|
| 51 | `DifferentialNameValue` | the lexer's value names a name the CST value lacks |
| 52 | `DifferentialCallValue` | the lexer's value calls or constants what the CST value lacks |
| 53 | `DifferentialConstantZero` | the CST wrote the value as the constant `0` |
| 54 | `DifferentialTargetsDiffer` | the statement's assignment targets differ |
| 55 | `DifferentialMissingFact` | a call, import, symbol or var the lexer recorded is absent from the CST record |

Without them, 95% of a real corpus lands in `UnclassifiedSyntaxBailout` and the
3% ceiling in the specification is unreachable. With them, `Unclassified` is a
signal rather than a habit: it should stay near zero and every occurrence is a
missing rule.

## Cost

Nothing is recorded on the success path. On a refusal the engine parses the line
number out of the detail string the guard already built, interns the path once,
and bumps a `u64` pair in a thread-local array — no map, no lock, no per-file
allocation. Samples are three fixed-size structs per reason (line, column, LOC,
path id, channel), so the resident cost is about 6 KB for the whole table.

## Tests

`cargo test --bin rust-graph --features 'deep parser' frce::` covers code order
and uniqueness, the classifier's guard arms, line/column extraction, sampling and
ranking, and the JSONL writer.

## Lane contract (from round 14 on)

Every lane reports both fallback rates, on both corpora, for every attempt:

1. **Baseline before the first edit**, and again after every attempt:
   - `RG_BIN=<bin> python3 tools/loc_weighted.py <lang>` — file % and LOC % on the
     normal corpus, with median and p90 file size so a skewed corpus is visible.
   - `RG_BIN=<bin> python3 tools/loc_weighted.py --root /tmp/opencode/lean/corpus45-large <lang>`
     — the same two numbers on the 700+ LOC corpus produced by `tools/large_files.py`.
2. **A fallback number without a cause is not an attempt.** After each language,
   run `--frce-report` and put the table in `report.json` under `frce`, plus one
   `--frce-dump` file for the top reason by LOC lost. The next attempt starts from
   the top row of that table, not from intuition.
3. If `UnclassifiedSyntaxBailout` is above 3% of a language's fallbacks, that is a
   missing classifier rule: record it in `neon_blockers.json` with the sample and
   move on. Guessing a fix for an unclassified bucket is what the engine exists to
   stop.
