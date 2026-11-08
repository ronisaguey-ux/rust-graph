#!/bin/sh
# The deep side of `compare`, in the harness's parser contract:
#
#     <binary> --list FILE [--reps N]
#
# wraps the deep scan that already ships in this repository:
#
#     rust-graph-deep-scan bench FILE [--reps N]
#
# Build that binary first and point DEEP_SCAN at it:
#
#     cargo build --release --features parser
#     DEEP_SCAN=target/x86_64-unknown-linux-musl/release/rust-graph-deep-scan \
#         crates/deep-bench/adapters/deep-scan.sh --list corpus.list
set -eu

binary=${DEEP_SCAN:?DEEP_SCAN must be the path to rust-graph-deep-scan}
list=
reps=1
while [ $# -gt 0 ]; do
    case $1 in
        --list) list=${2:?--list needs a file}; shift 2 ;;
        --reps) reps=${2:?--reps needs a number}; shift 2 ;;
        *) echo "deep-scan.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done
[ -n "$list" ] || { echo "deep-scan.sh: --list is required" >&2; exit 2; }
exec "$binary" bench "$list" --reps "$reps"
