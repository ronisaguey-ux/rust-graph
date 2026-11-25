#!/usr/bin/env python3
"""Build a document-frequency index of every identifier in the corpus.

Overfitting in this parser looks like a rule keyed on a *word that exists in our
corpus* rather than on a syntax class. To find those automatically we need to know
how rare a given word is: a keyword appears in tens of thousands of files, a
repo-specific macro appears in three.

Writes {token: file_count} for tokens of 3+ chars. One pass, ~8 GB of source.
"""
import json
import os
import re
import sys
import time

CORPUS = sys.argv[1] if len(sys.argv) > 1 else "/tmp/opencode/lean/corpus45"
OUT = sys.argv[2] if len(sys.argv) > 2 else "/tmp/opencode/corpus_token_df.json"
TOKEN = re.compile(rb"[A-Za-z_][A-Za-z0-9_]{2,}")

counts: dict[str, int] = {}
files = 0
started = time.time()

for root, dirs, names in os.walk(CORPUS):
    dirs[:] = [d for d in sorted(dirs) if not d.startswith(".")]
    for name in names:
        path = os.path.join(root, name)
        try:
            data = open(path, "rb").read()
        except OSError:
            continue
        if b"\0" in data:
            continue
        files += 1
        for token in set(TOKEN.findall(data)):
            key = token.decode("ascii", "ignore")
            counts[key] = counts.get(key, 0) + 1
        if files % 5000 == 0:
            print(f"{files} files, {len(counts)} tokens, {time.time()-started:.0f}s", flush=True)

with open(OUT, "w") as fh:
    json.dump({"corpus": CORPUS, "files": files, "tokens": counts}, fh)
print(f"done: {files} files, {len(counts)} tokens -> {OUT}")
