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

CORPORA = ("js", "ts", "python", "rust")
DEFAULT_ROOT = "/tmp/opencode/sweep-local"
DEFAULT_BIN = "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph"


def measure(binary: str, path: str) -> tuple[int, int, int]:
    result = subprocess.run(
        [binary, "build", path, "--no-cache", "--stdout"],
        capture_output=True,
        check=True,
    )
    pack = json.loads(result.stdout)
    rows = pack["sources"]
    fallbacks = [row for row in rows if row.get("deep_fallback")]
    losses = sum(row.get("deep_losses", 0) for row in fallbacks)
    return len(fallbacks), losses, len(rows)


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
