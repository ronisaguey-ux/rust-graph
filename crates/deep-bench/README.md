# deep-bench

The harness behind the deep parser's 2x gate: it verifies a corpus, times the
deep parser against Tree-sitter on exactly that corpus, and reports the deep
parser's peak memory. It measures whole processes, so it never needs to know how
either parser works, and it refuses to print a number it cannot stand behind.

```text
rust-graph-deep-bench corpus --list FILE
rust-graph-deep-bench compare --tree-sitter PATH --deep PATH --list FILE [--rounds N] [--reps N]
rust-graph-deep-bench memory --deep PATH --list FILE
```

Every report starts with the same four lines — operating system and host, CPU
model, core count, and the exact command line, shell-quoted — so a number can be
reproduced or disbelieved:

```text
machine: Ubuntu 24.04.4 LTS / roni-saguey-HP-ENVY-Laptop-13-ad1xx
cpu: Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz
cores: 8
command: target/x86_64-unknown-linux-musl/release/rust-graph-deep-bench corpus --list /tmp/rg-corpus-rs.list
```

Exit codes: `0` success or the gate passed, `1` both parsers ran and the deep
parser is under the margin, `2` nothing was measured — a missing manifest entry,
a missing binary, or a bad flag.

## Building it

```sh
cargo build --release
# target/x86_64-unknown-linux-musl/release/rust-graph-deep-bench
```

The binary is declared in the root `Cargo.toml` (the `[[bin]]` entry) and its
code, tests and `default-build.sha256` live here. `cargo test` at the repository
root builds the binary target, so these tests run with everyone else's;
`cargo test -p deep-bench` runs the same tests against the library.

## The corpus manifest

Plain text, one path per line, absolute. Blank lines and lines starting with `#`
are ignored. Nothing is timed until every path has been checked: each file must
exist and be a regular file, and their sizes are summed for the report. A
manifest with a missing entry is refused in full — the tool never measures the
subset it could see.

```sh
sh crates/deep-bench/make-corpus.sh src rs > /tmp/rg-corpus-rs.list
target/x86_64-unknown-linux-musl/release/rust-graph-deep-bench corpus --list /tmp/rg-corpus-rs.list
```

on this machine:

```text
machine: Ubuntu 24.04.4 LTS / roni-saguey-HP-ENVY-Laptop-13-ad1xx
cpu: Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz
cores: 8
command: target/x86_64-unknown-linux-musl/release/rust-graph-deep-bench corpus --list /tmp/rg-corpus-rs.list

manifest: /tmp/rg-corpus-rs.list  files: 28  bytes: 548741  sha256: 57da6644d25e83fb4f9a818600deed54495d375110545646c4693371c5ef11ce
```

The SHA-256 covers the manifest's own bytes, not the corpus, so a list that was
edited — a file added, a path reordered — is a different manifest even when the
files still check out.

A broken manifest refuses before anything runs:

```text
$ rust-graph-deep-bench corpus --list /tmp/broken.list
rust-graph-deep-bench: refusing to report: 1 manifest entry cannot be measured:
  /tmp/broken.list:2: /nope/missing.rs: No such file or directory (os error 2)
```

## The parser contract

Both sides are ordinary programs. For a timing run the harness executes

```text
<binary> --list FILE [--reps N]
```

and expects exit `0` after every path in `FILE` was parsed; `--reps N` is only
passed when the harness was given `--reps`, and means "parse the corpus `N`
times inside this process". Saying nothing on stdout is fine. `crates/deep-bench/adapters`
wraps the two CLIs already in this repository:

```sh
# the deep side
DEEP_SCAN=target/x86_64-unknown-linux-musl/release/rust-graph-deep-scan \
    crates/deep-bench/adapters/deep-scan.sh --list FILE
#   -> rust-graph-deep-scan bench FILE --reps N

# the tree-sitter side
TSDRV=target/bench/tsdrv TS_GRAMMAR=rust \
    crates/deep-bench/adapters/tsdrv.sh --list FILE
#   -> tsdrv bench rust FILE N
```

## The 2x gate

```sh
sh bench/build-tsdrv.sh                        # Tree-sitter + grammars, once; needs cc and network
cargo build --release --features parser        # the deep scan

sh crates/deep-bench/make-corpus.sh "${CORPUS_ROOT:-src}" rs > /tmp/rg-corpus-rs.list

target/x86_64-unknown-linux-musl/release/rust-graph-deep-bench compare \
    --tree-sitter crates/deep-bench/adapters/tsdrv.sh \
    --deep crates/deep-bench/adapters/deep-scan.sh \
    --list /tmp/rg-corpus-rs.list \
    --rounds 7
```

with `TSDRV` and `TS_GRAMMAR` exported for the tree-sitter adapter and `DEEP_SCAN`
exported for the deep one. Each round runs both parsers once on the whole corpus,
alternating which goes first, so a warming page cache or a frequency drop cannot
favour the parser that always ran first. The report is the median, mean, min and
max of `--rounds` (default 5) runs per parser, and the ratio is tree-sitter's
median over the deep parser's. Under 2.00x the process exits `1`.

## Peak memory

