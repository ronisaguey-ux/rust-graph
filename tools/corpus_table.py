#!/usr/bin/env python3
"""corpus_table.py — publish the 45-language table from the readers themselves.

The published table used to be typed by hand, which meant it was stale from the
first commit after the measurement. This runs the binary over the 45-language
corpus and over the local corpora, groups the rows by language, and writes
`docs/corpus45-table.md` with the numbers and the reason histogram of the
fallbacks it found.

    python3 tools/corpus_table.py            # measure everything and write the doc
    python3 tools/corpus_table.py --local    # re-measure the local corpora only
    python3 tools/corpus_table.py --check    # measure, compare, do not write

`--check` exits 1 when the document on disk no longer matches what the corpus
says, so a coverage fix that moves the number cannot land beside an old table.

A directory that crashes or does not finish is reported as such in the document
instead of as a row of files: a reader that dies on a corpus is a finding, and
counting the failure as one file would hide it under a plausible number.
"""
from __future__ import annotations

import argparse
import collections
import datetime
import json
import os
import subprocess
import sys
from pathlib import Path

BINARY = Path(
    os.environ.get(
        "RG_BIN",
        "/home/roni/Roni_workspace/rg-kbuilds/verify/x86_64-unknown-linux-musl/debug/rust-graph",
    )
)
WIDE = Path(os.environ.get("CORPUS45", "/home/roni/Roni_workspace/corpus45"))
LOCAL = Path(os.environ.get("RG_SWEEP_ROOT", "/tmp/opencode/sweep-local"))
DOC = Path(__file__).parent.parent / "docs" / "corpus45-table.md"
CST_LANGUAGES = ("JS", "TS", "PY", "RS")
LOCAL_HEADING = "Four-corpus comparison (larger local corpora, same binary):"
HISTOGRAM_HEADING = "Reason histogram, all 45 languages:"


def count_files(directory: Path) -> int:
    return sum(1 for path in directory.rglob("*") if path.is_file())


