#!/bin/sh
# The Tree-sitter side of `compare`, in the harness's parser contract:
#
#     <binary> --list FILE [--reps N]
#
# wraps the Phase 1 driver, whose CLI is
#
#     tsdrv bench GRAMMAR LIST REPS
#
# Build it with `sh bench/build-tsdrv.sh` (needs git, a C compiler and the
# network once), then pick the grammar for the corpus:
#
#     TSDRV=target/bench/tsdrv TS_GRAMMAR=rust \
#         crates/deep-bench/adapters/tsdrv.sh --list corpus.list
set -eu

driver=${TSDRV:?TSDRV must be the path to bench/tsdrv, built by bench/build-tsdrv.sh}
grammar=${TS_GRAMMAR:?TS_GRAMMAR must name the tree-sitter grammar, e.g. rust}
list=
reps=1
while [ $# -gt 0 ]; do
    case $1 in
        --list) list=${2:?--list needs a file}; shift 2 ;;
        --reps) reps=${2:?--reps needs a number}; shift 2 ;;
        *) echo "tsdrv.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done
[ -n "$list" ] || { echo "tsdrv.sh: --list is required" >&2; exit 2; }
exec "$driver" bench "$grammar" "$list" "$reps"
