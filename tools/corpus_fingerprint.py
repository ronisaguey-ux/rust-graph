#!/usr/bin/env python3
"""Fingerprint the corpus a measurement was taken on.

Round 16 exposed the failure this script exists to prevent: lane 2 reported
"1/118 (0.85%)" for objective-c while the same binary on the host corpus
measured 217/263. Neither number was wrong - they were taken on different
directories that both called themselves corpus45. A percentage without the
fingerprint of the files under it cannot be compared to anything.

Print one line per language: file count, total bytes, total lines, and a sha256
over every (relative path, sha256 of contents) pair, so two machines can be
compared by eye and any drift is visible before a measurement is believed.

A GUARD, not just a report: build a baseline once (--write-baseline), then any
lane runs --check before its first measurement. If a language's file count or LOC
drifts past --tolerance the tool refuses, prints both sides, and exits non-zero.
A lane that cannot prove it is measuring the host's corpus does not start.

Usage:
  python3 tools/corpus_fingerprint.py /tmp/opencode/lean/corpus45 c haskell
  python3 tools/corpus_fingerprint.py --root /tmp/opencode/lean/corpus45-large --all
  python3 tools/corpus_fingerprint.py --root ... --json   # machine-readable
  python3 tools/corpus_fingerprint.py --all --write-baseline corpus-baseline.json
  python3 tools/corpus_fingerprint.py --all --baseline corpus-baseline.json --check
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
    digest = hashlib.sha256()
    count = size = lines = 0
    for path in files_of(root, lang):
        rel = os.path.relpath(path, base).encode()
        content = open(path, "rb").read()
        count += 1
        size += len(content)
        lines += content.count(b"\n")
        digest.update(rel + b"\0" + hashlib.sha256(content).hexdigest().encode() + b"\n")
    return {"lang": lang, "files": count, "bytes": size, "lines": lines,
            "sha256": digest.hexdigest()[:16]}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default="/tmp/opencode/lean/corpus45")
    parser.add_argument("langs", nargs="*")
    parser.add_argument("--root", dest="root_opt")
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--write-baseline", metavar="PATH")
    parser.add_argument("--baseline", metavar="PATH")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--tolerance", type=float, default=0.02,
                        help="max allowed relative drift in files or LOC (default 2%%)")
    args = parser.parse_args()

    root = args.root_opt or args.root
    langs = args.langs
    if args.all or not langs:
        langs = sorted(d for d in os.listdir(root)
                       if os.path.isdir(os.path.join(root, d)))

    rows = [fingerprint(root, lang) for lang in langs]

    if args.write_baseline:
        import time as _time
        payload = {"root": root, "written": _time.strftime("%Y-%m-%dT%H:%M:%S"),
                   "languages": {r["lang"]: r for r in rows}}
        with open(args.write_baseline, "w") as fh:
            json.dump(payload, fh, indent=1, sort_keys=True)
        print(f"baseline written: {args.write_baseline} ({len(rows)} languages)")
        return 0

    if args.check:
        if not args.baseline:
            print("--check needs --baseline", file=sys.stderr)
            return 2
        base = json.load(open(args.baseline))["languages"]
        failures = []
        print(f"{'language':14s} {'base_files':>10s} {'files':>10s} {'base_loc':>10s} "
              f"{'loc':>10s}  drift")
        for row in rows:
            lang = row["lang"]
            if row.get("missing"):
                failures.append((lang, "missing on this box"))
                print(f"{lang:14s} {'-':>10s} {'-':>10s} {'-':>10s} {'-':>10s}  MISSING")
                continue
            ref = base.get(lang)
            if not ref:
                print(f"{lang:14s} {'n/a':>10s} {row['files']:10d} {'n/a':>10s} "
                      f"{row['lines']:10d}  (no baseline entry)")
                continue
            file_drift = abs(row["files"] / ref["files"] - 1) if ref["files"] else 1.0
            loc_drift = abs(row["lines"] / ref["lines"] - 1) if ref["lines"] else 1.0
            worst = max(file_drift, loc_drift)
            bad = worst > args.tolerance
            if bad:
                failures.append((lang, f"files {row['files']} vs {ref['files']}, "
                                       f"lines {row['lines']} vs {ref['lines']}"))
            print(f"{lang:14s} {ref['files']:10d} {row['files']:10d} {ref['lines']:10d} "
                  f"{row['lines']:10d}  {worst*100:5.1f}%{'  FAIL' if bad else ''}")
        if failures:
            print("\n" + "=" * 72)
            print("CORPUS GUARD REFUSED: this box is not measuring the baseline corpus.")
            print("Do not measure, do not report a percentage, do not start a round.")
            for lang, why in failures:
                print(f"  {lang:14s} {why}")
            print("Fix the corpus (re-sync it from the host) or run on the host corpus,")
            print("then re-run this check. Numbers from a mismatched corpus are void.")
            print("=" * 72)
            return 1
        print(f"\ncorpus guard passed: every language within {args.tolerance*100:.1f}% "
              f"of the baseline")
        return 0
    if args.json:
        print(json.dumps({"root": root, "languages": rows}, indent=1))
        return 0

    print(f"# corpus {root}")
    for row in rows:
        if row.get("missing"):
            print(f"{row['lang']}\tMISSING")
            continue
        print(f"{row['lang']}\tfiles={row['files']}\tlines={row['lines']}\t"
              f"bytes={row['bytes']}\tsha256={row['sha256']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
