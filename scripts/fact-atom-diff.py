#!/usr/bin/env python3
"""Show the first fact the CST record loses, atom by atom.

Runs `rust-graph build <dir> --stdout` twice (--lexer and --deep), pairs the
symbols, and for each symbol compares the two records the way the guard does:
targets as name sets, values as the multiset of names, calls and constants.
Prints the first difference per symbol with the offending atom, so a fallback
can be fixed from one line instead of a 200-character truncated message.

    ./fact-atom-diff.py <dir-or-file> [--bin path/to/rust-graph]

Constants (0) are only reported when the lexer has more of them than the CST.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

NAME = re.compile(r'\["n",\s*"([^"]*)"\]')
CALL = '["c",'
ZERO = re.compile(r'(?<![\w"])0(?![\w"])')


def run(binary: str, root: Path, mode: str) -> dict:
    out = subprocess.run(
        [binary, "build", str(root), "--stdout", "--no-cache", f"--{mode}"],
        capture_output=True, text=True, timeout=1800,
    ).stdout
    facts = {}
    for line in out.splitlines():
        line = line.strip().rstrip(",")
        if not line.startswith('{"symbol"'):
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        facts[row["symbol"]] = row
    return facts


def atoms(value) -> Counter:
    text = json.dumps(value)
    return Counter({"n:" + name for name in NAME.findall(text)}), \
        text.count(CALL), len(ZERO.findall(text))


def names_of(targets) -> frozenset:
    return frozenset(re.findall(r'"([^"]+)"', json.dumps(targets)))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("target")
    ap.add_argument("--bin", default=os.environ.get("RUST_GRAPH_BIN", "rust-graph"))
    args = ap.parse_args()

    target = Path(args.target)
    if target.is_file():
        tmp = Path(tempfile.mkdtemp(prefix="atom-diff-")) / target.name
        tmp.write_bytes(target.read_bytes())
        root = tmp.parent
    else:
        root = target

    lexer = run(args.bin, root, "lexer")
    deep = run(args.bin, root, "deep")
    shown = 0
    for symbol, lex in lexer.items():
        cst = deep.get(symbol)
        if cst is None:
            print(f"{symbol}: symbol missing from the CST record")
            shown += 1
            continue
        lex_events = [e for e in lex["events"] if e[0] in "arx"]
        cst_events = [e for e in cst["events"] if e[0] in "arx"]
        if len(cst_events) < len(lex_events):
            print(f"{symbol}: {len(lex_events)} events to the lexer, {len(cst_events)} to the CST")
            shown += 1
            continue
        for want, got in zip(lex_events, cst_events):
            if want[0] != got[0]:
                print(f"{symbol}: event tag {want[0]!r} vs {got[0]!r} at line {want[1]}")
                shown += 1
                break
            if want[0] == "a" and names_of(want[2]) != names_of(got[2]):
                print(f"{symbol}: line {want[1]} targets {sorted(names_of(want[2]))} vs {sorted(names_of(got[2]))}")
                shown += 1
                break
            wnames, wcalls, wzeros = atoms(want[3] if want[0] == "a" else want[2])
            gnames, gcalls, gzeros = atoms(got[3] if got[0] == "a" else got[2])
            missing = wnames - gnames
            if missing:
                print(f"{symbol}: line {want[1]} names missing from the CST value: {sorted(missing)}")
                shown += 1
                break
            if gnames - wnames:
                print(f"{symbol}: line {want[1]} names the CST added (harmless): {sorted(gnames - wnames)}")
                shown += 1
                break
            if gcalls < wcalls or gzeros < wzeros:
                print(f"{symbol}: line {want[1]} calls {wcalls}->{gcalls}, constants {wzeros}->{gzeros}")
                shown += 1
                break
    if shown == 0:
        print("no difference: the CST record covers every lexer fact in this tree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
