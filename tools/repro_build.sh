#!/usr/bin/env bash
# Two cold builds must be byte-identical, or "the artifact I measured" is not
# "the artifact the next run produces" and every number in docs/ is a rumour.
#
# Run once per phase, not per commit: two clean builds of this crate cost a few
# minutes, and a gate nobody can afford to run is a gate nobody runs.
#
#   tools/repro_build.sh            # build twice, compare sha256, fail on drift
#   tools/repro_build.sh --print    # print the sha256 of a single cold build
set -uo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
TD=${CARGO_TARGET_DIR:-$PWD/target-repro}
export CARGO_TARGET_DIR="$TD"
BIN="$TD/x86_64-unknown-linux-musl/debug/rust-graph"

build_once() {
  cargo clean >/dev/null 2>&1
  if ! cargo build --features "deep parser" >/tmp/repro-build.log 2>&1; then
    echo "build failed; see /tmp/repro-build.log" >&2
    return 1
  fi
  sha256sum "$BIN" | cut -d' ' -f1
}

one=$(build_once) || exit 2
if [ "${1:-}" = "--print" ]; then
  echo "$one"
  exit 0
fi
two=$(build_once) || exit 2

echo "cold build 1: $one"
echo "cold build 2: $two"
if [ "$one" != "$two" ]; then
  echo "DRIFT: two cold builds differ — the measurement is not reproducible" >&2
  exit 1
fi
echo "OK: two cold builds are identical"
