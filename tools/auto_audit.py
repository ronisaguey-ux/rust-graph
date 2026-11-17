#!/usr/bin/env python3
"""The auto-mode audit: how many files a language still loses.

`build <corpus>` in auto mode compares the two readers per file and keeps the
one that wins, so a file only shows up here when the CST record and the lexer
record disagree. That count is what decides whether a language is allowed into
`deep_facts::lang_for` — the switch that lets the guard pay for a second parse.

Only four languages are in auto mode today. For the rest the audit sets
`RG_AUDIT=1`, which makes `lang_for` answer for every language this build can
parse without changing what a real build does.

    tools/auto_audit.py --lang c
    tools/auto_audit.py --lang c --dir /root/corpus45/c --json
    tools/auto_audit.py --lang c go java --root /root/corpus45

The denominator is the corpus's own file count, because the corpus build keeps
only files the lexer can read at all: a file the lexer refuses is not a
disagreement.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

DEFAULT_ROOT = "/tmp/opencode/sweep-local"
DEFAULT_BIN = "/tmp/rg-night3-target/x86_64-unknown-linux-musl/release/rust-graph"

# The corpus directory a language is measured from, for the names
# tools/corpus45/build_corpus.py uses.
DIRS = {
    "python": "python",
    "javascript": "js",
    "typescript": "ts",
    "rust": "rust",
    "c": "c",
    "cpp": "cpp",
    "csharp": "csharp",
    "go": "go",
    "java": "java",
    "kotlin": "kotlin",
    "scala": "scala",
    "swift": "swift",
    "objc": "objc",
}


def measure(binary: str, path: str) -> tuple[int, int, int]:
    env = dict(os.environ, RG_AUDIT="1")
    result = subprocess.run(
        [binary, "build", path, "--no-cache", "--stdout"],
        capture_output=True,
        check=True,
        env=env,
    )
    pack = json.loads(result.stdout)
    rows = pack["sources"]
    fallbacks = [row for row in rows if row.get("deep_fallback")]
    return len(fallbacks), sum(row.get("deep_losses", 0) for row in fallbacks), len(rows)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=os.environ.get("RG_CORPORA", DEFAULT_ROOT))
    parser.add_argument("--binary", default=os.environ.get("RG_BIN", DEFAULT_BIN))
    parser.add_argument("--dir", help="measure this directory instead of <root>/<lang>")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("lang", nargs="*", help="language name, as tools/corpus45 names it")
    parser.add_argument("--lang", action="append", dest="langs", help="repeatable; same as the positional names")
    args = parser.parse_args()
    args.lang = (args.lang or []) + (args.langs or [])
    if not args.lang:
        parser.error("name at least one language")

    if not os.path.exists(args.binary):
        print(f"no binary at {args.binary} (build it, or set RG_BIN)", file=sys.stderr)
        return 1
    if args.dir and len(args.lang) > 1:
        print("--dir takes one language", file=sys.stderr)
        return 1

    out = []
    for lang in args.lang:
        path = args.dir or os.path.join(args.root, DIRS.get(lang, lang))
        if not os.path.isdir(path):
            print(f"{lang}: no corpus at {path}", file=sys.stderr)
            return 1
        fallbacks, losses, total = measure(args.binary, path)
        out.append({"lang": lang, "dir": path, "files": total, "losses": losses, "fallbacks": fallbacks})

    if args.json:
        print(json.dumps(out, indent=1))
        return 0
    print(f"{'language':11} {'files':>7} {'losing':>7} {'losses':>7} {'rate':>7}")
    for row in out:
        rate = f"{100 * row['fallbacks'] / row['files']:.1f}%" if row["files"] else "-"
        print(f"{row['lang']:11} {row['files']:>7} {row['fallbacks']:>7} {row['losses']:>7} {rate:>7}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
