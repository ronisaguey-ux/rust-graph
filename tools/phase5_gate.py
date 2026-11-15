#!/usr/bin/env python3
"""phase5_gate.py — the phase-5 exit gate, in one command.

Four conditions, and each one runs something rather than reading a number
somebody typed:

1. **rust at or below 120.** `tools/phase4_gate.py` sweeps the four corpora and
   refuses a moved pin, so this is the same measurement the phase has used.
2. **the arm-merge is proven on its reproduction.** `cargo test --test
   rust_arm_merge` runs the four-line reproduction: the arm body merges its
   statements on the arm's first line, both readers stamp the same lines, and an
   arm block opens no frame of its own.
3. **no other corpus moved.** js, ts and python are in the same phase-4 gate with
   their own pins; the gate fails when one of them moves in either direction.
4. **the workspace still parses clean.** The binary builds this repository's own
   `src` tree and reports zero fallbacks.

    python3 tools/phase5_gate.py                 # all four
    python3 tools/phase5_gate.py --skip-corpora  # skip the slow sweep
    python3 tools/phase5_gate.py --skip-tests    # skip the cargo test

Exit 0 only when every condition it ran holds.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
DEFAULT_BINARY = Path(
    os.environ.get(
        "RG_BIN",
        "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph",
    )
)
RUST_LIMIT = 120


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=ROOT, capture_output=True, text=True, **kwargs)


def corpora(binary: Path) -> tuple[bool, str]:
    result = run([sys.executable, "tools/phase4_gate.py", "--binary", str(binary)])
    tail = (result.stdout + result.stderr).strip().splitlines()
    summary = " / ".join(line.strip() for line in tail if "fallbacks" in line)
    if result.returncode != 0:
        return False, summary or (result.stderr.strip()[:300])
    rust = None
    for line in tail:
        words = line.split()
        if len(words) > 2 and words[0] == "rust":
            rust = int(words[1])
    if rust is None:
        return False, "the phase-4 gate printed no rust row"
    if rust > RUST_LIMIT:
        return False, f"rust {rust} fallbacks is over the phase-5 limit {RUST_LIMIT}"
    return True, f"rust {rust} <= {RUST_LIMIT}; {summary}"


def arm_merge() -> tuple[bool, str]:
    env = dict(os.environ)
    env.setdefault("CARGO_TARGET_DIR", str(ROOT.parent / "rg-kbuilds" / "verify"))
    result = run(
        ["cargo", "test", "--features", "deep parser", "--test", "rust_arm_merge"],
        env=env,
    )
    if result.returncode != 0:
        lines = [line for line in (result.stdout + result.stderr).splitlines() if "FAILED" in line or "panicked" in line]
        return False, "; ".join(lines[:3]) or (result.stderr.strip()[:300])
    return True, "the arm-merge reproduction passes"


def workspace(binary: Path) -> tuple[bool, str]:
    result = run([str(binary), "build", str(ROOT / "src"), "--no-cache"])
    if result.returncode != 0:
        return False, f"the build exited {result.returncode}: {result.stderr.strip()[:200]}"
    import re
    output = result.stdout + result.stderr
    line = next((line for line in output.splitlines() if "fallbacks" in line), "")
    if " 0 fallbacks" not in line:
        return False, line or "no fallback count in the build output"
    found = re.search(r"(\d+) files", output)
    files = found.group(1) if found else "?"
    return True, f"{files} files, 0 fallbacks"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--skip-corpora", action="store_true")
    parser.add_argument("--skip-tests", action="store_true")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args(argv)

    checks: list[tuple[str, bool, str]] = []
    if not args.skip_corpora:
        checks.append(("corpora", *corpora(args.binary)))
    if not args.skip_tests:
        checks.append(("arm-merge", *arm_merge()))
    checks.append(("workspace", *workspace(args.binary)))

    for name, ok, detail in checks:
        print(f"  {name:12} {'ok  ' if ok else 'FAIL'} {detail}")
    failed = [name for name, ok, _ in checks if not ok]
    print("PHASE5 GATE " + ("PASSED" if not failed else "FAILED: " + ", ".join(failed)))
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps({name: {"ok": ok, "detail": detail} for name, ok, detail in checks}, indent=2) + "\n")
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
