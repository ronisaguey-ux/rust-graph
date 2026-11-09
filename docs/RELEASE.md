# Release checklist

Commands only. Run every block from the repository root, in order. Stop at the
first non-zero exit; the last section says what to do with it.

## 0. Preconditions

```sh
git status --porcelain          # must print nothing
git rev-parse --abbrev-ref HEAD
git log --oneline -1
rustc --version
cargo --version
df -h .                         # at least 3 GB free; a fat-LTO build needs it
```

## 1. Tests

```sh
cargo test --workspace
cargo test --workspace --features "deep parser"
```

Both must exit `0`. The second one runs the deep parser's tests; its
`parser_gate` test is listed as ignored in a debug build by design.

## 2. Builds and sizes

```sh
cargo build --release
stat -c '%s %n' target/x86_64-unknown-linux-musl/release/rust-graph

cargo build --release --features "deep parser"
stat -c '%s %n' target/x86_64-unknown-linux-musl/release/rust-graph \
    target/x86_64-unknown-linux-musl/release/rust-graph-deep-scan \
    target/x86_64-unknown-linux-musl/release/rust-graph-deep-bench
```

Record both sizes in `CHANGELOG.md`. The default binary must not grow when the
features are off; `cargo test -p deep-bench` checks the recorded hash.

## 3. Fuzzing

```sh
cargo run -p deep-fuzz -- languages
for l in $(target/x86_64-unknown-linux-musl/debug/deep-fuzz languages | cut -f1); do
    target/x86_64-unknown-linux-musl/debug/deep-fuzz fuzz --lang "$l" --seconds 30 --seed 20261108
    test $? -eq 0 || break
done
target/x86_64-unknown-linux-musl/debug/deep-fuzz corpus --dir crates/deep-grammars/fixtures
```

Every language line must show `0` panics, `0` hangs, `0` unreachable and `0`
memory, and both commands must exit `0`.

## 4. Conformance over a real corpus

```sh
cargo run --release -p deep-conformance --features deep-grammars -- \
    scan --root /usr/lib/python3.12 --out /tmp/rg-report.json
cargo run --release -p deep-conformance --features deep-grammars -- \
    summary --report /tmp/rg-report.json
cargo run --release -p deep-conformance --features deep-grammars -- \
    gates --report /tmp/rg-report.json
```

`gates` must exit `0`: clean rate at least 95 % for every language with 10 or
more files, aggregate throughput at least 15 MB/s, no file above 2 GiB RSS,
cold start below 1 ms.

## 5. The 2x parser gate

```sh
sh bench/build-tsdrv.sh
cargo test --release --features parser --test parser_gate -- --nocapture
```

`build-tsdrv.sh` needs `git`, a C compiler and the network. If it cannot run,
the gate cannot be measured on this machine; say so in `CHANGELOG.md` and do not
quote a ratio from an older machine as this release's measurement. Where it can
run, every corpus line must end in `ok`.

## 6. Version and changelog

```sh
# Cargo.toml: version = "X.Y.Z" in [package]
git diff -- Cargo.toml
git checkout -- Cargo.lock      # the bump is committed without a lock rebuild
git add Cargo.toml
git -c "user.name=Roni Saguey" -c "user.email=ronisaguey-ux@users.noreply.github.com" \
    commit -m "chore(release): bump version to X.Y.Z"

git add CHANGELOG.md
git -c "user.name=Roni Saguey" -c "user.email=ronisaguey-ux@users.noreply.github.com" \
    commit -m "docs(changelog): record X.Y.Z"
```

## 7. Tag and push

```sh
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin HEAD
git push origin vX.Y.Z
```

## When a gate fails

```sh
# a test fails
cargo test --workspace --features "deep parser" -- --nocapture <test-name>
# fix, rerun the failing suite, then restart at section 1

# fuzz finds a failure
target/x86_64-unknown-linux-musl/debug/deep-fuzz shrink --input crates/deep-fuzz/crashes/<file>.bin
# keep the shrunk input as a fixture before restarting at section 3

# conformance is under a floor
cargo run --release -p deep-conformance --features deep-grammars -- \
    diff --a /tmp/rg-report.prev.json --b /tmp/rg-report.json
# no tag until `gates` exits 0

# the 2x gate is under 2.00x
git checkout -- .
# no tag; the ratio goes in the issue, not in a release note

# disk runs out mid-build
cargo clean
df -h .
# rerun from section 2
```

A failed gate means no tag and no push. `git reset --hard` is not part of this
list on purpose: fix forward, or reset only a local, un-pushed release commit
after copying its diff to a patch.
