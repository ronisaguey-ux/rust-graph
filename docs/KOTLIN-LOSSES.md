# Kotlin corpus: where the losses are (2026-11-19)

State of the kotlin campaign, written down so nobody re-derives it from scratch.

## Numbers

| point | files with a loss | loss rows |
|---|---|---|
| start (integ-f 8c3f120) | 480 | 549 |
| after wave 0 | 358 | 444 |
| after wave 4 (published `ed1633a`) | **144** | **160** |
| target (P6-09d) | 120 | — |

`js`, `ts`, `python` and `rust` are at zero losses and have been since wave 0.
Corpus root: `/tmp/opencode/sweep-local` (460 MB of kotlin under `kotlin/`).

## How to measure

```bash
RG_BIN=<repo>/target/x86_64-unknown-linux-musl/release/rust-graph \
  python3 tools/loss_count.py --root /tmp/opencode/sweep-local --corpus kotlin
```

`loss_count.py` sets `RG_AUDIT=1 RG_LOSSES=1` itself and reads **both** streams —
the dump goes to stderr in a directory build and a stdout-only read counts
nothing. One corpus takes ~4 min on 8 cores, ~1 min on the 64-core box.

The per-row dump for a single directory is the fastest repro:

```bash
RG_LOSSES=1 RG_AUDIT=1 rust-graph build <some/subdir> --no-cache 2>&1 | grep ^loss
loss  jvm/test/YamlConfigTestJvm.kt  call  call Yaml.default.decodeFromString line 67 in testSystemPropertyConfig
```

A row means the deep reader has that event and the lexer does not. The third
field is the event kind (`call`, `symbol`, `import`, `var`).

## The biggest open bug: `val x = a.b.c(y)` loses the call

Smallest reproduction that still fails:

```kotlin
package demo
fun g(content: String) {
    val one = a.b.c(content)     // calls lexer=0   <-- BUG
}
```

```bash
rust-graph build /tmp/kt-repro/e --no-cache --compare
R.kt  symbols lexer=2  calls lexer=0
```

Same statement without `val`, or as a plain assignment, records the call:

```kotlin
one = a.b.c(content)             // calls lexer=1
a.b.c(content)                   // calls lexer=1
```

A generic argument (`a.b.c<YamlMap>(content)`) makes no difference, so the
`<...>` is not the cause — an attempt that skipped kt type-arguments in
`expr.rs::chain()` changed nothing (144/160 before and after) and was reverted.

Where to look: `src/expr.rs`, the `close == b'}'` branch that skips
`val|var <name> =` (and `prop =`) inside a lambda body before reading the RHS.
It does `self.pos += at + 1; continue;`, i.e. it resumes **mid-statement**, and
the loop then reads the RHS as a value in declaration state instead of as a
fresh expression, so the call is never emitted. Resetting the item anchor there
(`item_start`, `first_word`) was tried and did not move the repro either.

This shape is the largest cluster in the plan (`kotlin-call-empty-val`, 9 files)
and it is what loses `Yaml.default.decodeFromString` in
`ktor/ktor-server/ktor-server-config-yaml/jvm/test/YamlConfigTestJvm.kt:67`.

## Wave mechanics that cost hours

- The lane plan must be regenerated from the **live** dump every wave, and
  `plan_from_losses.py` must run with `--corpus kotlin`: the audit dump still
  carries js/ts event rows for corpora at zero losses, and clustering those gave
  a wave of ten JS/TS steps that no lane could fix (wave 5: ten lanes, ten
  "nothing green", empty patch).
- `plan_from_losses.py` parses `result.stderr`; passing `--stdout` to the binary
  suppressed the dump and produced a plan with zero steps (wave 6's first run).
- `/root/tgt-measure` once held a **stale binary** that emitted no kotlin rows
  (different md5 from a fresh build of the same commit, 0 rows instead of 160).
  A fresh target dir (`/root/tgt-check`) was correct. A measure that reads 0 rows
  for kotlin is a stale binary, not progress.
- agy can return `RESOURCE_EXHAUSTED` (429, "Individual quota reached") on every
  lane at once — ten lanes, ten instant "no commit", nothing to integrate. A
  claude-haiku lane (`claude -p <prompt> --model haiku
  --dangerously-skip-permissions`) is the drop-in replacement: `ORCH_AGY=<claude>
  ORCH_AGENT_MODEL=haiku python3 run_agents.py --lanes N`.
- `run_agents.py` resolves the trunk as `github/<trunk>`; a checkout whose remote
  is named something else (or whose proxy fetch 500s) needs
  `git update-ref refs/remotes/github/<trunk> <sha>` before the lanes start.