```sh
target/x86_64-unknown-linux-musl/release/rust-graph-deep-bench memory \
    --deep crates/deep-bench/adapters/deep-scan.sh --list /tmp/rg-corpus-rs.list
```

`VmHWM` is the kernel's high-water mark of resident memory. The harness reads it
from `/proc/<pid>/status` every millisecond while the parser is alive, because
`VmHWM` is the same field `/proc/self/status` shows but a parser that already
exited has no peak left to read. It prints kB and MB, the number of samples, and
its own `VmHWM` as a sanity line; a run too short to sample is an error, not a
zero.

## What this machine could and could not measure

Nothing on this box can produce a parser number, and the tool says so instead of
inventing one:

- **Tree-sitter is not installed**, and there is no C compiler to build it:
  `cc`, `gcc`, `clang` and `tree-sitter` are all missing, and no
  `libtree-sitter` or `pkg-config` is present. `bench/build-tsdrv.sh` cannot run.
- **The deep scan cannot be built either.** `cargo build --features parser`
  fails at `rust-graph-deep`, a proc-macro crate that has to be built and linked
  for the host, with `linker 'cc' not found`; the host toolchain has no glibc
  development files to link against (pointing the host linker at `rust-lld`
  fails on `-lm`, `-ldl` and `-lc`). `.cargo/config.toml` only pins the musl
  target linker, which is why the default build works here and this does not.

So the gate's exact command, verbatim from this machine:

```text
$ rust-graph-deep-bench compare --tree-sitter /nonexistent/tsdrv \
      --deep /tmp/fixture-parser.sh --list /tmp/rg-corpus-rs.list --rounds 3
rust-graph-deep-bench: --tree-sitter: /nonexistent/tsdrv does not exist; Tree-sitter is not installed here and there is no C compiler to build it from source, so no ratio can be measured.
  toolchain: cc=missing gcc=missing clang=missing tree-sitter=missing
```

The harness itself was exercised end to end with a stand-in that only reads the
corpus — a fixture, not a parser, and its numbers mean nothing about parsing:

```text
$ rust-graph-deep-bench compare --tree-sitter /tmp/fixture-parser.sh \
      --deep /tmp/fixture-parser.sh --list /tmp/rg-corpus-rs.list --rounds 4
round  1  ts,deep  tree-sitter    26.631 ms  deep    26.931 ms
round  2  deep,ts  tree-sitter    26.998 ms  deep    26.131 ms
round  3  ts,deep  tree-sitter    27.219 ms  deep    27.547 ms
round  4  deep,ts  tree-sitter    27.198 ms  deep    28.134 ms

parser           median       mean        min        max  (ms)
tree-sitter      27.098     27.012     26.631     27.219
deep             27.239     27.186     26.131     28.134

deep throughput: 20.1 MB/s  tree-sitter: 20.3 MB/s
ratio (tree-sitter / deep): 0.99x  gate 2.0x  FAIL
```

That is the plumbing working — alternating rounds, both orders, statistics, and a
gate that fails honestly — and `memory` over the same fixture reported
`peak rss: 1892 kB = 1.8 MB (25 samples of /proc/<pid>/status VmHWM)`.

## The default build is unchanged

`src/deep_integration.rs` is the bridge rust-graph would call with the `deep`
feature on. Nothing in the default build names it, and its one door to the
runtime is behind `#[cfg(feature = "deep")]`, so adding it cannot change
rust-graph's binary. That is measured, not assumed:

```sh
cargo build --release && sha256sum target/x86_64-unknown-linux-musl/release/rust-graph
cargo build           && sha256sum target/x86_64-unknown-linux-musl/debug/rust-graph
```

before this field's files existed, and again after they were in place:

| profile | before                                                             | after                                                              | bytes   |
| ------- | ------------------------------------------------------------------ | ------------------------------------------------------------------ | ------- |
| release | `60a554b0f2d3116042ca0be3360c8514a68a3386df67782658257bc10e820b7c` | `60a554b0f2d3116042ca0be3360c8514a68a3386df67782658257bc10e820b7c` | 837496  |
| debug   | `1a183a0b62570a9526af15c55cd14be207ac58ce866a62f8755e45edb63de57d` | `1a183a0b62570a9526af15c55cd14be207ac58ce866a62f8755e45edb63de57d` | 10848816 |

A third build of the same tree in an empty target directory
(`cargo build --release --target-dir /tmp/rg-hash-target`) reproduced the release
hash, so the number is a property of the source and not of a warm build
directory. `default-build.sha256` records the rows and the commands, and
`cargo test` re-reads it —
`the_default_build_is_byte_identical_to_the_one_before_this_field` fails if the
two ever disagree, and `the_default_build_does_not_reference_the_bridge` fails if
`src/main.rs` starts naming the bridge or the runtime without a gate.

Wiring the bridge, once the runtime builds, is two lines in `src/main.rs`:

```rust
#[cfg(feature = "deep")] mod deep;
#[cfg(feature = "deep")] mod deep_integration;
```

`src/main.rs` is not this field's file, so the lines are written down and not
applied; the `deep` feature already exists in the root `Cargo.toml` and is off by
default.
