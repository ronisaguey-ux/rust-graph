#!/usr/bin/env python3
"""Shrink one file to the smallest that still shows a given loss.

    shrink_loss.py <file> <marker> [--bin PATH] [--out PATH] [--keep-head N]

`marker` is a substring that must appear in the RG_LOSSES output of a run, so a
value shape (`n:dyn`) survives the shrink even though line numbers move. Greedy
chunk removal: drop a chunk of n lines, keep the smaller file when the loss is
still there, halve n each pass, then a final pass one line at a time.

The point is the smallest file a person can read to see which reader is wrong,
so prints the survivor and its own loss line.
"""
import argparse
import pathlib
import subprocess
import sys
import tempfile

DEFAULT_BIN = "/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph"


def loses(lines, binary, marker, workdir):
    # `build` takes a directory and rejects a file ("is not a directory"), so the
    # shrunk source lives alone in a directory of its own and the run reads that.
    target = pathlib.Path(workdir) / "shrink.rs"
    target.write_text("\n".join(lines) + "\n")
    run = subprocess.run(
        [binary, "build", str(workdir), "--no-cache", "--stdout"],
        capture_output=True,
        text=True,
        env={"RG_LOSSES": "1", "PATH": "/usr/bin:/bin"},
    )
    return marker in run.stderr, run.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("marker")
    ap.add_argument("--bin", default=DEFAULT_BIN)
    ap.add_argument("--out", default="/tmp/shrunk.rs")
    args = ap.parse_args()

    lines = pathlib.Path(args.file).read_text().splitlines()
    workdir = tempfile.mkdtemp(prefix="shrink-")
    ok, _ = loses(lines, args.bin, args.marker, workdir)
    if not ok:
        sys.exit(f"{args.file} does not show {args.marker!r} at all")
    print(f"start: {len(lines)} lines", flush=True)

    size = max(1, len(lines) // 2)
    while True:
        dropped = 0
        i = 0
        while i < len(lines):
            candidate = lines[:i] + lines[i + size:]
            if loses(candidate, args.bin, args.marker, workdir)[0]:
                lines = candidate
                dropped += size
            else:
                i += size
        print(f"chunk {size}: -{dropped} lines -> {len(lines)}", flush=True)
        if size == 1 and dropped == 0:
            break
        size = max(1, size // 2)

    pathlib.Path(args.out).write_text("\n".join(lines) + "\n")
    _, stderr = loses(lines, args.bin, args.marker, workdir)
    loss = next((line for line in stderr.splitlines() if args.marker in line), "")
    print(f"shrunk: {len(lines)} lines -> {args.out}")
    print(loss[:400])


if __name__ == "__main__":
    main()
