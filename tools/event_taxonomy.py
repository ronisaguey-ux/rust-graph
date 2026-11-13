#!/usr/bin/env python3
"""Log every file where the two readers disagree on the NUMBER of events.

The fallback reasons (`the CST lost a event`) fold this into one bucket, and the
bucket says nothing: a file can differ by one event or by two hundred, the CST
can be the heavier writer or the lexer can, and the fix for one is not the fix
for the other. This tool keeps them apart — one taxonomy entry per
(language, direction, what else disagrees on that file) with the file list and a
count, so an entry can be taken on its own instead of "events" being one job.

    tools/event_taxonomy.py --corpus /tmp/js50 --lang js [--json out.json]

Prints a table and, with --json, writes the entries for the record.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections import defaultdict

MODELS = ("symbols", "edges", "events", "imports")


def build(binary: str, corpus: str, mode: str) -> tuple[str, dict]:
    proc = subprocess.run(
        [binary, "build", corpus, mode, "--no-cache", "--stdout"],
        capture_output=True, text=True, timeout=1800,
    )
    if proc.returncode != 0:
        raise SystemExit(f"{mode}: build failed: {proc.stderr[-400:]}")
    return proc.stdout, json.loads(proc.stdout)


def per_file(graph: dict) -> dict:
    files: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    for row in graph.get("sources") or []:
        files[row["path"]]  # every file is present, even a silent one
    for row in graph.get("symbols") or []:
        files[row.get("file", "?")]["symbols"] += 1
    for row in graph.get("imports") or []:
        files[row.get("file", "?")]["imports"] += 1
    for row in graph.get("edges") or []:
        files[(row.get("caller") or row.get("callee") or "?").split(":", 1)[0]]["edges"] += 1
    for row in graph.get("flows") or []:
        files[(row.get("symbol") or "?").split(":", 1)[0]]["events"] += len(row.get("events") or [])
    return {path: dict(counts) for path, counts in files.items()}


def entry_key(lang: str, delta: int, deep: dict, lexer: dict) -> str:
    """The taxonomy entry: who wrote more, and what else moved with it."""
    direction = "CST_writes_more" if delta > 0 else "lexer_writes_more"
    also = []
    for model in ("symbols", "edges", "imports"):
        if deep.get(model, 0) != lexer.get(model, 0):
            also.append(model)
    tag = "+".join(also) if also else "events_only"
    size = "large" if abs(delta) >= 20 else "small"
    return f"{lang}/{direction}/{size}/{tag}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--lang", required=True)
    parser.add_argument("--binary", default="rust-graph")
    parser.add_argument("--json")
    args = parser.parse_args()

    _, deep_graph = build(args.binary, args.corpus, "--deep")
    _, lexer_graph = build(args.binary, args.corpus, "--lexer")
    deep, lexer = per_file(deep_graph), per_file(lexer_graph)

    entries: dict[str, dict] = {}
    for path in sorted(set(deep) | set(lexer)):
        before = deep.get(path, {}).get("events", 0)
        after = lexer.get(path, {}).get("events", 0)
        if before == after:
            continue
        key = entry_key(args.lang, before - after, deep.get(path, {}), lexer.get(path, {}))
        entry = entries.setdefault(key, {"entry": key, "files": 0, "examples": [], "total_delta": 0})
        entry["files"] += 1
        entry["total_delta"] += before - after
        if len(entry["examples"]) < 3:
            entry["examples"].append({"path": path, "cst": before, "lexer": after})

    for key in sorted(entries):
        entry = entries[key]
        print(f"{key:52} files {entry['files']:3} delta {entry['total_delta']:+6}")
        for example in entry["examples"]:
            print(f"    {example['path']}  cst {example['cst']} / lexer {example['lexer']}")

    if args.json:
        with open(args.json, "w") as handle:
            json.dump(list(entries.values()), handle, indent=1, sort_keys=True)
            handle.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
