#!/usr/bin/env python3
"""commit_audit.py — did the merge fix land alone?

P5-19 asks for one thing: a regression after the arm-merge change must be
attributable to that change, not to a bundle it travelled in. That is a property
of history, so the check reads history instead of trusting a convention:

  * the commit touches the pinned files and nothing else,
  * every file it touches is production code (`src/`), so a test, a baseline and
    a document did not ride along,
  * the change is small enough to be one idea,
  * the commit says what it did, and
  * the corpus measurement that justified it was taken with the fix already in
    history (the measured commit has the fix as an ancestor).

    python3 tools/commit_audit.py            # print the audit
    python3 tools/commit_audit.py --check    # exit 1 when the discipline broke

The pin lives in `tools/baselines/merge_fix.json`, so a reverted or replayed fix
is re-pinned deliberately rather than drifting with `HEAD`.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
PIN = ROOT / "tools" / "baselines" / "merge_fix.json"


def git(*args: str) -> str:
    result = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(f"commit_audit: git {' '.join(args)} failed: {result.stderr.strip()[:200]}")
    return result.stdout


def exists(rev: str) -> bool:
    result = subprocess.run(["git", "cat-file", "-e", f"{rev}^{{commit}}"], cwd=ROOT, capture_output=True)
    return result.returncode == 0


def audit(pin: dict) -> list[str]:
    problems: list[str] = []
    sha = pin["commit"]
    if not exists(sha):
        return [f"the pinned commit {sha[:12]} is not in this history"]
    subject = git("show", "-s", "--format=%s", sha).strip()
    if subject != pin["subject"]:
        problems.append(f"subject changed: {subject!r} is not {pin['subject']!r}")
    body = git("show", "-s", "--format=%b", sha).strip()
    if not body:
        problems.append("the commit has no body saying what it changed")
    numstat = [line.split("\t") for line in git("show", "--numstat", "--format=", sha).splitlines() if line.strip()]
    files = [row[2] for row in numstat]
    added = sum(int(row[0]) for row in numstat if row[0].isdigit())
    removed = sum(int(row[1]) for row in numstat if row[1].isdigit())
    if sorted(files) != sorted(pin["files"]):
        problems.append(f"files touched {files} are not the pinned {pin['files']}")
    for file in files:
        if not file.startswith("src/"):
            problems.append(f"{file} is not production code: a test, a baseline or a document rode along")
    if added + removed > pin["max_lines"]:
        problems.append(f"{added + removed} lines changed, over the pinned {pin['max_lines']}: this is not one idea")
    measured = pin["measured_by"]
    if exists(measured):
        ancestor = subprocess.run(
            ["git", "merge-base", "--is-ancestor", sha, measured], cwd=ROOT, capture_output=True
        )
        if ancestor.returncode != 0:
            problems.append(f"the measurement at {measured[:12]} was not taken with this fix in history")
    else:
        problems.append(f"the measuring commit {measured[:12]} is not in this history")
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--pin", type=Path, default=PIN)
    args = parser.parse_args(argv)

    if not args.pin.exists():
        print(f"commit_audit: no pin at {args.pin}", file=sys.stderr)
        return 2
    pin = json.loads(args.pin.read_text())
    problems = audit(pin)
    print(f"commit_audit: {pin['commit'][:12]} {pin['subject']}")
    print(f"  files {', '.join(pin['files'])}; measured by {pin['measured_by'][:12]}")
    if problems:
        for line in problems:
            print(f"  BROKE: {line}", file=sys.stderr)
        return 1 if args.check else 0
    print("  the merge fix landed alone")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
