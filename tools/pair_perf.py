#!/usr/bin/env python3
"""pair_perf.py — per-file parse time and recovery count, for both readers.

A coverage fix makes files read *better*; it can also make them read slower, and
a corpus total hides that: two files doubling in cost vanish beside two thousand
unchanged ones. This records, for every file in a corpus, the microseconds it
took and how many `ERROR` nodes the CST needed, and compares the run against a
recorded baseline so a fix that buys a loss with throughput fails loudly.

    python3 tools/pair_perf.py --record          # write the baseline
    python3 tools/pair_perf.py                   # compare against it, exit 1 on regression

The thresholds are deliberately loose: parse time is noisy on a loaded box, so
the gate fires on a real regression (p95 up a quarter, or mean up a fifth, or
recoveries up five percent), not on jitter.
"""
from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(os.environ.get("RG_SWEEP_ROOT", "/tmp/opencode/sweep-local"))
BINARY = Path(os.environ.get("RG_BIN", "/home/roni/Roni_workspace/rg-kbuilds/verify/x86_64-unknown-linux-musl/debug/rust-graph"))
BASELINE = Path(__file__).parent / "baselines" / "perf_js_ts.json"
CORPORA = ("js", "ts")
P95_LIMIT = 1.25
MEAN_LIMIT = 1.20
RECOVERY_LIMIT = 1.05


def measure(corpus: Path) -> dict:
    result = subprocess.run(
        [str(BINARY), "build", str(corpus), "--no-cache", "--stdout"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit(f"pair_perf: the build failed on {corpus}: {result.stderr.strip()[:300]}")
    rows = json.loads(result.stdout)["sources"]
    micros = sorted(row["micros"] for row in rows if "micros" in row)
    recoveries = sum(row.get("recoveries", 0) for row in rows)
    if not micros:
        raise SystemExit(f"pair_perf: no timing in the rows for {corpus}")
    at = lambda share: micros[min(len(micros) - 1, int(len(micros) * share))]
    return {
        "files": len(micros),
        "micros_total": sum(micros),
        "micros_mean": round(statistics.fmean(micros), 2),
        "micros_p50": at(0.50),
        "micros_p95": at(0.95),
        "micros_max": micros[-1],
        "recoveries": recoveries,
    }


def compare(now: dict, before: dict) -> list[str]:
    broke = []
    if now["micros_p95"] > before["micros_p95"] * P95_LIMIT:
        broke.append(f"p95 {before['micros_p95']} -> {now['micros_p95']} us (limit +{int((P95_LIMIT - 1) * 100)}%)")
    if now["micros_mean"] > before["micros_mean"] * MEAN_LIMIT:
        broke.append(f"mean {before['micros_mean']} -> {now['micros_mean']} us (limit +{int((MEAN_LIMIT - 1) * 100)}%)")
    if before["recoveries"] and now["recoveries"] > before["recoveries"] * RECOVERY_LIMIT:
        broke.append(f"recoveries {before['recoveries']} -> {now['recoveries']} (limit +{int((RECOVERY_LIMIT - 1) * 100)}%)")
    return broke


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--baseline", type=Path, default=BASELINE)
    parser.add_argument("--record", action="store_true", help="write the baseline from this run")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    if not BINARY.exists():
        print(f"pair_perf: no binary at {BINARY} (build it, or set RG_BIN)", file=sys.stderr)
        return 2

    now = {name: measure(args.root / name) for name in CORPORA}
    if args.json:
        print(json.dumps(now, indent=2))
    else:
        for name, row in now.items():
            print(
                f"{name:3} files {row['files']:5}  mean {row['micros_mean']:8.1f} us  "
                f"p95 {row['micros_p95']:7} us  max {row['micros_max']:8} us  recoveries {row['recoveries']:6}"
            )

    if args.record:
        args.baseline.parent.mkdir(parents=True, exist_ok=True)
        args.baseline.write_text(json.dumps({"corpora": now}, indent=2, sort_keys=True) + "\n")
        print(f"pair_perf: baseline written to {args.baseline}")
        return 0

    if not args.baseline.exists():
        print(f"pair_perf: no baseline at {args.baseline} (run --record first)", file=sys.stderr)
        return 2
    before = json.loads(args.baseline.read_text())["corpora"]
    failed = []
    for name in CORPORA:
        if name not in before:
            continue
        for line in compare(now[name], before[name]):
            failed.append(f"{name}: {line}")
    if failed:
        for line in failed:
            print(f"pair_perf: regression  {line}", file=sys.stderr)
        return 1
    print("pair_perf: no throughput or recovery regression against the baseline")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
