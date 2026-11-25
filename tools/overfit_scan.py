#!/usr/bin/env python3
"""Scan rust-graph's own rules for corpus-specific literals - the fingerprints of overfitting.

A rule that keys on a *syntax class* generalises; a rule that keys on a *word that
happens to appear in our corpus* does not. The two look identical in Rust code:

    if tok == "is"                     // a keyword: appears in ~every file
    if tok == "ZEND_NONSTRING"         // a corpus macro: appears in 12 files

This tool extracts every string literal from the parser sources, looks each one up
in a document-frequency index of the corpus, and ranks the literals by how rare
they are. A literal used as a comparison whose rarest token appears in a handful of
files is either (a) a genuine language keyword we missed, or (b) overfitting.

Usage:
  python3 tools/corpus_token_df.py            # build the index once
  python3 tools/overfit_scan.py               # ranked report
  python3 tools/overfit_scan.py --json > /tmp/overfit.json
  python3 tools/overfit_scan.py --rare 25 --max-rows 60
"""
import argparse
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DF_PATH = "/tmp/opencode/corpus_token_df.json"
SOURCES = ["src/deep_facts", "src/deep", "crates/deep-grammars/src", "src"]

LITERAL = re.compile(r'"((?:[^"\\]|\\.){2,120})"')
TOKEN = re.compile(r"[A-Za-z_][A-Za-z0-9_]{2,}")
# a literal is only interesting where it is used as a decision, not as a message
GUARD = re.compile(
    r"(==|!=|\.contains\(|\.starts_with\(|\.ends_with\(|\.eq\(|matches!|\.find\(|"
    r"strip_prefix\(|strip_suffix\(|trim_matches\()")


def corpus_repo_names(corpus: str) -> set[str]:
    names = set()
    for lang in os.listdir(corpus):
        path = os.path.join(corpus, lang)
        if not os.path.isdir(path) or lang.startswith("."):
            continue
        for repo in os.listdir(path):
            if os.path.isdir(os.path.join(path, repo)) and not repo.startswith("."):
                names.add(repo.lower())
    return names


def rust_files(root: str):
    for rel in SOURCES:
        base = os.path.join(root, rel)
        if not os.path.isdir(base):
            continue
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = [d for d in sorted(dirnames) if d not in ("target", ".git")]
            for name in sorted(filenames):
                if name.endswith(".rs"):
                    yield os.path.join(dirpath, name)


def scan(df: dict, rare_max: int, repos: set[str]):
    rows = []
    seen = set()
    for path in rust_files(ROOT):
        try:
            lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
        except OSError:
            continue
        for number, line in enumerate(lines, 1):
            if line.lstrip().startswith("//"):
                continue
            guard = bool(GUARD.search(line))
            for match in LITERAL.finditer(line):
                literal = match.group(1)
                if len(literal) < 3 or " " in literal and len(literal) > 40:
                    continue
                tokens = TOKEN.findall(literal)
                if not tokens:
                    continue
                counts = [(tok, df.get(tok, 0)) for tok in tokens]
                rarest = min(counts, key=lambda pair: pair[1])
                if rarest[1] > rare_max:
                    continue
                key = (path, number, literal)
                if key in seen:
                    continue
                seen.add(key)
                lowered = literal.lower()
                rows.append({
                    "file": os.path.relpath(path, ROOT),
                    "line": number,
                    "literal": literal,
                    "rarest_token": rarest[0],
                    "token_df": rarest[1],
                    "guard": guard,
                    "repo_named": any(name and name in lowered for name in repos),
                })
    rows.sort(key=lambda r: (not r["guard"], r["repo_named"], r["token_df"]))
    return rows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--df", default=DF_PATH)
    ap.add_argument("--corpus", default="/tmp/opencode/lean/corpus45")
    ap.add_argument("--rare", type=int, default=25, help="flag literals whose rarest token is in <= N files")
    ap.add_argument("--max-rows", type=int, default=80)
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    if not os.path.exists(args.df):
        print(f"missing index {args.df}; run tools/corpus_token_df.py first", file=sys.stderr)
        return 2
    data = json.load(open(args.df))
    df = data["tokens"]
    repos = corpus_repo_names(args.corpus)
    rows = scan(df, args.rare, repos)

    if args.json:
        print(json.dumps({"corpus_files": data["files"], "rare_max": args.rare,
                          "findings": rows}, indent=1))
        return 0

    guarded = [r for r in rows if r["guard"]]
    print(f"# corpus {data['files']} files, {len(df)} tokens; "
          f"{len(rows)} rare literals, {len(guarded)} of them used in a decision")
    print(f"# {'file:line':52s} {'df':>5s} {'token':22s} literal")
    for row in rows[:args.max_rows]:
        mark = "*" if row["repo_named"] else " "
        flag = "G" if row["guard"] else " "
        print(f"{mark}{flag} {row['file'] + ':' + str(row['line']):52s} "
              f"{row['token_df']:5d} {row['rarest_token'][:22]:22s} {row['literal'][:60]}")
    print("\n* = literal names a corpus repository   G = used in a comparison/contains guard")
    return 0


if __name__ == "__main__":
    sys.exit(main())
