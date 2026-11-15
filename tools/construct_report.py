#!/usr/bin/env python3
"""construct_report.py — attribute every Rust fallback to a construct.

P5-08 exits when
  * every Rust file that falls back carries a sentence and a machine code, and
    the code is not `unknown`,
  * and a construct two languages share is fixed in the code they share, not
    once per language.

The codes are assigned in `src/pipeline.rs`, which is language neutral: a Rust
loss and a TypeScript loss with the same code are the same disagreement between
the two readers, so they belong in `src/parser.rs` or `src/expr.rs`, not in a
per-language guard.

`tools/construct_fixes.json` records, per code, the module where that construct
is decided. The report refuses a shared code whose entry names a
language-specific file, and refuses a code that appears in the measurement with
no entry at all — that is the moment a construct starts being fixed twice.

    python3 tools/construct_report.py --measure   # sweep the four corpora, save the record
    python3 tools/construct_report.py             # write docs/RUST_CONSTRUCTS.md
    python3 tools/construct_report.py --check     # fail if the document is stale or a code is unowned
"""
from __future__ import annotations

import argparse
import json
import re
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
DOC = ROOT / "docs" / "RUST_CONSTRUCTS.md"
RECORD = ROOT / "tools" / "baselines" / "constructs.json"
FIXES = ROOT / "tools" / "construct_fixes.json"
CORPORA = Path(os.environ.get("RG_SWEEP_ROOT", "/tmp/opencode/sweep-local"))
LANGUAGES = ("js", "ts", "python", "rust")
DEFAULT_BINARY = Path(
    os.environ.get(
        "RG_BIN",
        "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph",
    )
)
# The modules that both readers of every language go through. A shared construct
# fixed anywhere else — `src/deep_facts/rust.rs` and friends — is a second fix
# waiting to disagree with the first.
SHARED_MODULES = (
    "src/parser.rs",
    "src/expr.rs",
    "src/pipeline.rs",
    "src/deep_integration.rs",
    "src/deep_facts/mod.rs",
)


def measure(binary: Path) -> dict:
    corpora: dict[str, dict] = {}
    codes: dict[str, dict] = {}
    for language in LANGUAGES:
        corpus = CORPORA / language
        if not corpus.is_dir():
            raise SystemExit(f"construct_report: no corpus at {corpus}")
        result = subprocess.run(
            [str(binary), "build", str(corpus), "--no-cache", "--stdout"],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise SystemExit(f"construct_report: {corpus} exited {result.returncode}")
        rows = json.loads(result.stdout)["sources"]
        fallen = [row for row in rows if "deep_fallback" in row]
        corpora[language] = {"total": len(rows), "fallen": len(fallen)}
        for row in fallen:
            code = row.get("deep_fallback_code") or ""
            entry = codes.setdefault(
                code,
                {"rus": 0, "js": 0, "ts": 0, "python": 0, "samples": []},
            )
            entry[language] = entry.get(language, 0) + 1
            if len(entry["samples"]) < 2:
                line = re.search(r"line (\d+)", row.get("deep_fallback", ""))
                entry["samples"].append(
                    {
                        "path": Path(row["path"]).name,
                        "line": line.group(1) if line else "",
                        "reason": row.get("deep_fallback", "")[:160],
                    }
                )
    return {"corpora": corpora, "codes": codes}


def load(path: Path) -> dict:
    return json.loads(path.read_text())


def render(record: dict, fixes: dict) -> tuple[str, list[str]]:
    problems: list[str] = []
    corpora = record.get("corpora", {})
    codes = {code: entry for code, entry in record.get("codes", {}).items() if entry.get("rust", 0) > 0}
    rows: list[tuple[str, int, str, bool, str, str]] = []
    for code in sorted(codes):
        rust = codes[code].get("rust", 0)
        others = {language: codes[code].get(language, 0) for language in LANGUAGES if language != "rust"}
        spread_counts = {language: count for language, count in others.items() if count}
        shared = bool(spread_counts)
        entry = fixes.get(code)
        if entry is None:
            problems.append(f"{code}: {rust} rust file(s), no entry in tools/construct_fixes.json")
        module = entry["module"] if entry else "—"
        why = entry["why"] if entry else "unowned"
        if shared and entry and module not in SHARED_MODULES:
            problems.append(
                f"{code}: shared with {', '.join(sorted(spread_counts))} but recorded as fixed in {module}, "
                "which only one language reads"
            )
        if not shared and entry and entry.get("shared"):
            problems.append(f"{code}: recorded as shared but no other language reports it")
        if not code:
            problems.append("unknown: a rust file fell back with no code")
        elif code.split(":")[0] == "unknown":
            problems.append(f"{code}: a rust file fell back with an unattributed code")
        spread = ", ".join(f"{language} {count}" for language, count in sorted(spread_counts.items())) or "—"
        rows.append((code, rust, spread, shared, module, why))

    lines = [
        "# Rust constructs still failing, and where they are fixed",
        "",
        "Generated by `tools/construct_report.py` from `tools/baselines/constructs.json`.",
        "Do not edit by hand: `python3 tools/construct_report.py --check` fails when this",
        "file and the record disagree.",
        "",
        "Measured corpora: " + ", ".join(
            f"{language} {corpora.get(language, {}).get('fallen', 0)}/{corpora.get(language, {}).get('total', 0)}"
            for language in LANGUAGES
        ),
        "",
        "A code two languages share is one disagreement between the two readers, so it is",
        "fixed in the module both readers go through — never in a per-language guard.",
        "",
        "| construct code | rust | also lost in | fixed in | why |",
        "| --- | --- | --- | --- | --- |",
    ]
    for code, count, spread, _shared, module, why in rows:
        lines.append(f"| `{code}` | {count} | {spread} | `{module}` | {why} |")
    lines.append("")
    lines.append("## Samples")
    lines.append("")
    for code, count, _spread, _shared, _module, _why in rows:
        samples = codes[code].get("samples", [])
        if not samples:
            continue
        lines.append(f"### `{code}` — {count} file(s)")
        lines.append("")
        for sample in samples:
            where = f"{sample['path']}:{sample['line']}" if sample["line"] else sample["path"]
            lines.append(f"- `{where}` — {sample['reason']}")
        lines.append("")
    return "\n".join(lines).rstrip("\n") + "\n", problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--measure", action="store_true", help="sweep the four corpora and save the record")
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--check", action="store_true", help="fail when the document is stale or a code is unowned")
    args = parser.parse_args(argv)

    if args.measure:
        if not args.binary.exists():
            print(f"construct_report: no binary at {args.binary}", file=sys.stderr)
            return 2
        record = measure(args.binary)
        RECORD.parent.mkdir(parents=True, exist_ok=True)
        RECORD.write_text(json.dumps(record, indent=2) + "\n")
        print(f"construct_report: wrote {RECORD.relative_to(ROOT)}")
    if not RECORD.exists():
        print(f"construct_report: no record at {RECORD}; run --measure", file=sys.stderr)
        return 2

    record = load(RECORD)
    fixes = load(FIXES) if FIXES.exists() else {}
    text, problems = render(record, fixes)

    if args.check:
        current = DOC.read_text() if DOC.exists() else ""
        stale = current != text
        if stale:
            print(f"construct_report: {DOC.relative_to(ROOT)} is stale", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1 if stale or problems else 0

    DOC.parent.mkdir(parents=True, exist_ok=True)
    DOC.write_text(text)
    print(f"construct_report: wrote {DOC.relative_to(ROOT)}")
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
