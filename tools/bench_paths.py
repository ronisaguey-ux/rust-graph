#!/usr/bin/env python3
"""One command that shows both readers: time and peak memory, side by side.

A regression in either reader is invisible when only `auto` is measured, because
the mix changes with the corpus. This runs the three modes the binary already
supports over the same directory — `--lexer`, `--deep`, `auto` — and prints the
wall time and peak RSS each one reports.

    tools/bench_paths.py                       # the four local corpora
    tools/bench_paths.py /tmp/corpus --reps 3

`auto` is not the sum of the other two: it runs both readers per file and then
the guard, so its time is the honest end-to-end number and the guard's share is
`auto - lexer - deep` when the corpus is the same. Peak RSS is the process's own
high-water mark, so the three rows are three separate runs.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

CORPORA = ("js", "ts", "python", "rust")
DEFAULT_ROOT = "/tmp/opencode/sweep-local"
DEFAULT_BIN = "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph"
LINE = re.compile(r"(\d+) files \((\d+) parsed, \d+ cached\), (\d+) symbols, (\d+) ms, peak (\d+) KB")


def run(binary: str, corpus: str, mode: str) -> tuple[int, int, int, int] | None:
    command = [binary, "build", corpus, "--no-cache", "--out", "/tmp/bench-paths"]
    if mode:
        command.append(mode)
    result = subprocess.run(command, capture_output=True, text=True)
    match = LINE.search(result.stderr)
    if not match:
        return None
    files, parsed, symbols, ms, kb = (int(group) for group in match.groups())
    return files, parsed, symbols, ms, kb


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("corpus", nargs="*", help="directories, or corpus names under --root")
    parser.add_argument("--root", default=os.environ.get("RG_CORPORA", DEFAULT_ROOT))
    parser.add_argument("--binary", default=os.environ.get("RG_BIN", DEFAULT_BIN))
    args = parser.parse_args()

    if not os.path.exists(args.binary):
        print(f"no binary at {args.binary} (build it, or set RG_BIN)", file=sys.stderr)
        return 1

    targets = []
    for name in args.corpus or CORPORA:
        path = name if os.path.isdir(name) else os.path.join(args.root, name)
        if not os.path.isdir(path):
            print(f"{path} is not a directory", file=sys.stderr)
            continue
        targets.append((name, path))

    print(f"{'corpus':8} {'mode':7} {'files':>6} {'symbols':>8} {'ms':>7} {'peak MB':>8}")
    for name, path in targets:
        rows = {}
        for mode in ("--lexer", "--deep", ""):
            measured = run(args.binary, path, mode)
            label = mode.lstrip("-") or "auto"
            rows[label] = measured
            if measured is None:
                print(f"{name:8} {label:7} measurement did not parse", file=sys.stderr)
                continue
            files, _, symbols, ms, kb = measured
            print(f"{name:8} {label:7} {files:>6} {symbols:>8} {ms:>7} {kb / 1024:>8.1f}")
        if all(rows.get(mode) for mode in ("lexer", "deep", "auto")):
            guard = rows["auto"][3] - rows["lexer"][3] - rows["deep"][3]
            print(f"{name:8} {'guard':7} {'':>6} {'':>8} {guard:>7} {'':>8}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
