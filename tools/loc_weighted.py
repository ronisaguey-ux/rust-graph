#!/usr/bin/env python3
"""File-weighted and LOC-weighted fallback percentages per corpus language.

File weighting is what every earlier round reported: a 40-line fixture counts
the same as a 3,000-line module, so a corpus full of small files makes the rate
look better than the code it represents. LOC weighting answers the other
question: what share of the *code* falls back.

Definitions, chosen once and used everywhere:
  source file   any file under corpus45/<lang>/ that the binary accepted (no NUL byte)
  LOC           non-blank lines, comments included
  fallback file a file with at least one `loss\t<file>` row from the binary

Usage:
  RG_BIN=/path/to/rust-graph python3 tools/loc_weighted.py tcl awk powershell
  RG_BIN=... python3 tools/loc_weighted.py --root /tmp/opencode/lean/corpus45 --all
"""
import argparse
import os
import subprocess
import sys

ROOT = os.environ.get("CORPUS_ROOT", "/tmp/opencode/lean/corpus45")
BIN = os.environ.get(
    "RG_BIN",
    "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph",
)


def loc(path):
    try:
        with open(path, "rb") as fh:
            raw = fh.read()
    except OSError:
        return 0
    if b"\0" in raw:
        return 0
    return sum(1 for line in raw.splitlines() if line.strip())


def measure(lang, timeout):
    path = os.path.join(ROOT, lang)
    if not os.path.isdir(path):
        return None
    env = dict(os.environ, RG_LOSSES="1", RG_AUDIT="1")
    try:
        run = subprocess.run(
            [BIN, "build", path, "--no-cache"],
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
        rc = run.returncode
        out = run.stdout + run.stderr
    except subprocess.TimeoutExpired:
        return {"lang": lang, "timeout": True}

    fallback = set()
    for line in out.splitlines():
        if line.startswith("loss\t"):
            parts = line.split("\t")
            if len(parts) > 2:
                # the binary prints loss paths relative to the corpus root it was given
                fallback.add(os.path.realpath(os.path.join(path, parts[1])))

    total_loc = 0
    fallback_loc = 0
    files = 0
    sizes = []
    for base, _dirs, names in os.walk(path):
        for name in names:
            full = os.path.join(base, name)
            n = loc(full)
            if n == 0:
                continue
            files += 1
            sizes.append(n)
            total_loc += n
            if os.path.realpath(full) in fallback:
                fallback_loc += n

    sizes.sort()
    median = sizes[len(sizes) // 2] if sizes else 0
    top10 = sizes[int(len(sizes) * 0.9)] if sizes else 0
    return {
        "lang": lang,
        "files": files,
        "fallback_files": len(fallback),
        "loc": total_loc,
        "fallback_loc": fallback_loc,
        "median_loc": median,
        "p90_loc": top10,
        "rc": rc,
        "timeout": False,
    }


def pct(part, whole):
    return 100.0 * part / whole if whole else 0.0


def main():
    global ROOT
    ap = argparse.ArgumentParser()
    ap.add_argument("languages", nargs="*")
    ap.add_argument("--root", default=ROOT)
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--timeout", type=int, default=1800)
    args = ap.parse_args()

    ROOT = args.root
    langs = args.languages
    if args.all:
        langs = sorted(
            d for d in os.listdir(ROOT) if os.path.isdir(os.path.join(ROOT, d))
        )
    if not langs:
        ap.error("give one or more languages, or --all")

    print(
        f"{'language':14} {'files':>6} {'fall':>6} {'file%':>7} "
        f"{'loc':>9} {'fall_loc':>9} {'loc%':>7} {'med':>6} {'p90':>6}"
    )
    for lang in langs:
        r = measure(lang, args.timeout)
        if r is None:
            print(f"{lang:14} missing")
            continue
        if r["timeout"]:
            print(f"{lang:14} timeout")
            continue
        print(
            f"{r['lang']:14} {r['files']:6} {r['fallback_files']:6} "
            f"{pct(r['fallback_files'], r['files']):6.2f}% {r['loc']:9} "
            f"{r['fallback_loc']:9} {pct(r['fallback_loc'], r['loc']):6.2f}% "
            f"{r['median_loc']:6} {r['p90_loc']:6}"
        )
        if r["rc"] != 0:
            print(f"{'':14} exit={r['rc']}", file=sys.stderr)


if __name__ == "__main__":
    main()
