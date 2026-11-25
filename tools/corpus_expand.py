#!/usr/bin/env python3
"""Grow the corpus until every language has enough distinct REPOSITORIES to split.

A repo-level holdout needs 5+ repos per language: sort repos by size, hold out every
fifth. Twenty of our languages have exactly two repos and three have one, so those
languages cannot be split at all - and faking a split with two repos is worse than
none (one repo of two is a coin flip, and training on the single remaining repo
overfits harder).

This tool fills that hole: search GitHub per language, clone shallow, copy source
files under a per-repo budget, dedupe by content inside the language, and stop as
soon as the language has --target repos. It never touches an existing repo
directory, and it never deletes.

Usage:
  python3 tools/corpus_expand.py --corpus /tmp/opencode/lean/corpus45 --target 5
  python3 tools/corpus_expand.py --langs c go java --target 6
  python3 tools/corpus_expand.py --dry-run          # what it would look for

Run it BEFORE freezing a holdout: expanding after the freeze replaces the corpus
under a manifest that already claims a hash.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time

CLONES = "/tmp/opencode/corpus-expand-clones"
MAX_FILE = 512 * 1024
PER_REPO = 120

EXCLUDE_PATH = re.compile(
    r"/(node_modules|vendor|vendored|third_party|thirdparty|external|deps|dist|\.git|__pycache__|"
    r"bower_components|site-packages|\.venv|venv|coverage|\.next|out|obj|bin|Pods|Carthage)/", re.I)
EXCLUDE_NAME = re.compile(
    r"(\.min\.|\.bundle\.|-min\.|_pb2\.|\.pb\.(cc|h|cpp)$|\.g\.dart$|\.designer\.vb$|"
    r"\.generated\.|\.g\.cs$|\.freezed\.dart$|\.moc$|\.qm$)", re.I)

# language dir -> (GitHub search language, source extensions)
LANGS = {
    "assembly": ("Assembly", (".s", ".asm")),
    "awk": ("Awk", (".awk",)),
    "bash": ("Shell", (".sh", ".bash")),
    "c": ("C", (".c", ".h")),
    "clojure": ("Clojure", (".clj", ".cljs", ".cljc")),
    "cobol": ("COBOL", (".cob", ".cbl", ".cpy")),
    "cpp": ("C++", (".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx")),
    "crystal": ("Crystal", (".cr",)),
    "csharp": ("C#", (".cs",)),
    "d": ("D", (".d",)),
    "dart": ("Dart", (".dart",)),
    "elixir": ("Elixir", (".ex", ".exs")),
    "elm": ("Elm", (".elm",)),
    "erlang": ("Erlang", (".erl", ".hrl")),
    "fortran": ("Fortran", (".f", ".f90", ".f95", ".f03", ".for")),
    "fsharp": ("F#", (".fs", ".fsi", ".fsx")),
    "go": ("Go", (".go",)),
    "groovy": ("Groovy", (".groovy",)),
    "haskell": ("Haskell", (".hs", ".lhs")),
    "java": ("Java", (".java",)),
    "javascript": ("JavaScript", (".js", ".mjs", ".cjs")),
    "julia": ("Julia", (".jl",)),
    "kotlin": ("Kotlin", (".kt", ".kts")),
    "lua": ("Lua", (".lua",)),
    "matlab": ("MATLAB", (".m",)),
    "nim": ("Nim", (".nim",)),
    "objective-c": ("Objective-C", (".m", ".mm", ".h")),
    "ocaml": ("OCaml", (".ml", ".mli")),
    "pascal": ("Pascal", (".pas", ".pp")),
    "perl": ("Perl", (".pl", ".pm")),
    "php": ("PHP", (".php",)),
    "powershell": ("PowerShell", (".ps1", ".psm1")),
    "python": ("Python", (".py",)),
    "r": ("R", (".r", ".R")),
    "ruby": ("Ruby", (".rb",)),
    "rust": ("Rust", (".rs",)),
    "scala": ("Scala", (".scala", ".sc")),
    "solidity": ("Solidity", (".sol",)),
    "sql": ("SQL", (".sql",)),
    "swift": ("Swift", (".swift",)),
    "tcl": ("Tcl", (".tcl",)),
    "typescript": ("TypeScript", (".ts", ".tsx")),
    "vbnet": ("Visual Basic .NET", (".vb",)),
    "visual-basic": ("Visual Basic 6.0", (".bas", ".cls", ".frm")),
    "zig": ("Zig", (".zig",)),
}


def sha256(path: str) -> str | None:
    digest = hashlib.sha256()
    try:
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(65536), b""):
                digest.update(chunk)
    except OSError:
        return None
    return digest.hexdigest()


def repos_of(root: str, lang: str) -> list[str]:
    base = os.path.join(root, lang)
    if not os.path.isdir(base):
        return []
    return sorted(d for d in os.listdir(base)
                  if os.path.isdir(os.path.join(base, d)) and not d.startswith("."))


def existing_hashes(root: str, lang: str, exts) -> set[str]:
    seen = set()
    for repo in repos_of(root, lang):
        for cur, dirs, files in os.walk(os.path.join(root, lang, repo)):
            dirs[:] = [d for d in dirs if not d.startswith(".")]
            for name in files:
                if not name.lower().endswith(exts):
                    continue
                digest = sha256(os.path.join(cur, name))
                if digest:
                    seen.add(digest)
    return seen


def copy_repo(clone: str, dest: str, exts, budget: int, seen: set[str]) -> int:
    copied = 0
    for cur, dirs, files in os.walk(clone):
        dirs[:] = [d for d in dirs if not d.startswith(".")]
        rel_cur = "/" + os.path.relpath(cur, clone).replace(os.sep, "/") + "/"
        if EXCLUDE_PATH.search(rel_cur):
            dirs[:] = []
            continue
        for name in sorted(files):
            if copied >= budget:
                return copied
            if not name.lower().endswith(exts) or EXCLUDE_NAME.search(name):
                continue
            src = os.path.join(cur, name)
            try:
                if os.path.getsize(src) > MAX_FILE:
                    continue
            except OSError:
                continue
            digest = sha256(src)
            if not digest or digest in seen:
                continue
            out = os.path.join(dest, os.path.relpath(src, clone))
            os.makedirs(os.path.dirname(out), exist_ok=True)
            try:
                shutil.copyfile(src, out)
            except OSError:
                continue
            seen.add(digest)
            copied += 1
    return copied


def search(gh_lang: str, page: int, stars: int) -> list[dict]:
    query = f"language:{gh_lang} stars:>={stars} archived:false"
    cmd = ["gh", "api", "-X", "GET", "search/repositories", "-f", f"q={query}",
           "-f", "sort=stars", "-f", "order=desc", "-f", f"per_page=30",
           "-f", f"page={page}",
           "--jq", ".items[] | [.full_name, (.stargazers_count|tostring), (.size|tostring)] | @tsv"]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    if result.returncode != 0:
        print(f"    search failed: {(result.stderr or '').strip()[:90]}", flush=True)
        return []
    rows = []
    for line in result.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) >= 3:
            rows.append({"full_name": parts[0], "stars": int(parts[1] or 0),
                         "size_kb": int(parts[2] or 0)})
    return rows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default="/tmp/opencode/lean/corpus45")
    ap.add_argument("--target", type=int, default=5)
    ap.add_argument("--stars", type=int, default=250)
    ap.add_argument("--per-repo", type=int, default=PER_REPO)
    ap.add_argument("--langs", nargs="*")
    ap.add_argument("--max-repo-mb", type=int, default=120)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--delete-clone", action="store_true", default=True)
    args = ap.parse_args()

    os.makedirs(CLONES, exist_ok=True)
    langs = args.langs or sorted(LANGS)
    summary = {}

    for lang in langs:
        if lang not in LANGS:
            print(f"{lang:14} unknown language, skipped", flush=True)
            continue
        gh_lang, exts = LANGS[lang]
        have = repos_of(args.corpus, lang)
        need = args.target - len(have)
        print(f"{lang:14} repos={len(have):2d} need={max(need, 0)}", flush=True)
        if need <= 0:
            summary[lang] = {"added": [], "repos": len(have)}
            continue
        if args.dry_run:
            summary[lang] = {"would_search": gh_lang, "repos": len(have)}
            continue

        seen = existing_hashes(args.corpus, lang, exts)
        added = []
        page = 1
        while need > 0 and page <= 4:
            for row in search(gh_lang, page, args.stars):
                name = row["full_name"].split("/")[-1]
                if name in have or any(name == a for a in added):
                    continue
                if row["size_kb"] > args.max_repo_mb * 1024:
                    continue
                clone = os.path.join(CLONES, f"{lang}-{name}")
                shutil.rmtree(clone, ignore_errors=True)
                started = time.time()
                try:
                    subprocess.run(["git", "clone", "--depth", "1", "--single-branch", "-q",
                                    f"https://github.com/{row['full_name']}.git", clone],
                                   check=True, timeout=420, capture_output=True)
                except Exception as exc:
                    print(f"  {name:28} clone failed ({type(exc).__name__})", flush=True)
                    shutil.rmtree(clone, ignore_errors=True)
                    continue
                copied = copy_repo(clone, os.path.join(args.corpus, lang, name),
                                   exts, args.per_repo, seen)
                shutil.rmtree(clone, ignore_errors=True)
                if copied < 5:
                    shutil.rmtree(os.path.join(args.corpus, lang, name), ignore_errors=True)
                    print(f"  {name:28} +{copied:3} too few, dropped", flush=True)
                    continue
                have.append(name)
                added.append(name)
                need -= 1
                print(f"  {name:28} +{copied:3} files -> repos={len(have)} "
                      f"({int(time.time()-started)}s)", flush=True)
                if need <= 0:
                    break
            page += 1
            time.sleep(2)
        summary[lang] = {"added": added, "repos": len(have)}
        print(f"{lang:14} final repos={len(have)} {'OK' if len(have) >= args.target else 'SHORT'}",
              flush=True)

    path = os.path.join(CLONES, "expand-summary.json")
    with open(path, "w") as fh:
        json.dump({"corpus": args.corpus, "target": args.target, "languages": summary}, fh, indent=1)
    print(f"summary: {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
