#!/usr/bin/env python3
"""phase4_gate.py — the Phase 4 exit gate, measured rather than asserted.

Phase 4 exits when:
  * JavaScript falls back on at most 150 files of its corpus,
  * TypeScript at most 120,
  * every remaining fallback is attributed — a file that falls back carries both
    a sentence and a machine code, and the code is not `unknown`,
  * no other language moved: Python and Rust are pinned to the count they had
    when the gate was written.

    python3 tools/phase4_gate.py                     # measure and judge
    python3 tools/phase4_gate.py --save              # re-pin python and rust
    python3 tools/phase4_gate.py --binary <path>

Exit 1 on any breached limit, so a regression in one language cannot be paid for
with a win in another without the gate saying so.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
BASELINE = ROOT / "tools" / "baselines" / "phase4_exit.json"
CORPORA = Path(os.environ.get("RG_SWEEP_ROOT", "/tmp/opencode/sweep-local"))
DEFAULT_BINARY = Path(
    os.environ.get(
        "RG_BIN",
        "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph",
    )
)
LIMITS = {"js": 150, "ts": 120}


def measure(binary: Path, corpus: Path) -> list[dict]:
    result = subprocess.run(
        [str(binary), "build", str(corpus), "--no-cache", "--stdout"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit(f"phase4_gate: {corpus} exited {result.returncode}: {result.stderr.strip()[:200]}")
    return json.loads(result.stdout)["sources"]


def report(binary: Path) -> tuple[dict[str, int], list[str]]:
    counts: dict[str, int] = {}
    problems: list[str] = []
    for name in sorted(LIMITS) + ["python", "rust"]:
        corpus = CORPORA / name
        if not corpus.is_dir():
            raise SystemExit(f"phase4_gate: no corpus at {corpus}")
        rows = measure(binary, corpus)
        fallen = [row for row in rows if "deep_fallback" in row]
        counts[name] = len(fallen)
        for row in fallen:
            reason = row.get("deep_fallback", "")
            code = row.get("deep_fallback_code", "")
            if not reason or not code:
                problems.append(f"{name}:{row.get('path')} fell back with no {'reason' if not reason else 'code'}")
            elif code.split(":")[0] == "unknown":
                problems.append(f"{name}:{row.get('path')} fell back with an unattributed code: {reason[:80]}")
    return counts, problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--save", action="store_true", help="re-pin python and rust to what the corpus says now")
    args = parser.parse_args(argv)

    if not args.binary.exists():
        print(f"phase4_gate: no binary at {args.binary}", file=sys.stderr)
        return 2

    counts, problems = report(args.binary)
    pinned = json.loads(BASELINE.read_text()) if BASELINE.exists() else {}
    if args.save:
        BASELINE.parent.mkdir(parents=True, exist_ok=True)
        BASELINE.write_text(json.dumps({"limits": LIMITS, "pinned": {k: counts[k] for k in ("python", "rust")}}, indent=2) + "\n")
        print(f"phase4_gate: pinned {BASELINE}")
        return 0

    failed = False
    for name, limit in LIMITS.items():
        verdict = "ok" if counts[name] <= limit else "OVER"
        failed |= counts[name] > limit
        print(f"  {name:<8} {counts[name]:>5} fallbacks   limit {limit:>4}   {verdict}")
    for name in ("python", "rust"):
        was = pinned.get("pinned", {}).get(name)
        if was is None:
            print(f"  {name:<8} {counts[name]:>5} fallbacks   not pinned")
            continue
        moved = counts[name] - was
        verdict = "unchanged" if moved == 0 else f"MOVED {moved:+d}"
        failed |= moved != 0
        print(f"  {name:<8} {counts[name]:>5} fallbacks   pinned {was:>5}   {verdict}")
    if problems:
        failed = True
        print(f"  attribution: {len(problems)} problem(s)", file=sys.stderr)
        for problem in problems[:10]:
            print(f"    {problem}", file=sys.stderr)
    else:
        print("  attribution: every fallback names its construct and its code")
    print("PHASE4 GATE", "FAILED" if failed else "PASSED")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
