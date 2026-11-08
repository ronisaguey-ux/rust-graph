#!/bin/sh
# Writes a corpus manifest: one absolute path per line, sorted, no blanks.
#
#     sh crates/deep-bench/make-corpus.sh ROOT [EXTENSION...]
#
# Defaults to `rs`. Paths are absolute because the harness refuses anything
# else, and they are not relative to the working directory, so the manifest
# means the same corpus wherever it is read.
set -eu

root=${1:?usage: make-corpus.sh ROOT [EXTENSION...]}
shift || true
[ -d "$root" ] || { echo "make-corpus.sh: $root is not a directory" >&2; exit 2; }
root=$(cd "$root" && pwd)
extensions=${*:-rs}

find_args=
for extension in $extensions; do
    find_args="$find_args -o -name *.$extension"
done
# shellcheck disable=SC2086
find "$root" -type f \( ${find_args# -o } \) | LC_ALL=C sort
