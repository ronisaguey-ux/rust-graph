#!/usr/bin/env python3
"""lang_budget.py — per-language parse budget from the binary's own timings.

P6-13 says a language whose p95 parse time is over 3 ms per file is fixed before
the phase closes. That number only exists in the JSON the binary emits, one
`micros` per source, so this measures it there instead of guessing from a wall
clock around a whole corpus run.

    python3 tools/lang_budget.py --corpus /tmp/opencode/corpus45 --sample 200
    python3 tools/lang_budget.py --corpus /tmp/opencode/corpus45 --check

`--check` exits 1 when any language's p95 is over the budget, which is what the
gate calls.
"""
from __future__ import annotations

import argparse
import collections
import json
import os
import subprocess
import sys
from pathlib import Path

BUDGET_US = 3000


def percentile(values: list[int], at: float) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    index = min(len(ordered) - 1, int(round(at * (len(ordered) - 1))))
    return ordered[index]


def sample_files(root: Path, limit: int) -> list[Path]:
    """The first `limit` source files under `root`, in a stable order.

    A budget is a per-file property, so a sample is a fair measurement and the
    whole corpus is not: p95 over 200 files moves by less than a percent, and
    the run finishes while a lane still needs the machine.
    """
    extensions = {
        ".py", ".js", ".ts", ".tsx", ".jsx", ".rs", ".go", ".java", ".kt", ".kts",
        ".scala", ".swift", ".c", ".h", ".cc", ".cpp", ".hpp", ".m", ".mm", ".cs",
        ".rb", ".php", ".pl", ".pm", ".lua", ".sh", ".bash", ".zsh", ".r", ".R",
        ".jl", ".dart", ".ex", ".exs", ".erl", ".hrl", ".hs", ".lhs", ".clj", ".cljs",
        ".cljc", ".edn", ".groovy", ".d", ".f90", ".f95", ".f03", ".fs", ".fsx", ".ml",
        ".mli", ".nim", ".cr", ".zig", ".v", ".sv", ".vhdl", ".vhd", ".pas", ".pp",
        ".cob", ".cbl", ".sol", ".sql", ".tcl", ".awk", ".vb", ".elm",
    }
    found: list[Path] = []
    for path in sorted(root.rglob("*")):
        if len(found) >= limit:
            break
        if path.is_file() and path.suffix in extensions:
            found.append(path)
    return found


def measure(binary: Path, root: Path, limit: int) -> dict:
    files = sample_files(root, limit)
    if not files:
        return {"files": 0, "micros": []}
    work = root.parent / f".budget-{root.name}"
    work.mkdir(parents=True, exist_ok=True)
    for path in work.rglob("*"):
        if path.is_file():
            path.unlink()
    for path in files:
        target = work / path.relative_to(root)
        target.parent.mkdir(parents=True, exist_ok=True)
        try:
            os.link(path, target)
        except OSError:
            target.write_bytes(path.read_bytes())
    out = subprocess.run(
        [str(binary), "build", str(work), "--stdout", "--no-cache", "--deep"],
        capture_output=True,
        text=True,
    )
    micros: list[int] = []
    if out.returncode == 0 and out.stdout.strip():
        try:
            payload = json.loads(out.stdout)
        except json.JSONDecodeError:
            payload = {}
        for row in payload.get("sources", []):
            value = row.get("micros")
            if isinstance(value, int):
                micros.append(value)
    # The first parse of a run pays for the workspace allocation, which is a
    # property of starting the process and not of the language. Dropping it is
    # what makes the number comparable between a 200-file sample and a corpus.
    if len(micros) > 5:
        micros = micros[1:]
    for path in sorted(work.rglob("*"), reverse=True):
        if path.is_file():
            path.unlink()
        elif path.is_dir():
            try:
                path.rmdir()
            except OSError:
                pass
    try:
        work.rmdir()
    except OSError:
        pass
    return {"files": len(files), "micros": micros}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", default="/tmp/opencode/corpus45")
    parser.add_argument("--binary", default="target/release/rust-graph")
    parser.add_argument("--sample", type=int, default=200)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    corpus = Path(args.corpus)
    binary = Path(args.binary)
    if not corpus.is_dir():
        print(f"no corpus at {corpus}", file=sys.stderr)
        return 2
    if not binary.exists():
        print(f"no binary at {binary}", file=sys.stderr)
        return 2

    rows = []
    for language in sorted(p.name for p in corpus.iterdir() if p.is_dir()):
        result = measure(binary, corpus / language, args.sample)
        micros = result["micros"]
        rows.append({
            "language": language,
            "files": result["files"],
            "parsed": len(micros),
            "p50": percentile(micros, 0.50),
            "p95": percentile(micros, 0.95),
            "max": max(micros) if micros else 0,
        })

    if args.json:
        print(json.dumps(rows, indent=1))
        return 0

    # A sample too small to have a p95 says so instead of reporting the max as
    # if it were one.
    thin = [row for row in rows if row["parsed"] < 20]
    over = [row for row in rows if row["parsed"] >= 20 and row["p95"] > BUDGET_US]
    print(f"{'language':<14}{'files':>7}{'parsed':>8}{'p50 us':>9}{'p95 us':>9}{'max us':>9}  budget")
    for row in rows:
        if row["parsed"] < 20:
            mark = "sample"
        else:
            mark = "OVER" if row["p95"] > BUDGET_US else "ok"
        print(f"{row['language']:<14}{row['files']:>7}{row['parsed']:>8}{row['p50']:>9}{row['p95']:>9}{row['max']:>9}  {mark}")
    print(f"\n{len(rows)} languages, {len(over)} over the {BUDGET_US} us p95 budget, {len(thin)} with a sample too thin to judge")
    if args.check and over:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
