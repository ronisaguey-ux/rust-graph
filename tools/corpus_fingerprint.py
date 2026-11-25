#!/usr/bin/env python3
"""Fingerprint the corpus a measurement was taken on.

Round 16 exposed the failure this script exists to prevent: lane 2 reported
"1/118 (0.85%)" for objective-c while the same binary on the host corpus
measured 217/263. Neither number was wrong - they were taken on different
directories that both called themselves corpus45. A percentage without the
fingerprint of the files under it cannot be compared to anything.

Print one line per language: file count, total bytes, total lines, and a sha1
over every (relative path, sha1 of contents) pair, so two machines can be
compared by eye and any drift is visible before a measurement is believed.

Usage:
  python3 tools/corpus_fingerprint.py /tmp/opencode/lean/corpus45 c haskell
  python3 tools/corpus_fingerprint.py --root /tmp/opencode/lean/corpus45-large --all
  python3 tools/corpus_fingerprint.py --root ... --json   # machine-readable
"""
import argparse
import hashlib
import json
import os
import sys

SKIP_SUFFIX = (".pyc",)


def files_of(root: str, lang: str):
    base = os.path.join(root, lang)
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:] = [d for d in sorted(dirnames) if d not in (".git", "__pycache__")]
        for name in sorted(filenames):
            if name.endswith(SKIP_SUFFIX):
                continue
            path = os.path.join(dirpath, name)
            if os.path.islink(path) or not os.path.isfile(path):
                continue
            yield path


def fingerprint(root: str, lang: str) -> dict:
    base = os.path.join(root, lang)
    if not os.path.isdir(base):
        return {"lang": lang, "missing": True}
    digest = hashlib.sha1()
    count = size = lines = 0
    for path in files_of(root, lang):
        rel = os.path.relpath(path, base).encode()
        content = open(path, "rb").read()
        count += 1
        size += len(content)
        lines += content.count(b"\n")
        digest.update(rel + b"\0" + hashlib.sha1(content).hexdigest().encode() + b"\n")
    return {"lang": lang, "files": count, "bytes": size, "lines": lines,
            "sha1": digest.hexdigest()[:16]}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default="/tmp/opencode/lean/corpus45")
    parser.add_argument("langs", nargs="*")
    parser.add_argument("--root", dest="root_opt")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    root = args.root_opt or args.root
    langs = args.langs
    if args.all or not langs:
        langs = sorted(d for d in os.listdir(root)
                       if os.path.isdir(os.path.join(root, d)))

    rows = [fingerprint(root, lang) for lang in langs]
    if args.json:
        print(json.dumps({"root": root, "languages": rows}, indent=1))
        return 0

    print(f"# corpus {root}")
    for row in rows:
        if row.get("missing"):
            print(f"{row['lang']}\tMISSING")
            continue
        print(f"{row['lang']}\tfiles={row['files']}\tlines={row['lines']}\t"
              f"bytes={row['bytes']}\tsha1={row['sha1']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
