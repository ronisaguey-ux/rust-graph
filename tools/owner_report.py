#!/usr/bin/env python3
"""owner_report.py — compare the two readers' symbol tables file by file.

P5-12 exits when a trait implementation and an impl block record the owner the
way the lexer does. `pipeline::loses` answers "did either reader lose a fact";
this answers the narrower question that hid inside it: for every file both
readers parse, does each symbol carry the same name, kind, line and *owner*?

The owner is the class frame a function sits in. An `impl Trait for Type` frame
is named `Type` with `Trait` as its base, in both readers, because a method's
edges hang off it.

    python3 tools/owner_report.py                 # measure and report
    python3 tools/owner_report.py --save          # write the baseline
    python3 tools/owner_report.py --check         # fail when the tables drift
    python3 tools/owner_report.py --deep-dir DIR --lexer-dir DIR   # read existing records

The baseline records only the counts, so a reader improvement moves them and
`--check` says so rather than silently passing.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).parent.parent
BASELINE = ROOT / "tools" / "baselines" / "owners.json"
CORPUS = Path(os.environ.get("RG_SWEEP_ROOT", "/tmp/opencode/sweep-local")) / "rust"
DEFAULT_BINARY = Path(
    os.environ.get(
        "RG_BIN",
        "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph",
    )
)


def build(binary: Path, mode: str, out: Path) -> Path:
    flag = "--deep" if mode == "deep" else "--lexer"
    result = subprocess.run([str(binary), "build", str(CORPUS), "--no-cache", flag, "--out", str(out)], capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(f"owner_report: {mode} build exited {result.returncode}: {result.stderr[:200]}")
    return out / "graph.json"


def tables(graph: Path) -> dict[str, dict[tuple, str]]:
    rows = json.loads(graph.read_text())["symbols"]
    out: dict[str, dict[tuple, str]] = {}
    for row in rows:
        key = (row["name"], row["kind"], row["line"])
        out.setdefault(row["file"], {})[key] = row.get("owner", "")
    return out


def compare(deep: dict, lexer: dict) -> dict:
    files_only_deep = [f for f in deep if f not in lexer]
    files_only_lexer = [f for f in lexer if f not in deep]
    owner_mismatch: list[tuple[str, tuple, str, str]] = []
    rows_only_deep = 0
    rows_only_lexer = 0
    for file in deep:
        if file not in lexer:
            continue
        for key, owner in deep[file].items():
            if key not in lexer[file]:
                rows_only_deep += 1
            elif lexer[file][key] != owner:
                owner_mismatch.append((file, key, owner, lexer[file][key]))
        rows_only_lexer += sum(1 for key in lexer[file] if key not in deep[file])
    return {
        "files": len(deep),
        "files_only_deep": len(files_only_deep),
        "files_only_lexer": len(files_only_lexer),
        "rows_only_deep": rows_only_deep,
        "rows_only_lexer": rows_only_lexer,
        "owner_mismatches": len(owner_mismatch),
        "samples": [
            {"file": file, "name": key[0], "kind": key[1], "line": key[2], "deep": deep_owner, "lexer": lexer_owner}
            for file, key, deep_owner, lexer_owner in owner_mismatch[:20]
        ],
    }


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--deep-dir", type=Path)
    parser.add_argument("--lexer-dir", type=Path)
    parser.add_argument("--save", action="store_true")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args(argv)

    if args.deep_dir and args.lexer_dir:
        deep_graph = args.deep_dir / "graph.json"
        lexer_graph = args.lexer_dir / "graph.json"
    else:
        if not args.binary.exists():
            print(f"owner_report: no binary at {args.binary}", file=sys.stderr)
            return 2
        if not CORPUS.is_dir():
            print(f"owner_report: no corpus at {CORPUS}", file=sys.stderr)
            return 2
        work = Path(tempfile.mkdtemp(prefix="owner-report-"))
        deep_graph = build(args.binary, "deep", work / "deep")
        lexer_graph = build(args.binary, "lexer", work / "lexer")

    record = compare(tables(deep_graph), tables(lexer_graph))
    print(
        "owner_report: {files} files, {rows_only_deep} symbol(s) only the CST has, "
        "{rows_only_lexer} only the lexer, {owner_mismatches} owner mismatch(es)".format(**record)
    )
    for sample in record["samples"]:
        print(
            f"  {sample['file']}:{sample['line']} {sample['kind']} {sample['name']!r} "
            f"cst owner {sample['deep']!r} lexer owner {sample['lexer']!r}",
            file=sys.stderr,
        )
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(record, indent=2) + "\n")
        print(f"owner_report: wrote {args.json}")

    if args.save:
        BASELINE.parent.mkdir(parents=True, exist_ok=True)
        BASELINE.write_text(json.dumps({key: record[key] for key in ("files", "rows_only_deep", "rows_only_lexer", "owner_mismatches")}, indent=2) + "\n")
        print(f"owner_report: wrote {BASELINE.relative_to(ROOT)}")
    if args.check:
        if not BASELINE.exists():
            print(f"owner_report: no baseline at {BASELINE}; run --save", file=sys.stderr)
            return 2
        pinned = json.loads(BASELINE.read_text())
        drift = {key: (pinned.get(key), record[key]) for key in pinned if pinned[key] != record[key]}
        for key, (was, now) in drift.items():
            print(f"  {key}: {was} -> {now}", file=sys.stderr)
        return 1 if drift else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
