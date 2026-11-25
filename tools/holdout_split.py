#!/usr/bin/env python3
"""Split the corpus into a tuning set and a frozen blind holdout, by REPOSITORY.

Why by repository and not by file: files inside one repo share idioms, macros and
style, so a random 20% of files leaks. You end up with 200 kubernetes files that
use the exact macro the tuning set special-cased, they pass, and the holdout stops
meaning anything.

The rules this implements (owner, 2026-11-25):
  1. Split whole repos. Never files.
  2. Sort each language's repos by size (LOC), take every fifth repo (20%).
     The metric is LOC-weighted, so this keeps the big monsters spread across
     both halves instead of dumping them all in one.
  3. A language with fewer than ``--min-repos`` distinct repos cannot be split:
     it is flagged unholdoutable, all of it stays in tuning, and it is measured
     cold (no holdout claim).
  4. One shot per peek. The holdout is measured ONCE, after the tuning set hits
     the gates, and never per round. See docs/HOLDOUT.md.

Usage:
  python3 tools/holdout_split.py --corpus /tmp/opencode/lean/corpus45 \
      --train /tmp/opencode/rg_train_corpus --holdout /tmp/opencode/rg_holdout_corpus \
      --manifest /tmp/opencode/rg_holdout_manifest.json --freeze

The holdout is a real copy (chmod 0444/0555 when frozen); the tuning set is built
with hard links so it costs no disk.
"""
import argparse
import hashlib
import json
import os
import shutil
import stat
import sys


def repo_files(root: str, lang: str, repo: str):
    base = os.path.join(root, lang, repo)
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:] = [d for d in sorted(dirnames) if not d.startswith(".")]
        for name in sorted(filenames):
            path = os.path.join(dirpath, name)
            if os.path.isfile(path) and not os.path.islink(path):
                yield path


def repo_loc(paths) -> tuple[int, int, int]:
    files = loc = size = 0
    for path in paths:
        try:
            data = open(path, "rb").read()
        except OSError:
            continue
        if b"\0" in data:
            continue
        files += 1
        size += len(data)
        loc += sum(1 for line in data.splitlines() if line.strip())
    return files, loc, size


def languages(root: str):
    return sorted(d for d in os.listdir(root)
                  if os.path.isdir(os.path.join(root, d)) and not d.startswith("."))


def repos_of(root: str, lang: str):
    return sorted(d for d in os.listdir(os.path.join(root, lang))
                  if os.path.isdir(os.path.join(root, lang, d)) and not d.startswith("."))


def copy_repo(src: str, dst: str, link: bool) -> None:
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    for dirpath, dirnames, filenames in os.walk(src):
        rel = os.path.relpath(dirpath, src)
        target_dir = os.path.join(dst, rel) if rel != "." else dst
        os.makedirs(target_dir, exist_ok=True)
        for name in filenames:
            source = os.path.join(dirpath, name)
            if os.path.islink(source) or not os.path.isfile(source):
                continue
            target = os.path.join(target_dir, name)
            if os.path.exists(target):
                continue
            if link:
                try:
                    os.link(source, target)
                    continue
                except OSError:
                    pass
            shutil.copy2(source, target)


def fingerprint_dir(root: str) -> tuple[int, int, str]:
    digest = hashlib.sha256()
    files = size = 0
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in sorted(dirnames) if not d.startswith(".")]
        for name in sorted(filenames):
            path = os.path.join(dirpath, name)
            data = open(path, "rb").read()
            digest.update(os.path.relpath(path, root).encode() + b"\0" +
                          hashlib.sha256(data).hexdigest().encode() + b"\n")
            files += 1
            size += len(data)
    return files, size, digest.hexdigest()[:16]


def freeze(root: str) -> None:
    for dirpath, dirnames, filenames in os.walk(root, topdown=False):
        for name in filenames:
            path = os.path.join(dirpath, name)
            if os.path.islink(path):
                continue
            os.chmod(path, stat.S_IRUSR | stat.S_IRGRP | stat.S_IROTH)
        for name in dirnames:
            path = os.path.join(dirpath, name)
            os.chmod(path, 0o555)
    os.chmod(root, 0o555)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default="/tmp/opencode/lean/corpus45")
    ap.add_argument("--train", default="/tmp/opencode/rg_train_corpus")
    ap.add_argument("--holdout", default="/tmp/opencode/rg_holdout_corpus")
    ap.add_argument("--manifest", default="/tmp/opencode/rg_holdout_manifest.json")
    ap.add_argument("--every", type=int, default=5, help="take every Nth repo (5 = 20%%)")
    ap.add_argument("--min-repos", type=int, default=5,
                    help="languages with fewer repos than this cannot be split")
    ap.add_argument("--freeze", action="store_true", help="make the holdout read-only")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    manifest = {"corpus": args.corpus, "every": args.every,
                "min_repos": args.min_repos, "languages": {}}

    for lang in languages(args.corpus):
        repos = repos_of(args.corpus, lang)
        rows = []
        for repo in repos:
            files, loc, size = repo_loc(repo_files(args.corpus, lang, repo))
            rows.append({"repo": repo, "files": files, "loc": loc, "bytes": size})
        rows.sort(key=lambda r: (-r["loc"], r["repo"]))

        splittable = len(rows) >= args.min_repos
        holdout = [r["repo"] for i, r in enumerate(rows) if splittable and (i + 1) % args.every == 0]
        held = set(holdout)

        if not args.dry_run:
            for row in rows:
                is_holdout = row["repo"] in held
                dst_root = args.holdout if is_holdout else args.train
                copy_repo(os.path.join(args.corpus, lang, row["repo"]),
                          os.path.join(dst_root, lang, row["repo"]), link=not is_holdout)

        manifest["languages"][lang] = {
            "repos": len(rows),
            "splittable": splittable,
            "holdout_repos": holdout,
            "train_repos": [r["repo"] for r in rows if r["repo"] not in held],
            "holdout_loc": sum(r["loc"] for r in rows if r["repo"] in held),
            "train_loc": sum(r["loc"] for r in rows if r["repo"] not in held),
            "detail": rows,
        }
        flag = "holdout" if holdout else "UNHOLDOUTABLE"
        print(f"{lang:14s} repos={len(rows):2d} {flag:12s} holdout={','.join(holdout) or '-'}")

    if not args.dry_run:
        os.makedirs(args.holdout, exist_ok=True)
        files, size, sha = fingerprint_dir(args.holdout)
        manifest["holdout"] = {"files": files, "bytes": size, "sha256": sha,
                               "frozen": bool(args.freeze)}
        if args.freeze:
            freeze(args.holdout)
        with open(args.manifest, "w") as fh:
            json.dump(manifest, fh, indent=1)
        print(f"\nholdout: {files} files, {size} bytes, sha256={sha}")
        print(f"manifest: {args.manifest}")
        split = sum(1 for v in manifest["languages"].values() if v["splittable"])
        print(f"splittable languages: {split}/{len(manifest['languages'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
