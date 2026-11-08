"""A small argument-driven command line tool."""

import argparse
import shutil
import sys
from pathlib import Path


def build_parser():
    parser = argparse.ArgumentParser(description="copy files by suffix")
    parser.add_argument("source", type=Path)
    parser.add_argument("target", type=Path)
    parser.add_argument("-s", "--suffix", action="append", default=[".py"])
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("-v", "--verbose", action="count", default=0)
    return parser


def matches(path, suffixes):
    return any(path.name.endswith(suffix) for suffix in suffixes)


def run(args):
    if not args.source.is_dir():
        print(f"{args.source} is not a directory", file=sys.stderr)
        return 2
    copied = 0
    for path in args.source.rglob("*"):
        if not path.is_file() or not matches(path, args.suffix):
            continue
        relative = path.relative_to(args.source)
        destination = args.target / relative
        if args.verbose:
            print(f"{path} -> {destination}")
        if not args.dry_run:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, destination)
        copied += 1
    return 0 if copied else 1


def main(argv=None):
    args = build_parser().parse_args(argv)
    match args.verbose:
        case 0:
            level = "quiet"
        case 1:
            level = "normal"
        case _:
            level = "noisy"
    print(f"mode: {level}")
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
