#!/usr/bin/env python3
"""Measure one corpus with N parallel rust-graph processes over disjoint shards.

The record carries a per-reason taxonomy, not just the total: a phase that fixes
twelve missing events and introduces twelve missing symbols is a phase that did
nothing, and a single fallback count cannot tell those two apart.

    python3 tools/shard_sweep.py <corpus-dir> <out.json> [jobs]

Writes the record to <out.json> and the taxonomy to <out.json>.reasons.json so
one phase's reason histogram can be diffed against the next without re-reading
the whole tree.
"""
import json, os, shutil, subprocess, sys, tempfile
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

BIN = os.environ.get(
    "RUST_GRAPH_BIN",
    "/home/roni/Roni_workspace/rg-kbuilds/verify/x86_64-unknown-linux-musl/debug/rust-graph",
)
root, out = Path(sys.argv[1]), Path(sys.argv[2])
jobs = int(sys.argv[3]) if len(sys.argv) > 3 else 8
files = sorted(p for p in root.rglob("*") if p.is_file())
work = Path(tempfile.mkdtemp(prefix="shard-"))
shards = [files[i::jobs] for i in range(jobs)]


def run(i):
    d = work / f"s{i}"
    d.mkdir()
    for f in shards[i]:
        os.link(f, d / f.name)
    r = subprocess.run([BIN, "build", str(d), "--no-cache", "--stdout"], capture_output=True, text=True)
    shutil.rmtree(d)
    return r.stdout


with ThreadPoolExecutor(jobs) as pool:
    outs = list(pool.map(run, range(jobs)))

sources, total = [], 0
for o in outs:
    d = json.loads(o)
    total += d.get("files", len(d["sources"]))
    sources += d["sources"]

fallbacks = [s for s in sources if s.get("deep_fallback")]
reasons = Counter((s.get("deep_fallback") or "").split(":")[0] for s in fallbacks)
by_lang = Counter()
for s in sources:
    if s.get("deep_fallback"):
        by_lang[s.get("lang") or "?"] += 1

record = {
    "files": total,
    "fallback": len(fallbacks),
    "reasons": dict(reasons),
    "by_lang": dict(by_lang),
    "sources": sources,
}
out.write_text(json.dumps(record))
out.with_suffix(out.suffix + ".reasons.json").write_text(
    json.dumps({"corpus": root.name, "files": total, "fallback": len(fallbacks),
                "reasons": dict(reasons), "by_lang": dict(by_lang)}, indent=1)
)
print(f"{root.name}: {len(fallbacks)} of {total} fallbacks  "
      f"{sorted(reasons.items(), key=lambda x: -x[1])[:4]}")
shutil.rmtree(work, ignore_errors=True)
