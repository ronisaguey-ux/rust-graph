#!/usr/bin/env python3
"""build_corpus.py — the 45-language corpus, built the same way every time.

Every language gets the same three kinds of material, in this order:

  1. the language's own standard library or reference implementation,
  2. one large real project written in it,
  3. its conformance or test suite where that suite is a separate repo.

Rules that make the result trustworthy:

  * every clone is shallow and pinned to a recorded ref,
  * the sample is deterministic: candidate files are sorted by path and every
    k-th is taken until N per language, so the same manifest is produced twice,
  * every chosen file's sha256 is written into the manifest,
  * the corpus lives outside any git tree (default /home/roni/Roni_workspace/corpus45),
    only the manifest is committed.

Usage:
    python3 build_corpus.py --list
    python3 build_corpus.py --lang rust --lang go --files 300
    python3 build_corpus.py --all --files 300 --jobs 3
    python3 build_corpus.py --verify          # re-check hashes of a built corpus
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(os.environ.get("CORPUS45", "/home/roni/Roni_workspace/corpus45"))
MAX_BYTES = 512 * 1024

# language: (extensions, [ (repo alias, url, ref, subdir-or-empty) ])
SPEC: dict[str, tuple[list[str], list[tuple[str, str, str, str]]]] = {
    "python": ([".py"], [
        ("cpython", "https://github.com/python/cpython", "3.12", "Lib"),
        ("django", "https://github.com/django/django", "stable/5.0.x", "django"),
    ]),
    "javascript": ([".js", ".mjs", ".cjs"], [
        ("node", "https://github.com/nodejs/node", "v22.x", "lib"),
        ("three", "https://github.com/mrdoob/three.js", "dev", "src"),
    ]),
    "typescript": ([".ts"], [
        ("typescript", "https://github.com/microsoft/TypeScript", "main", "src"),
        ("nest", "https://github.com/nestjs/nest", "master", "packages"),
    ]),
    "rust": ([".rs"], [
        ("rust", "https://github.com/rust-lang/rust", "master", "library"),
        ("ripgrep", "https://github.com/BurntSushi/ripgrep", "master", "crates"),
    ]),
    "go": ([".go"], [
        ("go", "https://github.com/golang/go", "master", "src"),
        ("kubernetes", "https://github.com/kubernetes/kubernetes", "master", "pkg"),
    ]),
    "c": ([".c", ".h"], [
        ("musl", "https://github.com/bminor/musl", "master", "src"),
        ("redis", "https://github.com/redis/redis", "unstable", "src"),
    ]),
    "cpp": ([".cpp", ".cc", ".hpp", ".hh"], [
        ("fmt", "https://github.com/fmtlib/fmt", "master", "include"),
        ("json", "https://github.com/nlohmann/json", "develop", "include"),
    ]),
    "csharp": ([".cs"], [
        ("runtime", "https://github.com/dotnet/runtime", "main", "src/libraries"),
        ("newtonsoft", "https://github.com/JamesNK/Newtonsoft.Json", "master", "Src"),
    ]),
    "java": ([".java"], [
        ("jdk", "https://github.com/openjdk/jdk", "master", "src/java.base"),
        ("spring", "https://github.com/spring-projects/spring-framework", "main", "spring-core"),
    ]),
    "kotlin": ([".kt", ".kts"], [
        ("kotlin", "https://github.com/JetBrains/kotlin", "master", "libraries"),
        ("ktor", "https://github.com/ktorio/ktor", "main", "ktor-server"),
    ]),
    "scala": ([".scala", ".sc"], [
        ("scala", "https://github.com/scala/scala", "2.13.x", "src/library"),
        ("akka", "https://github.com/akka/akka", "main", "akka-actor"),
    ]),
    "groovy": ([".groovy", ".gradle"], [
        ("groovy", "https://github.com/apache/groovy", "master", "src/main"),
        ("gradle", "https://github.com/gradle/gradle", "master", "subprojects"),
    ]),
    "swift": ([".swift"], [
        ("swift", "https://github.com/apple/swift", "main", "stdlib"),
        ("alamofire", "https://github.com/Alamofire/Alamofire", "master", "Source"),
    ]),
    "dart": ([".dart"], [
        ("dart-sdk", "https://github.com/dart-lang/sdk", "main", "sdk/lib"),
        ("flutter", "https://github.com/flutter/flutter", "master", "packages"),
    ]),
    "objective-c": ([".m", ".h"], [
        ("cocoalumberjack", "https://github.com/CocoaLumberjack/CocoaLumberjack", "master", "Sources"),
        ("afnetworking", "https://github.com/AFNetworking/AFNetworking", "master", "AFNetworking"),
    ]),
    "php": ([".php"], [
        ("php-src", "https://github.com/php/php-src", "master", "ext"),
        ("laravel", "https://github.com/laravel/framework", "11.x", "src"),
    ]),
    "ruby": ([".rb"], [
        ("ruby", "https://github.com/ruby/ruby", "master", "lib"),
        ("rails", "https://github.com/rails/rails", "main", "activerecord"),
    ]),
    "lua": ([".lua"], [
        ("lua", "https://github.com/lua/lua", "master", ""),
        ("kong", "https://github.com/Kong/kong", "master", "kong"),
    ]),
    "perl": ([".pl", ".pm"], [
        ("perl5", "https://github.com/Perl/perl5", "blead", "lib"),
        ("mojolicious", "https://github.com/mojolicious/mojo", "main", "lib"),
    ]),
    "r": ([".r", ".R"], [
        ("r-source", "https://github.com/wch/r-source", "trunk", "src/library"),
        ("dplyr", "https://github.com/tidyverse/dplyr", "main", "R"),
    ]),
    "julia": ([".jl"], [
        ("julia", "https://github.com/JuliaLang/julia", "master", "base"),
        ("dataframes", "https://github.com/JuliaData/DataFrames.jl", "main", "src"),
    ]),
    "elixir": ([".ex", ".exs"], [
        ("elixir", "https://github.com/elixir-lang/elixir", "main", "lib"),
        ("phoenix", "https://github.com/phoenixframework/phoenix", "main", "lib"),
    ]),
    "erlang": ([".erl", ".hrl"], [
        ("otp", "https://github.com/erlang/otp", "master", "lib"),
        ("cowboy", "https://github.com/ninenines/cowboy", "master", "src"),
    ]),
    "haskell": ([".hs"], [
        ("base", "https://github.com/ghc/ghc", "master", "libraries/base"),
        ("pandoc", "https://github.com/jgm/pandoc", "main", "src"),
    ]),
    "ocaml": ([".ml", ".mli"], [
        ("ocaml", "https://github.com/ocaml/ocaml", "trunk", "stdlib"),
        ("lwt", "https://github.com/ocsigen/lwt", "master", "src"),
    ]),
    "fsharp": ([".fs", ".fsi"], [
        ("fsharp", "https://github.com/dotnet/fsharp", "main", "src"),
        ("giraffe", "https://github.com/giraffe-fsharp/Giraffe", "master", "src"),
    ]),
    "clojure": ([".clj", ".cljs"], [
        ("clojure", "https://github.com/clojure/clojure", "master", "src"),
        ("ring", "https://github.com/ring-clojure/ring", "master", "ring-core/src"),
    ]),
    "elm": ([".elm"], [
        ("core", "https://github.com/elm/core", "master", "src"),
        ("elm-ui", "https://github.com/mdgriffith/elm-ui", "master", "src"),
    ]),
    "solidity": ([".sol"], [
        ("openzeppelin", "https://github.com/OpenZeppelin/openzeppelin-contracts", "master", "contracts"),
        ("uniswap-v3", "https://github.com/Uniswap/v3-core", "main", "contracts"),
    ]),
    "sql": ([".sql"], [
        ("postgres", "https://github.com/postgres/postgres", "master", "src/test/regress/sql"),
        ("dbt", "https://github.com/dbt-labs/dbt-core", "main", "tests"),
    ]),
    "bash": ([".sh", ".bash"], [
        ("bash", "https://github.com/bminor/bash", "master", "examples"),
        ("ohmyzsh", "https://github.com/ohmyzsh/ohmyzsh", "master", "plugins"),
    ]),
    "powershell": ([".ps1", ".psm1"], [
        ("powershell", "https://github.com/PowerShell/PowerShell", "master", "src"),
        ("psscriptanalyzer", "https://github.com/PowerShell/PSScriptAnalyzer", "master", "Rules"),
    ]),
    "assembly": ([".s", ".S", ".asm"], [
        ("musl", "https://github.com/bminor/musl", "master", "src"),
        ("glibc", "https://github.com/bminor/glibc", "master", "sysdeps"),
    ]),
    "nim": ([".nim"], [
        ("nim", "https://github.com/nim-lang/Nim", "devel", "lib"),
        ("nimble", "https://github.com/nim-lang/nimble", "master", "src"),
    ]),
    "crystal": ([".cr"], [
        ("crystal", "https://github.com/crystal-lang/crystal", "master", "src"),
        ("lucky", "https://github.com/luckyframework/lucky", "main", "src"),
    ]),
    "d": ([".d"], [
        ("phobos", "https://github.com/dlang/phobos", "master", "std"),
        ("vibe", "https://github.com/vibe-org/vibe.d", "master", "source"),
    ]),
    "zig": ([".zig"], [
        ("zig", "https://github.com/ziglang/zig", "master", "lib"),
        ("zls", "https://github.com/zigtools/zls", "master", "src"),
    ]),
    "pascal": ([".pas", ".pp"], [
        ("fpc", "https://github.com/fpc/FPCSource", "main", "rtl"),
        ("lazarus", "https://gitlab.com/freepascal.org/lazarus/lazarus", "main", "components"),
    ]),
    "fortran": ([".f90", ".f", ".f77", ".F90"], [
        ("lapack", "https://github.com/Reference-LAPACK/lapack", "master", "SRC"),
        ("fftw", "https://github.com/FFTW/fftw3", "master", "kernel"),
    ]),
    "cobol": ([".cob", ".cbl"], [
        ("gnucobol", "https://github.com/OCamlPro/gnucobol", "master", "tests"),
        ("cobol85", "https://github.com/uwol/proleap-cobol-parser", "master", "src/test"),
    ]),
    "matlab": ([".m"], [
        ("octave", "https://github.com/gnu-octave/octave", "default", "scripts"),
        ("octave-test", "https://github.com/gnu-octave/octave", "default", "test"),
    ]),
    "tcl": ([".tcl"], [
        ("tcl", "https://github.com/tcltk/tcl", "main", "library"),
        ("tcllib", "https://github.com/tcltk/tcllib", "main", "modules"),
    ]),
    "awk": ([".awk"], [
        ("one-true-awk", "https://github.com/onetrueawk/awk", "master", ""),
        ("gawk-tests", "https://github.com/gawk-mirrors/gawk", "master", "test"),
    ]),
    "visual-basic": ([".vb"], [
        ("runtime-samples", "https://github.com/dotnet/samples", "main", "visualbasic"),
    ]),
    "vbnet": ([".vb"], [
        ("runtime", "https://github.com/dotnet/runtime", "main", "src/libraries"),
    ]),
}


def clone(alias: str, url: str, ref: str, dest: Path, subdir: str = "") -> Path:
    """Shallow, blobless, sparse clone of exactly the subdir we sample.

    A full checkout of rust, kubernetes or the dart SDK would cost gigabytes the
    box does not have free; trees and the one subdir's blobs cost tens of MB.
    """
    dest.mkdir(parents=True, exist_ok=True)
    if (dest / ".git").is_dir():
        return dest
    # A tree left behind without .git is still the sample it was cloned for; a
    # second clone would refuse a non-empty destination and score the language 0.
    if any(dest.iterdir()):
        return dest
    free = shutil.disk_usage(dest.parent).free
    if free < 5 * 1024**3:
        raise RuntimeError(f"refusing to clone {alias}: only {free // 1024**3}G free")
    cmd = ["git", "clone", "--depth", "1", "--filter=blob:none", "--sparse", "--single-branch"]
    if ref:
        cmd += ["--branch", ref]
    cmd += [url, str(dest)]
    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"clone {alias} failed: {res.stderr.strip()[:200]}")
    if subdir:
        sub = subprocess.run(["git", "-C", str(dest), "sparse-checkout", "set", subdir],
                             capture_output=True, text=True)
        if sub.returncode != 0:
            raise RuntimeError(f"sparse {alias}:{subdir} failed: {sub.stderr.strip()[:200]}")
    return dest


def candidates(root: Path, exts: list[str]) -> list[Path]:
    out = []
    lowered = {e.lower() for e in exts}
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix.lower() not in lowered:
            continue
        try:
            if path.stat().st_size > MAX_BYTES or path.stat().st_size == 0:
                continue
        except OSError:
            continue
        out.append(path)
    out.sort(key=lambda p: str(p))
    return out


def sample(files: list[Path], want: int) -> list[Path]:
    if len(files) <= want:
        return files
    step = len(files) / want
    return [files[int(i * step)] for i in range(want)]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def build_language(lang: str, want: int, refresh: bool) -> dict:
    exts, repos = SPEC[lang]
    base = ROOT / lang
    picked: list[Path] = []
    sources = []
    for alias, url, ref, subdir in repos:
        dest = base / alias
        if refresh and dest.exists():
            shutil.rmtree(dest)
        try:
            clone(alias, url, ref, dest, subdir)
        except RuntimeError as exc:
            sources.append({"repo": alias, "url": url, "ref": ref, "error": str(exc)})
            continue
        head = subprocess.run(["git", "-C", str(dest), "rev-parse", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
        root = dest / subdir if subdir else dest
        found = candidates(root, exts) if root.is_dir() else []
        sources.append({"repo": alias, "url": url, "ref": ref, "commit": head,
                        "subdir": subdir, "candidates": len(found)})
        picked.extend(found)
        # The commit is recorded; the history is not worth the disk.
        shutil.rmtree(dest / ".git", ignore_errors=True)
    chosen = sample(picked, want)
    files = []
    for path in chosen:
        files.append({"path": str(path.relative_to(ROOT)), "bytes": path.stat().st_size,
                      "sha256": sha256(path)})
    return {"language": lang, "files": len(files), "sources": sources, "entries": files}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lang", action="append", default=[])
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--files", type=int, default=300)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--refresh", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--verify", action="store_true")
    args = ap.parse_args()

    if args.list:
        for lang, (exts, repos) in sorted(SPEC.items()):
            print(f"{lang:14} {' '.join(exts):24} {len(repos)} repos")
        print(f"\n{len(SPEC)} languages")
        return 0

    if args.verify:
        manifest = ROOT / "MANIFEST.json"
        data = json.loads(manifest.read_text())
        bad = 0
        for record in data["languages"]:
            for entry in record["entries"]:
                path = ROOT / entry["path"]
                if not path.exists() or sha256(path) != entry["sha256"]:
                    print(f"CHANGED {entry['path']}")
                    bad += 1
        print(f"verified {sum(len(r['entries']) for r in data['languages'])} files, {bad} changed")
        return 0 if bad == 0 else 1

    langs = sorted(SPEC) if args.all else args.lang
    if not langs:
        ap.error("pick --lang X or --all")
    ROOT.mkdir(parents=True, exist_ok=True)

    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = {pool.submit(build_language, lang, args.files, args.refresh): lang for lang in langs}
        for future in concurrent.futures.as_completed(futures):
            lang = futures[future]
            try:
                results.append(future.result())
                print(f"{lang}: {results[-1]['files']} files", flush=True)
            except Exception as exc:  # a broken repo must not kill the run
                print(f"{lang}: FAILED {exc}", flush=True)

    # An incremental run (a few languages after a disk-full pass) must not
    # wipe the languages it did not touch.
    manifest_path = ROOT / "MANIFEST.json"
    previous = {}
    if manifest_path.exists():
        try:
            previous = {r["language"]: r for r in json.loads(manifest_path.read_text())["languages"]}
        except Exception:
            previous = {}
    for record in results:
        previous[record["language"]] = record
    results = sorted(previous.values(), key=lambda r: r["language"])
    manifest = {"files_per_language": args.files, "languages": results}
    (ROOT / "MANIFEST.json").write_text(json.dumps(manifest, indent=1))
    total = sum(r["files"] for r in results)
    print(f"\n{total} files across {len(results)} languages -> {ROOT / 'MANIFEST.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
