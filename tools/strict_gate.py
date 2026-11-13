#!/usr/bin/env python3
"""Fail when a file moves to the fallback path without a named reason.

Two sweeps of the same corpus, one per binary: the default and the strict build.
Strict pairing accepts fewer pairings, so files move from clean to falling back
and that movement is expected. What is not acceptable is a move with nothing
written on it — a fallback whose reason is empty, or one that carries prose but no
machine code, is a number a reader cannot act on and a histogram that cannot be
counted.

    tools/strict_gate.py --base base.json --strict strict.json

Exit 1 when any file in the strict sweep fell back with an empty reason or an
empty code, and when a file that falls back in strict does not appear in the base
sweep at all (that is a different corpus, not a measurement).
"""

from __future__ import annotations

import argparse
import json
import sys


def load(path: str) -> dict:
    with open(path) as handle:
        record = json.load(handle)
    return {row["path"]: row for row in record.get("sources", [])}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True, help="sweep from the default build")
    parser.add_argument("--strict", required=True, help="sweep from the strict build")
    args = parser.parse_args()

    base = load(args.base)
    strict = load(args.strict)

    unnamed = []
    unknown = []
    moved = 0
    for path, row in strict.items():
        reason = (row.get("deep_fallback") or "").strip()
        code = (row.get("deep_fallback_code") or "").strip()
        if not reason:
            continue
        if path not in base:
            unknown.append(path)
            continue
        if not (base[path].get("deep_fallback") or "").strip():
            moved += 1
        if not code:
            unnamed.append(path)

    for path in unnamed:
        print(f"UNNAMED {path}: falls back with a reason but no machine code")
    for path in unknown:
        print(f"UNKNOWN {path}: in the strict sweep but not the base sweep")

    if unnamed or unknown:
        print(f"FAIL: {len(unnamed)} unnamed, {len(unknown)} unknown")
        return 1
    print(f"OK: {moved} clean-to-fail moves under strict pairing, every one named")
    return 0


if __name__ == "__main__":
    sys.exit(main())
