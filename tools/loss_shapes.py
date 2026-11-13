#!/usr/bin/env python3
"""Cluster the guard's losses so the next fix is picked by size, not by hunch.

A file-level fallback count says how many files lost something; the loss dump
(`RG_LOSSES=1`) says *what* each loss wanted. This runs the binary over a corpus,
reads every loss line, and groups them two ways:

- by the wanted value's first two atoms — the shape the CST failed to build;
- by the first word-ish characters of the source line the event sits on — the
  syntax the lexer was reading when it wrote that event.

The two together are what a fix is chosen from: a cluster that is 20% of a
corpus is a construct, and the line head says which construct.

    tools/loss_shapes.py                        # every corpus under --root
    tools/loss_shapes.py --corpus ts --top 12

Gate: the totals printed here must equal `tools/loss_count.py`'s loss column for
the same corpus, because both read the same lines from the same guard.
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import re
import subprocess
import sys

CORPORA = ("js", "ts", "python", "rust")
DEFAULT_ROOT = "/tmp/opencode/sweep-local"
DEFAULT_BIN = "/home/roni/Roni_workspace/rg-kbuilds/verify/x86_64-unknown-linux-musl/debug/rust-graph"

LOSS = re.compile(r"^loss\t(?P<path>[^\t]+)\t(?P<kind>\w+)\t(?P<detail>.*)$")
LINE = re.compile(r"line (?P<line>\d+)")
VALUE = re.compile(r"wanted line \d+ targets ?\S* value (?P<value>.*?); found")
CLAUSE = re.compile(r"\): (?P<clause>[^;]+)")


def corpus_files(root: str) -> list[str]:
    """Every file under the corpus root, relative names as the guard prints them."""
    out = []
    for base, _, files in os.walk(root):
        for name in files:
            path = os.path.join(base, name)
            out.append(os.path.relpath(path, root))
    return sorted(out)


def scan(binary: str, corpus: str) -> tuple[int, list[dict]]:
    result = subprocess.run(
        [binary, "build", corpus, "--no-cache", "--stdout"],
        capture_output=True,
        text=True,
        env={**os.environ, "RG_LOSSES": "1"},
    )
    rows = []
    for line in result.stderr.splitlines():
        match = LOSS.match(line)
        if not match:
            continue
        detail = match.group("detail")
        line_number = LINE.search(detail)
        value = VALUE.search(detail)
        clause = CLAUSE.search(detail)
        rows.append(
            {
                "path": match.group("path"),
                "kind": match.group("kind"),
                "line": int(line_number.group("line")) if line_number else 0,
                "value": value.group("value") if value else "",
                "clause": clause.group("clause").strip() if clause else detail[:60],
            }
        )
    return len(rows), rows


def atoms(value: str, take: int = 2) -> str:
    """The first `take` atoms of a shape value: `m:[n:foo,…]` -> `m: n:foo`."""
    parts = re.split(r"[\[\],]", value)
    kept = [part.strip() for part in parts if part.strip()][:take]
    return " ".join(kept) if kept else "(empty)"


def head(source: str, line: int) -> str:
    """The first token-ish characters of the line: what the lexer was reading."""
    lines = source.splitlines()
    if not (0 < line <= len(lines)):
        return "(no line)"
    text = lines[line - 1].strip()
    out = []
    for char in text:
        if char.isalnum() or char in "_$.:()[]{}<>=|&?":
            out.append(char)
        elif out and char.isspace():
            break
        elif char.isspace():
            continue
        else:
            break
    head_text = "".join(out)
    return head_text[:18] if head_text else "(punctuation)"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=os.environ.get("RG_CORPORA", DEFAULT_ROOT))
    parser.add_argument("--binary", default=os.environ.get("RG_BIN", DEFAULT_BIN))
    parser.add_argument("--corpus", action="append", choices=CORPORA)
    parser.add_argument("--top", type=int, default=8)
    args = parser.parse_args()

    if not os.path.exists(args.binary):
        print(f"no binary at {args.binary} (build it, or set RG_BIN)", file=sys.stderr)
        return 1

    for name in args.corpus or list(CORPORA):
        corpus = os.path.join(args.root, name)
        if not os.path.isdir(corpus):
            print(f"{corpus} is not a directory", file=sys.stderr)
            continue
        sources = {rel: open(os.path.join(corpus, rel), errors="replace").read() for rel in corpus_files(corpus)}
        total, rows = scan(args.binary, corpus)
        shapes = collections.Counter(atoms(row["value"]) for row in rows)
        heads = collections.Counter(head(sources.get(row["path"], ""), row["line"]) for row in rows)
        pairs = collections.Counter((atoms(row["value"]), head(sources.get(row["path"], ""), row["line"])) for row in rows)

        print(f"== {name}: {total} losses over {len({row['path'] for row in rows})} files")
        print("   by wanted shape:")
        for shape, count in shapes.most_common(args.top):
            print(f"     {count:5}  {count * 100 / max(total, 1):5.1f}%  {shape}")
        print("   by line head:")
        for head_text, count in heads.most_common(args.top):
            print(f"     {count:5}  {count * 100 / max(total, 1):5.1f}%  {head_text}")
        print("   by shape + head:")
        for (shape, head_text), count in pairs.most_common(args.top):
            print(f"     {count:5}  {count * 100 / max(total, 1):5.1f}%  {shape}   @ {head_text}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
