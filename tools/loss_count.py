#!/usr/bin/env python3
"""Losses, not just the files that lost something.

The file-level fallback count is the number everyone quotes, and it has a trap:
it only moves when a file loses its **last** loss. Fix one event in a file that
loses five and the number does not budge, so a real improvement looks like no
improvement at all.

The sweep carries `deep_losses` per file for exactly this reason — the event
comparison counts every loss it finds, not just the first. This tool prints both
numbers side by side for the four corpora:

    tools/loss_count.py            # all four, release binary
    tools/loss_count.py --corpus js

Losses are counted per kind: the event comparison counts every event it could
not pair, and the import, symbol, call and variable checks count one each,
because those stop at their first miss. So the total is a floor, not a census —
it is enough to show an event-level fix moving, and it never overstates.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

CORPORA = ("js", "ts", "python", "rust", "kotlin")
DEFAULT_ROOT = "/tmp/opencode/sweep-local"
DEFAULT_BIN = "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph"


def measure(binary: str, path: str) -> tuple[int, int, int]:
    """Every loss row the deep reader reports, not only the files that fell back.

    `deep_fallback` is set when a file loses completely, so a corpus whose reader
    is mostly right reports zero files and still drops real events (kotlin was
    quoted as 0 files while 549 event rows failed). RG_AUDIT=1 RG_LOSSES=1 dumps the rows
    themselves, and a row is what a lane fixes.
    """
    env = dict(os.environ, RG_LOSSES="1", RG_AUDIT="1")
    result = subprocess.run(
        [binary, "build", path, "--no-cache"],
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    files: set[str] = set()
    losses = 0
    # The dump is written to stderr, so a stdout-only read counts nothing.
    for line in (result.stdout + result.stderr).splitlines():
        if not line.startswith("loss\t"):
            continue
        parts = line.split("\t")
        if len(parts) > 2:
            files.add(parts[1])
            losses += 1
    return len(files), losses, len(files)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=os.environ.get("RG_CORPORA", DEFAULT_ROOT))
    parser.add_argument("--binary", default=os.environ.get("RG_BIN", DEFAULT_BIN))
    parser.add_argument("--corpus", choices=CORPORA, action="append", help="repeatable")
    args = parser.parse_args()

    if not os.path.exists(args.binary):
        print(f"no binary at {args.binary} (build it, or set RG_BIN)", file=sys.stderr)
        return 1

    wanted = args.corpus or list(CORPORA)
    unmeasured = 0
    print(f"{'corpus':8} {'files':>7} {'losses':>7} {'per file':>9}")
    for name in wanted:
        path = os.path.join(args.root, name)
        if not os.path.isdir(path):
            print(f"{name:8} missing", file=sys.stderr)
            continue
        fallbacks, losses, total = measure(args.binary, path)
        rate = f"{losses / fallbacks:.2f}" if fallbacks else "-"
        print(f"{name:8} {fallbacks:>4}/{total:<4} {losses:>7} {rate:>9}")
        if losses < fallbacks:
            unmeasured += 1
    if unmeasured:
        print(f"\n{unmeasured} corpus/corpora report fewer losses than files, which cannot", file=sys.stderr)
        print("happen if every falling-back file carries at least one loss.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
