#!/usr/bin/env python3
"""Replay a corpus subset through both readers and fail when a file loses a row.

The event model is measured against the corpus, so a change that trades one
construct for another is invisible in the totals: five files gain a symbol, five
lose an event, the count looks flat and the graph is worse. This harness keeps
the per-file counts instead, for both readers, and treats any decrease as a
failure — the row was written before and is not any more.

    tools/event_regression.py --corpus /tmp/js50 --baseline tools/baselines/event_js50.json
    tools/event_regression.py --corpus /tmp/js50 --baseline ... --update

Exit status is 1 when a file lost a row, 2 when the run itself failed. Writes
nothing but the baseline (and only under --update).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from collections import Counter, defaultdict

MODELS = ("symbols", "edges", "events", "imports")


def run_binary(binary: str, corpus: str, mode: str) -> dict:
    """One build of the corpus, reduced to per-file row counts."""
    proc = subprocess.run(
        [binary, "build", corpus, mode, "--no-cache", "--stdout"],
        capture_output=True,
        text=True,
        timeout=1800,
    )
    if proc.returncode != 0:
        raise SystemExit(f"{binary} build {corpus} {mode} failed: {proc.stderr[-400:]}")
    try:
        graph = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"{mode}: output is not JSON: {exc}")

    counts: dict[str, Counter] = defaultdict(Counter)
    for row in graph.get("symbols") or []:
        counts[row.get("file", "?")]["symbols"] += 1
    for row in graph.get("imports") or []:
        counts[row.get("file", "?")]["imports"] += 1
    for row in graph.get("edges") or []:
        owner = (row.get("caller") or row.get("callee") or "?").split(":", 1)[0]
        counts[owner]["edges"] += 1
    for row in graph.get("flows") or []:
        owner = (row.get("symbol") or "?").split(":", 1)[0]
        counts[owner]["events"] += len(row.get("events") or [])
    return {path: dict(counter) for path, counter in counts.items()}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--binary", default=os.environ.get("RUST_GRAPH_BIN", "rust-graph"))
    parser.add_argument("--update", action="store_true", help="write the baseline instead of checking")
    args = parser.parse_args()

    current = {"deep": run_binary(args.binary, args.corpus, "--deep"),
               "lexer": run_binary(args.binary, args.corpus, "--lexer")}

    if args.update:
        os.makedirs(os.path.dirname(os.path.abspath(args.baseline)), exist_ok=True)
        with open(args.baseline, "w") as handle:
            json.dump(current, handle, indent=1, sort_keys=True)
            handle.write("\n")
        files = len(current["deep"])
        print(f"baseline written: {args.baseline} ({files} files)")
        return 0

    try:
        with open(args.baseline) as handle:
            baseline = json.load(handle)
    except FileNotFoundError:
        print(f"no baseline at {args.baseline}; run with --update first", file=sys.stderr)
        return 2

    lost = []
    for mode in ("deep", "lexer"):
        for path, want in baseline.get(mode, {}).items():
            got = current[mode].get(path, {})
            for model in MODELS:
                before = want.get(model, 0)
                after = got.get(model, 0)
                if after < before:
                    lost.append((mode, path, model, before, after))

    for mode, path, model, before, after in lost:
        print(f"LOST {mode:5} {path} {model} {before} -> {after}")
    if lost:
        print(f"FAIL: {len(lost)} row counts decreased")
        return 1

    total = sum(sum(c.values()) for c in current["deep"].values())
    print(f"OK: {len(current['deep'])} files, {total} deep rows, no file lost a row")
    return 0


if __name__ == "__main__":
    sys.exit(main())
