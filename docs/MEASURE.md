# Measuring rust-graph

Every number in this repo comes from one of these commands. They are written to be
pasted as-is, in this order, and each one prints the only line that matters.

## 1. Build the binary the measurements use

```bash
export PATH=$HOME/.cargo/bin:$PATH \
       CARGO_TARGET_DIR=/home/roni/Roni_workspace/rg-kbuilds/verify \
       CARGO_BUILD_JOBS=6
cd /home/roni/Roni_workspace/rg-ultimate
cargo build --features "deep parser"
```

The binary lands at
`$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/debug/rust-graph`.

Any `cargo test` run in the same target directory **overwrites** that binary
without the `deep` feature; the symptom is `unknown option --lexer`. Rebuild
before measuring. (The `event-split` feature was deleted on 2026-11-13: it never
beat the default on the measurement subset, so it is gone rather than dormant.)

## 2. Measure one corpus

```bash
python3 /tmp/shard_sweep.py /tmp/opencode/sweep-local/python \
        /tmp/sweep_python.json 8
```

`shard_sweep.py` links the corpus into N disjoint shards, runs
`rust-graph build <shard> --no-cache --stdout` on each, merges the `sources` rows
and writes the merged record plus a one-line total. The corpora are copied under
`/tmp/opencode/sweep-local/` (CPython 3.12 stdlib, a JS sample, a TS sample and
this workspace's Rust); they are not in the repo because they are 6,000 files of
other people's code.

| language | root | baseline |
| --- | --- | --- |
| javascript | `/tmp/opencode/sweep-local/js` | 691 / 1995 |
| typescript | `/tmp/opencode/sweep-local/ts` | 406 / 2000 |
| python | `/tmp/opencode/sweep-local/python` | 146 / 568 |
| rust | `/tmp/opencode/sweep-local/rust` | 345 / 1816 |

A record is the JSON `build --stdout` prints; the fields that matter are
`files`, `sources[].deep_fallback` and `deep.mode`.

Sweep with the harness in this repository, not a copy in `/tmp`, so the record it
writes outlives the box:

```bash
python3 tools/shard_sweep.py /tmp/opencode/sweep-local/js /tmp/js.json 6
```

It writes `/tmp/js.json` and `/tmp/js.json.reasons.json`. The second file is the
per-language reason histogram, and it exists because a single number hides a
trade: a phase that fixes twelve missing events and introduces twelve missing
symbols reports the same total as a phase that did nothing. Compare the two
histograms, not just the two totals:

```bash
python3 - <<'EOF'
import json
for phase in ("before", "after"):
    d = json.load(open(f"/tmp/{phase}.reasons.json"))["reasons"]
    print(phase, d)
EOF
```

## 3. Prove a delta is explained

```bash
python3 tools/diff.py /tmp/sweep_python.json /tmp/sweep_python_after.json
```

Exit 0 means every file that moved forward is accounted for. Exit 2 means a file
moved clean → fallback without a named reason: that is a regression and the
commit does not happen. `--by lang,reason` groups the delta; `--json` prints it.

The comparison is only meaningful between records of the same corpus and the same
binary features. A sweep made against a binary built for another feature set
compares to nothing.

## 4. The suites

```bash
cd /home/roni/Roni_workspace/rg-ultimate
cargo test --workspace --features "deep parser"     # 1463 tests
cd /home/roni/Roni_workspace/felicity
python3 -m pytest -q                                 # 1169 tests
```

Both must be green before a commit. `cargo test` in the measurement target
directory leaves a test binary in place of the measuring one — rebuild step 1
afterwards.

## 5. Inspect one file by hand

```bash
B=/home/roni/Roni_workspace/rg-kbuilds/verify/x86_64-unknown-linux-musl/debug/rust-graph
mkdir -p /tmp/one && cp <file> /tmp/one/
$B build /tmp/one --lexer --no-cache --stdout > /tmp/lexer.json
$B build /tmp/one --deep  --no-cache --stdout > /tmp/deep.json
```

`--lexer` is the one-pass reader, `--deep` is the CST only, `--compare` prints the
per-file difference without choosing. The record holds `symbols`, `edges`,
`imports`, `unresolved` and `flows`; `deep.mode` and `paths` say which reader
produced it.

## 6. Cost, batching and the cooling-off rule

A full sweep on a rented 80-core Vast box costs about **4 cents**: rent, run,
destroy. `tools/vast_sweep.sh` rents and runs the command after `--`, and
destroys the instance from an `EXIT/INT/TERM/HUP` trap, so a sweep that dies
mid-run (a compiler crash, a timeout, a kill) still destroys the box. The rule
is proven rather than assumed:

```bash
tools/vast_sweep.sh --self-test
# SELF-TEST OK: failed sweep still destroyed instance 12345
```

Spend it once per language, not once per fix. One call per language, its output
piped through `head`/`awk`/`jq` — a raw log or a full `ps` table in the
conversation is paid for again on every later step of that turn, which costs
more than the sweep did.

**Cooling-off rule.** A fix that flips one file and breaks two is reverted in
the same turn, and the reversal is committed — never left half-applied while the
next idea is tried. The measurement that shows the break is quoted in the commit
message so the next reader does not re-run the experiment.

## 7. Rolling a phase back

A phase is one commit (or a short chain). Undoing it is one command, and the
numbers that justified it are re-derived in the same breath:

```bash
git revert --no-edit <phase-commit>          # the commit, not the tree
git log --oneline -3                         # the revert is its own commit
tools/repro_build.sh --print                 # the binary is a new artifact now
# then the four sweeps and the gate, exactly as in section 2 and 3:
python3 /tmp/shard_sweep.py /tmp/opencode/sweep-local/js  /tmp/after_js.json 6
python3 /tmp/shard_sweep.py /tmp/opencode/sweep-local/ts  /tmp/after_ts.json 6
python3 /tmp/shard_sweep.py /tmp/opencode/sweep-local/python /tmp/after_python.json 6
python3 /tmp/shard_sweep.py /tmp/opencode/sweep-local/rust /tmp/after_rust.json 6
tools/diff.py <before>.json <after>.json     # refuses an unexplained regression
cargo test --workspace --features "deep parser"
(cd /home/roni/Roni_workspace/bugbounty_agent && python3 -m pytest -q)
```

`git revert` never rewrites history, so a bad phase stays visible in the log as
the thing that was tried and withdrawn — which is the point: the next reader
learns from the reversal instead of digging for the missing code.

Do not `git reset --hard` a phase commit that has been pushed: the remote keeps
the commit and the local branch then lies about what the corpus was measured on.

## The guard's cost, and the 5% budget

Measured 2026-11-13 on the 1,995-file JavaScript corpus, release build, cold
cache, `RG_TIMING=1` (the binary then prints the nanoseconds it spent inside the
guard's comparison):

| run | wall | guard | share |
| --- | ---: | ---: | ---: |
| before any work | 27.4 s | 19.8 s | 72% |
| after the indexes and the line table | 14.8 s | 7.4 s | 50% |

So the guard got 2.7× faster and the build 1.85× faster, and the 5% budget is
still not met. It is worth saying plainly rather than tuning the definition: the
comparison is not 5% of this build, it is half of it.

What was fixed, in order of what it bought:

- **`src.lines().nth(n)` in every line-shaped check.** Each of the twenty-odd
  helpers scanned the file from its first byte to reach line `n`, and they run
  per event, so the guard was quadratic in file size for no reason at all. The
  source is now split once per file into a `Lines` table and lookups are
  constant time. This was the largest single win.
- **Deep calls indexed by leaf name and line.** The call check rescanned every
  deep call for every lexer call, allocating a `String` per comparison.
- **Event shapes cached per CST symbol.** A module symbol's pool is every other
  symbol's events, so the same value string was parsed once per enclosing
  comparison.

What is left is the shape derivation itself — `shape_at` walking each value and
`strip_literal_prefixes` looking at the statement's neighbourhood — which is
linear work the comparison needs for every event on both sides. Reaching 5% from
here means not deriving most shapes, not deriving them faster: a cheap
pre-screen that can prove agreement without shaping, or comparing lazily until
the first real difference. Both change the guard's design, so they belong to a
decision about the budget rather than to a tuning pass.

Instrumentation: `RG_TIMING=1` prints the guard total, `RG_PHASE=1` breaks it
into imports, symbol names, calls, the event comparison, and inside that the
wanted rows, the found rows, and the pairing. Both are environment-gated and
cost one environment lookup per file when off.
