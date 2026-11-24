#!/usr/bin/env python3
"""Harvest large source files (>= N non-blank lines) into a per-language corpus.

The 45-language corpora are dominated by small files — median 9 LOC in awk, 101
in powershell — so their fallback rate says more about the file-size mix than
about the reader. This tool builds the other half of the evidence: for every
language, real files above a LOC floor, deduplicated by content, laid out at
  <out>/<lang>/<sha8>-<basename>
with a manifest of source path, LOC and sha256, so the same measurement can be
run file-weighted AND LOC-weighted on code that actually has weight.

Usage:
  python3 tools/large_files.py --roots /home/roni /tmp/opencode --min-loc 700
  python3 tools/large_files.py --list            # per-language counts after a run
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import os
import sys
from pathlib import Path

DEFAULT_OUT = Path("/tmp/opencode/lean/corpus45-large")
PRUNE = {
    ".git", ".hg", ".svn", "target", "node_modules", "build", "dist", ".next",
    ".cache", ".venv", "venv", "site-packages", "__pycache__", ".terraform",
    ".cargo-target", "rg-kbuilds", "corpus45-large", ".gradle", "Pods", "DerivedData",
}
MAX_BYTES = 2 * 1024 * 1024
MIN_BYTES = 12 * 1024


def load_spec():
    here = Path(__file__).resolve().parent
    mod_path = here / "corpus45" / "build_corpus.py"
    spec = importlib.util.spec_from_file_location("build_corpus", mod_path)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod.SPEC


def ext_index(spec):
    idx = {}
    for lang, (exts, _repos) in spec.items():
        for ext in exts:
            # .h is claimed by c first, .m by objective-c; first writer wins
            idx.setdefault(ext.lower(), lang)
    return idx


def loc_of(path: Path):
    try:
        raw = path.read_bytes()
    except OSError:
        return 0, ""
    if b"\0" in raw[:4096]:
        return 0, ""
    n = sum(1 for line in raw.splitlines() if line.strip())
    return n, raw


def harvest(roots, out: Path, min_loc: int, max_files: int):
    spec = load_spec()
    by_ext = ext_index(spec)
    out.mkdir(parents=True, exist_ok=True)
    seen = set()
    kept = {}
    manifest = (out / "manifest.tsv").open("w")
    manifest.write("language\tloc\tbytes\tsha256\tsource\n")

    for root in roots:
        for base, dirs, names in os.walk(root):
            dirs[:] = [d for d in dirs if d not in PRUNE and not d.startswith(".git")]
            for name in names:
                ext = os.path.splitext(name)[1].lower()
                lang = by_ext.get(ext)
                if not lang:
                    continue
                full = Path(base) / name
                try:
                    size = full.stat().st_size
                except OSError:
                    continue
                if size < MIN_BYTES or size > MAX_BYTES:
                    continue
                n, raw = loc_of(full)
                if n < min_loc:
                    continue
                digest = hashlib.sha256(raw).hexdigest()
                if digest in seen:
                    continue
                bucket = kept.setdefault(lang, 0)
                if max_files and bucket >= max_files:
                    continue
                lang_dir = out / lang
                lang_dir.mkdir(parents=True, exist_ok=True)
                dest = lang_dir / f"{digest[:8]}-{name}"
                try:
                    dest.write_bytes(raw)
                except OSError:
                    continue
                seen.add(digest)
                kept[lang] = bucket + 1
                manifest.write(f"{lang}\t{n}\t{size}\t{digest}\t{full}\n")
                manifest.flush()

    manifest.close()
    return kept


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--roots", nargs="*", default=["/home/roni", "/tmp/opencode"])
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    ap.add_argument("--min-loc", type=int, default=700)
    ap.add_argument("--max-files", type=int, default=0, help="0 = no cap per language")
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    out = Path(args.out)
    if args.list:
        rows = []
        for lang_dir in sorted(p for p in out.iterdir() if p.is_dir()):
            files = list(lang_dir.iterdir())
            total = 0
            for f in files:
                n, _ = loc_of(f)
                total += n
            rows.append((lang_dir.name, len(files), total))
        for lang, count, total in rows:
            print(f"{lang:14} {count:5} files {total:8} loc")
        print(f"{len(rows)} languages")
        return

    kept = harvest(args.roots, out, args.min_loc, args.max_files)
    total_files = sum(kept.values())
    print(f"kept {total_files} files across {len(kept)} languages in {out}")
    for lang, count in sorted(kept.items(), key=lambda kv: -kv[1]):
        print(f"  {lang:14} {count}")
    missing = sorted(set(load_spec()) - set(kept))
    if missing:
        print("none found for: " + " ".join(missing))
        print("(languages whose local sources are all under the LOC floor)", file=sys.stderr)


if __name__ == "__main__":
    main()
