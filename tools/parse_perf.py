#!/usr/bin/env python3
"""parse_perf.py — per-file CST parse time on a corpus, against the 3 ms target.

The product's own per-file number (`"micros"` in a build row) is the *record*:
lexer facts, CST parse and facts extraction together. This measures the parse
alone — `rust-graph-deep-scan conformance --reps 1`, one pass over every file —
because that is the quantity the phase-5 target names, and mixing it with the
record would let an extraction change hide a parser regression.

    python3 tools/parse_perf.py                  # measure and report
    python3 tools/parse_perf.py --record         # write the baseline
    python3 tools/parse_perf.py --check          # fail over the p95 limit or the pin

The largest file is reported on its own line: a corpus p95 says nothing about
the file that is five times bigger than the rest, and that file is where a
quadratic shows up first.
"""
from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
BASELINE = ROOT / "tools" / "baselines" / "parse_perf_rust.json"
CORPUS = Path(os.environ.get("RG_SWEEP_ROOT", "/tmp/opencode/sweep-local")) / "rust"
SCAN = Path(
    os.environ.get(
        "RG_SCAN",
        "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph-deep-scan",
    )
)
P95_LIMIT_MS = 3.0
# The box runs a fleet of agents, so a single pass measures the scheduler as much
# as the parser. `--reps` keeps the best pass of each file; five is the smallest
# number that made the p95 repeatable across runs (1.5-5.8 ms at one rep).
REPS = 5


def measure(corpus: Path) -> dict:
    result = subprocess.run([str(SCAN), "conformance", str(corpus), "--reps", str(REPS)], capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(f"parse_perf: conformance exited {result.returncode}: {result.stderr[:200]}")
    rows = []
    for line in result.stdout.splitlines():
        fields = line.split("\t")
        if not fields or fields[0] != "file":
            continue
        row = {"path": fields[1] if len(fields) > 1 else ""}
        for field in fields[2:]:
            if "=" in field:
                key, value = field.split("=", 1)
                row[key] = value
        rows.append(row)
    if not rows:
        raise SystemExit("parse_perf: no file rows in the conformance output")
    times = sorted(float(row["ms"]) for row in rows)
    at = lambda share: times[min(len(times) - 1, int(len(times) * share))]
    biggest = max(rows, key=lambda row: int(row["bytes"]))
    return {
        "files": len(times),
        "mean_ms": round(statistics.fmean(times), 3),
        "p50_ms": round(at(0.50), 3),
        "p95_ms": round(at(0.95), 3),
        "p99_ms": round(at(0.99), 3),
        "max_ms": round(times[-1], 3),
        "largest": {
            "path": biggest["path"].rsplit("/", 1)[-1],
            "bytes": int(biggest["bytes"]),
            "nodes": int(biggest.get("nodes", 0)),
            "ms": round(float(biggest["ms"]), 3),
        },
    }


def main(argv: list[str]) -> int:
    global SCAN
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--corpus", type=Path, default=CORPUS)
    parser.add_argument("--scan", type=Path, default=SCAN)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args(argv)

    SCAN = args.scan
    if not SCAN.exists():
        print(f"parse_perf: no scan binary at {SCAN}", file=sys.stderr)
        return 2
    if not args.corpus.is_dir():
        print(f"parse_perf: no corpus at {args.corpus}", file=sys.stderr)
        return 2

    now = measure(args.corpus)
    largest = now["largest"]
    print(
        f"parse_perf: {now['files']} files, mean {now['mean_ms']} ms, p50 {now['p50_ms']}, "
        f"p95 {now['p95_ms']}, p99 {now['p99_ms']}, max {now['max_ms']}"
    )
    print(
        f"parse_perf: largest {largest['path']} {largest['bytes']} bytes {largest['nodes']} nodes "
        f"{largest['ms']} ms"
    )
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(now, indent=2) + "\n")
        print(f"parse_perf: wrote {args.json}")

    if args.record:
        BASELINE.parent.mkdir(parents=True, exist_ok=True)
        BASELINE.write_text(json.dumps(now, indent=2) + "\n")
        print(f"parse_perf: wrote {BASELINE.relative_to(ROOT)}")

    failed = []
    if now["p95_ms"] >= P95_LIMIT_MS:
        failed.append(f"p95 {now['p95_ms']} ms is not under {P95_LIMIT_MS} ms")
    if args.check:
        if not BASELINE.exists():
            print(f"parse_perf: no baseline at {BASELINE}; run --record", file=sys.stderr)
            return 2
        pinned = json.loads(BASELINE.read_text())
        for key in ("mean_ms", "p50_ms", "p95_ms", "p99_ms", "max_ms"):
            if key in pinned and now[key] > pinned[key] * 1.25:
                failed.append(f"{key} {pinned[key]} -> {now[key]} (limit +25%)")
    for line in failed:
        print(f"  {line}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
