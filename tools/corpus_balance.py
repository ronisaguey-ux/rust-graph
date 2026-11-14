#!/usr/bin/env python3
"""corpus_balance.py — is a corpus made of more than big files?

A reader bug in a 300-line file is usually visible in a 40-line one, and a
corpus of large files hides it: the constructs that go wrong are common in
small modules and rare in monoliths. This measures the shape of a corpus
instead of assuming it.

It reports, per language directory, the file count and how many are under a
limit, and it fails when the JavaScript or TypeScript corpus has fewer small
files than the floor P4-13 asks for.

Usage:
    python3 tools/corpus_balance.py                     # defaults below
    python3 tools/corpus_balance.py --root DIR --limit 2048 --floor 100
    python3 tools/corpus_balance.py --json
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

ROOT = Path(os.environ.get("RG_SWEEP_ROOT", "/tmp/opencode/sweep-local"))
LIMIT = 2048
FLOOR = 100
REQUIRED = ("js", "ts")
EXTS = (".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx")


def measure(directory: Path, limit: int) -> tuple[int, int]:
    total = small = 0
    for path in directory.rglob("*"):
        if not path.is_file() or path.suffix not in EXTS:
            continue
        total += 1
        if path.stat().st_size < limit:
            small += 1
    return total, small


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--limit", type=int, default=LIMIT, help="small-file ceiling in bytes")
    parser.add_argument("--floor", type=int, default=FLOOR, help="least small files a required corpus may have")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    if not args.root.is_dir():
        print(f"corpus_balance: {args.root} is not a directory", file=sys.stderr)
        return 2

    report = {}
    failed = []
    for directory in sorted(path for path in args.root.iterdir() if path.is_dir()):
        total, small = measure(directory, args.limit)
        if total == 0:
            continue
        report[directory.name] = {"total": total, "small": small}
        if directory.name in REQUIRED and small < args.floor:
            failed.append(f"{directory.name}: {small} files under {args.limit} bytes, floor {args.floor}")

    if args.json:
        print(json.dumps({"root": str(args.root), "limit": args.limit, "floor": args.floor, "corpora": report}, indent=2))
    else:
        for name, row in report.items():
            mark = "!" if name in REQUIRED and row["small"] < args.floor else " "
            print(f"{mark} {name:8} total {row['total']:5}  under {args.limit}B {row['small']:5}")
    if failed:
        for line in failed:
            print(f"corpus_balance: {line}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
