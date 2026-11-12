#!/usr/bin/env python3
"""Compare two sweep records and refuse a delta it cannot explain.

A sweep record is the JSON that `rust-graph build <dir> --stdout` prints, or a merged
record with the same shape: {"files": N, "sources": [{"path", "lang", "deep_fallback"}]}.

`deep_fallback` is either absent (the file reads clean), a reason string (the guard could
not reconcile the two readers), or a structured dict carrying "reason"/"reader"/"kind".

The gate this tool enforces:
  a file may move fallback -> clean for any reason (that is the point of the work),
  but a file that moves clean -> fallback must name a reason. If it does not, the delta is
  refused: exit 2, and the caller must not commit. That is the regression gate the
  eight-phase plan hangs on.

Usage:
  tools/diff.py before.json after.json [--by lang,reason] [--json] [--allow-clean-to-fail]
"""
from __future__ import annotations

import argparse
import json
import sys
from collections import Counter


def reason_of(row: dict) -> str:
    value = row.get("deep_fallback")
    if not value:
        return ""
    if isinstance(value, dict):
        return str(value.get("reason") or value.get("kind") or "named-without-text")
    return str(value)


def short_reason(text: str) -> str:
    """The class of a reason, not its instance: `the CST lost a event: event x line 176 …`."""
    if not text:
        return ""
    head = text.split("(")[0].strip()
    return head[:70]


def load(path: str) -> dict[str, dict]:
    with open(path) as fh:
        data = json.load(fh)
    sources = data.get("sources", data if isinstance(data, list) else [])
    return {row["path"]: row for row in sources}


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("before")
    ap.add_argument("after")
    ap.add_argument("--by", default="lang,reason", help="group the delta by these fields")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--allow-clean-to-fail", action="store_true", help="do not gate (report only)")
    args = ap.parse_args(argv)

    before, after = load(args.before), load(args.after)
    per_lang: dict[str, Counter] = {}
    for name, rows in (("before", before), ("after", after)):
        for row in rows.values():
            lang = row.get("lang") or "?"
            counter = per_lang.setdefault(lang, Counter())
            if name == "before":
                counter["before_total"] += 1
                counter["before_fail"] += bool(reason_of(row))
            else:
                counter["after_total"] += 1
                counter["after_fail"] += bool(reason_of(row))

    clean_to_fail, fail_to_clean, unexplained = [], [], []
    for path, row in after.items():
        was, now = before.get(path), reason_of(row)
        if was is None:
            if now:
                clean_to_fail.append((path, now, "new file"))
            continue
        before_reason = reason_of(was)
        if not before_reason and now:
            clean_to_fail.append((path, now, ""))
            if not short_reason(now):
                unexplained.append(path)
        elif before_reason and not now:
            fail_to_clean.append(path)

    reasons = Counter(short_reason(reason_of(row)) for row in after.values() if reason_of(row))
    verdict = 0
    if unexplained and not args.allow_clean_to_fail:
        verdict = 2

    if args.json:
        print(json.dumps({
            "per_lang": {k: dict(v) for k, v in sorted(per_lang.items())},
            "clean_to_fail": len(clean_to_fail),
            "fail_to_clean": len(fail_to_clean),
            "unexplained": unexplained[:20],
            "top_reasons": reasons.most_common(10),
            "verdict": "refused-unexplained-regression" if verdict else "ok",
        }, indent=1))
        return verdict

    print(f"{'lang':<12}{'before':>12}{'after':>12}{'delta':>9}{'moved to fail':>15}{'top reason':>0}")
    for lang in sorted(per_lang):
        c = per_lang[lang]
        b, a = c["before_fail"], c["after_fail"]
        moved = sum(1 for p, _, _ in clean_to_fail if after[p].get("lang") == lang)
        top = Counter(
            short_reason(reason_of(r)) for r in after.values()
            if reason_of(r) and (r.get("lang") or "?") == lang
        ).most_common(1)
        print(f"{lang:<12}{b:>6}/{c['before_total']:<5}{a:>6}/{c['after_total']:<5}"
              f"{a - b:>+9}{moved:>15}  {top[0][0] if top else '-'}")

    print(f"\nfail -> clean: {len(fail_to_clean)}   clean -> fail: {len(clean_to_fail)}")
    for path, reason, note in clean_to_fail[:10]:
        print(f"  REGRESSED {path}: {short_reason(reason) or 'NO REASON'}{(' ' + note) if note else ''}")
    if verdict:
        print(f"\nREFUSED: {len(unexplained)} file(s) moved clean -> fallback with no named reason.")
        return verdict
    print("gate: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
