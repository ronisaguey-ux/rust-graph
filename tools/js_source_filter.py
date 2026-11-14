#!/usr/bin/env python3
"""Filter a list of .js paths down to human-written source.

The js corpus used to be a plain `find | shuf` over every .js under the two home
trees, so a large share of it was built output: webpack chunk files, Next.js
`static/chunks`, `dist/`, `.min.js`. Those files are one enormous line, so
per-line pairing has nothing to compare and every loss inside them is noise that
no reader fix can remove (measured 2026-11-14: 164 losses over 96 files, 19 of
them in one bundled chunk's line 1).

Reads paths on stdin, writes the survivors to stdout.
"""
import hashlib
import os
import sys

MIN_LINES = 15
MAX_LINE = 400
MAX_BYTES = 400_000
SKIP_DIR = (
    "/dist/", "/build/", "/.next/", "/static/chunks/", "/out/", "/vendor/",
    "/coverage/", "/.cache/", "/bundle", "/bundles/", "/min/", "/compiled/",
    "/public/assets/", "/.git/", "/browser-profile",
)
SKIP_NAME = (".min.js", ".bundle.js", ".chunk.js", "-min.js")


def keep(path):
    low = path.lower()
    if any(part in low for part in SKIP_DIR) or low.endswith(SKIP_NAME):
        return False
    try:
        with open(path, "rb") as fh:
            raw = fh.read(MAX_BYTES + 1)
    except OSError:
        return False
    if len(raw) > MAX_BYTES:
        return False
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return False
    lines = text.splitlines()
    if len(lines) < MIN_LINES:
        return False
    longest = 0
    for line in lines:
        n = len(line)
        if n > longest:
            longest = n
            if longest > MAX_LINE:
                return False
    return True


def main():
    seen = set()
    for path in sys.stdin:
        path = path.rstrip("\n")
        if not path or not os.path.isfile(path):
            continue
        try:
            with open(path, "rb") as fh:
                digest = hashlib.sha256(fh.read()).hexdigest()
        except OSError:
            continue
        if digest in seen:
            continue
        seen.add(digest)
        if keep(path):
            sys.stdout.write(path + "\n")


if __name__ == "__main__":
    main()