def budget(seconds: int, files: int) -> int:
    """A clock that scales with the corpus.

    One flat number is either too tight for a two-thousand-file directory or so
    loose that a reader stuck on a single file is never caught. A second of
    slack plus half a second per file was enough for every directory that
    finished, and still an order of magnitude above what a healthy reader needs.
    """
    return max(seconds, 60 + files // 2)


def measure(directory: Path) -> tuple[list[dict], str | None]:
    """Rows for one directory, and a note when the directory was not read."""
    limit = budget(int(os.environ.get("RG_TABLE_TIMEOUT", "120")), count_files(directory))
    try:
        result = subprocess.run(
            [str(BINARY), "build", str(directory), "--no-cache", "--stdout"],
            capture_output=True,
            text=True,
            timeout=limit,
        )
    except subprocess.TimeoutExpired:
        # A reader that does not finish is a worse bug than one that crashes: it
        # is invisible unless something puts a clock on it, so the table does.
        print(f"corpus_table: {directory} did not finish in {limit}s", file=sys.stderr)
        return [], f"`{directory.name}` did not finish in {limit}s"
    if result.returncode != 0:
        print(f"corpus_table: {directory} exited {result.returncode}", file=sys.stderr)
        return [], f"`{directory.name}` exited {result.returncode}"
    return json.loads(result.stdout)["sources"], None


def wide_rows() -> tuple[list[tuple[str, int, int]], collections.Counter, int, list[str]]:
    # One directory at a time: the whole tree in one run is fifty thousand files
    # and one bad file takes every row with it.
    rows: list[dict] = []
    notes: list[str] = []
    for directory in sorted(path for path in WIDE.iterdir() if path.is_dir()):
        found, note = measure(directory)
        rows.extend(found)
        if note:
            notes.append(note)
    per_language: dict[str, list[int]] = {}
    reasons: collections.Counter = collections.Counter()
    for row in rows:
        tag = row.get("lang", "?")
        bucket = per_language.setdefault(tag, [0, 0])
        bucket[0] += 1
        if "deep_fallback" in row:
            bucket[1] += 1
            code = row.get("deep_fallback_code", "unknown")
            reasons[code.split(":")[0]] += 1
    table = sorted(((tag, files, falls) for tag, (files, falls) in per_language.items()), key=lambda row: -row[2])
    return table, reasons, sum(row[1] for row in table), notes


def local_rows() -> tuple[list[tuple[str, int, int]], list[str]]:
    out = []
    notes: list[str] = []
    for name in sorted(path.name for path in LOCAL.iterdir() if path.is_dir()):
        rows, note = measure(LOCAL / name)
        if note:
            notes.append(note)
        out.append((name, len(rows), sum(1 for row in rows if "deep_fallback" in row)))
    return out, notes


def render_wide(table, reasons, files, notes) -> str:
    today = datetime.date.today().isoformat()
    cst = [row for row in table if row[0] in CST_LANGUAGES]
    rest_files = sum(row[1] for row in table if row[0] not in CST_LANGUAGES)
    lines = [
        f"# rust-graph — 45-language corpus table (generated {today})",
        "",
        f"Corpora: `{WIDE}` ({files} files) and `{LOCAL}`.",
        "Engine: `rust-graph build <dir> --no-cache --stdout`.",
        "Generated by `tools/corpus_table.py`: the numbers below are the readers' own, so they move with the work.",
        "",
        "| language | files | fallbacks |",
        "|---|---|---|",
    ]
    lines += [f"| {tag} | {count} | {falls} |" for tag, count, falls in cst]
    lines.append(f"| all other languages (no CST reader) | {rest_files} | 0 |")
    if notes:
        lines += ["", "Not read: " + "; ".join(notes) + "."]
    lines += ["", HISTOGRAM_HEADING if reasons else "", ""]
    if reasons:
        lines += [f"- {code}: {count}" for code, count in reasons.most_common()]
        lines.append(f"- total: {sum(reasons.values())}")
    return "\n".join(line for line in lines if line is not None)


def render_local(local, notes) -> str:
    lines = [
        LOCAL_HEADING,
        "",
        "| corpus | files | fallbacks |",
        "|---|---|---|",
    ]
    lines += [f"| {name} | {count} | {falls} |" for name, count, falls in local]
    if notes:
        lines += ["", "Not read: " + "; ".join(notes) + "."]
    return "\n".join(lines)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="compare against the document, do not write")
    parser.add_argument("--local", action="store_true", help="re-measure the local corpora only")
    args = parser.parse_args(argv)

    if not BINARY.exists():
        print(f"corpus_table: no binary at {BINARY}", file=sys.stderr)
        return 2
    if not WIDE.is_dir():
        print(f"corpus_table: no wide corpus at {WIDE}", file=sys.stderr)
        return 2

    local, local_notes = local_rows()

    if args.local:
        if not DOC.exists():
            print(f"corpus_table: {DOC} does not exist yet; run without --local", file=sys.stderr)
            return 2
        document = DOC.read_text()
        start = document.index(LOCAL_HEADING)
        end = document.find(HISTOGRAM_HEADING, start)
        tail = document[end:] if end >= 0 else ""
        text = document[:start] + render_local(local, local_notes) + ("\n\n" + tail if tail else "\n")
    else:
        table, reasons, files, notes = wide_rows()
        text = render_wide(table, reasons, files, notes) + "\n\n" + render_local(local, local_notes) + "\n"

    if args.check:
        if not DOC.exists() or DOC.read_text() != text:
            print(f"corpus_table: {DOC} is stale; run tools/corpus_table.py", file=sys.stderr)
            return 1
        print(f"corpus_table: {DOC} matches the corpora")
        return 0
    DOC.parent.mkdir(parents=True, exist_ok=True)
    DOC.write_text(text)
    print(f"wrote {DOC}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
