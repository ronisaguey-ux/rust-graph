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

`--features event-split` and any `cargo test` run in the same target directory
**overwrite** that binary without the `deep` feature; the symptom is
`unknown option --lexer`. Rebuild before measuring.

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

## 3. Prove a delta is explained

```bash
python3 tools/diff.py /tmp/sweep_python.json /tmp/sweep_python_after.json
```

Exit 0 means every file that moved forward is accounted for. Exit 2 means a file
moved clean → fallback without a named reason: that is a regression and the
commit does not happen. `--by lang,reason` groups the delta; `--json` prints it.

The comparison is only meaningful between records of the same corpus and the same
binary features. A sweep made with `event-split` compares to nothing.

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
