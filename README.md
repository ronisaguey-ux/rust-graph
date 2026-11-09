# rust-graph

A code knowledge graph for **42 languages** in one static binary. No parser
libraries, no crates: Rust standard library only.

It walks a source tree, lexes each file once, and writes a single JSON document
with what it found — symbols, calls, imports and per-function value flow. It
reports facts and does not judge them. It has no idea what a source, a sink or a
vulnerability is; a reader decides that from the facts.

- **Zero dependencies.** `cargo build` needs nothing but a Rust toolchain. There
  is no `tree-sitter`, no `serde`, no `regex`; the SHA-256, the JSON writer and
  the lexer are in this repository.
- **Deterministic.** The same tree produces the same bytes, on any machine.
- **Bounded.** Facts spool to disk while the walk runs and the JSON is streamed
  out, so peak memory does not scale with the size of the graph.
- **Honest edges.** Every edge says whether the source states it (`EXTRACTED`) or
  a heuristic chose it (`INFERRED`, with the strategy named). Ambiguity is
  reported as `unresolved` rather than guessed.

## Why it exists

Most code-graph tools assume a C toolchain and a parser library per language, an
embedding database, or a hosted service. That is a fine trade for a workstation
and a poor one for a build box that must stay small, offline and reproducible.
rust-graph is the opposite bet: one binary you can `scp` anywhere, that reads a
tree in a fraction of a second and writes a file you can grep, diff and archive.

It is the graph engine behind a private static analyser, but nothing here is
specific to that use. If you have a tree of source and want to know what calls
what without installing anything, this is built for you.

## Quick start

```bash
git clone https://github.com/ronisaguey-ux/rust-graph
cd rust-graph

cargo build --release
./target/release/rust-graph build path/to/repo
```

Three files land in `path/to/repo/.rust-graph/`:

| File | What it is |
|:--|:--|
| `graph.json` | the graph, schema below |
| `report.md` | a short human summary: counts, languages, how calls resolved |
| `facts.pack` | the per-file fact cache, opaque and safe to delete |

For a fully static binary (no libc, runs on any Linux of the same architecture):

```bash
cargo build --release --target x86_64-unknown-linux-musl
```

Write somewhere else, or to stdout:

```bash
rust-graph build path/to/repo --out /tmp/graphs
rust-graph build path/to/repo --stdout | jq '.edges | length'
```

## Command line

```
rust-graph build <root> [--out DIR] [--stdout] [--cache DIR] [--no-cache]
                        [--max-files N] [--max-file-bytes N]
rust-graph languages
rust-graph --version
```

| Flag | Meaning |
|:--|:--|
| `--out DIR` | Write `graph.json` here instead of `<root>/.rust-graph`. |
| `--stdout` | Stream the graph to standard output; nothing is written. |
| `--cache DIR` | Keep the per-file fact cache here instead of `<root>/.rust-graph/cache`. |
| `--no-cache` | Parse everything; write no cache. |
| `--max-files N` | Stop walking after N files and set `truncated: true`. |
| `--max-file-bytes N` | Skip files larger than N bytes and count them in `too_large`. |

`languages` prints one line per extension: `extension`, `tag`, `name`. It is the
authoritative list and always matches the lexer.

Exit status is `0` on success and `2` on a usage error, an unreadable root or an
output path that cannot be written. A tree with no files is a success with an
empty graph — that is a fact about the tree, not a failure.

## Output

```json
{
  "version": 1, "tool": "rust-graph", "files": 2, "bytes": 126,
  "truncated": false, "too_large": 0,
  "sources":    [{"path": "app/a.py", "lang": "PY", "sha256": "…", "bytes": 83, "lines": 7}],
  "symbols":    [{"id": "app/a.py:main", "name": "main", "owner": "", "file": "app/a.py",
                  "line": 5, "end_line": 7, "kind": "function", "lang": "PY", "params": ["x"]}],
  "edges":      [{"caller": "app/a.py:main", "callee": "app/b.py:run", "line": 7,
                  "kind": "call", "tag": "EXTRACTED", "strategy": "import"}],
  "imports":    [{"file": "app/a.py", "line": 2, "module": "app.b", "name": "run", "alias": "run",
                  "star": false, "target": "app/b.py", "tag": "EXTRACTED"}],
  "unresolved": [],
  "flows":      [{"symbol": "app/a.py:main", "events": [], "calls": []}]
}
```

Paths are relative to the root and use `/` on every platform, so two graphs of
the same tree compare equal regardless of where it was checked out. Symbol ids
are `<path>:<qualified name>`, where the qualified name includes the owner for
methods (`app/views.py:B.get`).

`truncated` is true when `--max-files` stopped the walk, and `too_large` counts
files skipped by `--max-file-bytes`. **Check both before trusting a graph as
whole** — a truncated graph is a partial answer, not a clean one.

### What an edge means

| Field | Values |
|:--|:--|
| `kind` | `call`, `inherit`, `include` |
| `tag` | `EXTRACTED` — an import or a definition in the same file states it; `INFERRED` — a heuristic chose the target |
| `strategy` | the heuristic that ran: `import`, `same-file`, `same-directory`, `qualified`, `unique-name`, `export-map` |
| `line` | where the call appears, 1-based |

A call name with more than one plausible target goes to `unresolved` with the
candidates, never to `edges`. A graph that resolves nothing is not wrong; it is
telling you the tree did not give it enough to decide.

### Value flow

`flows` holds, per function, the local events (`assign`, `param`, `return`,
`call`) and the calls it makes, in source order. This is what lets a reader ask
"does this parameter reach that call" without a type system. It is a record of
what the function does with its values, not a dataflow solution: no aliasing
beyond direct assignment, no points-to analysis, no interprocedural fixpoint.

## Languages

42 languages, 89 extensions. `.m` is resolved by content — Objective-C and
MATLAB look alike at a glance and do not at the first line.

| Language | Extensions |
|:--|:--|
| C | `.c` `.h` |
| C++ | `.cc` `.cpp` `.cxx` `.hh` `.hpp` `.hxx` `.ipp` |
| C# | `.cs` |
| Clojure | `.clj` `.cljc` `.cljs` |
| COBOL | `.cbl` `.cob` `.cpy` |
| Crystal | `.cr` |
| D | `.d` |
| Dart | `.dart` |
| Elixir | `.ex` `.exs` |
| Elm | `.elm` |
| Erlang | `.erl` `.hrl` |
| F# | `.fs` `.fsx` |
| Fortran | `.f` `.f03` `.f08` `.f90` `.f95` `.for` |
| Go | `.go` |
| Groovy | `.gradle` `.groovy` |
| Haskell | `.hs` `.lhs` |
| Java | `.java` |
| JavaScript | `.js` `.jsx` `.mjs` `.cjs` |
| Julia | `.jl` |
| Kotlin | `.kt` `.kts` |
| Lua | `.lua` |
| Nim | `.nim` |
| Objective-C | `.mm` `.m` (by content) |
| OCaml | `.ml` `.mli` |
| Pascal | `.pas` `.pp` `.dpr` |
| Perl | `.pl` `.pm` |
| PHP | `.php` `.phtml` |
| PowerShell | `.ps1` `.psm1` |
| Python | `.py` `.pyi` |
| R | `.r` `.rmd` |
| Ruby | `.rb` `.rake` `.gemspec` |
| Rust | `.rs` |
| Scala | `.sc` `.scala` |
| Shell | `.sh` `.bash` `.zsh` `.ksh` |
| Solidity | `.sol` |
| SQL | `.sql` `.ddl` |
| Swift | `.swift` |
| Tcl | `.tcl` |
| TypeScript | `.ts` `.tsx` `.mts` `.cts` |
| Visual Basic | `.vb` `.bas` `.cls` `.vbs` |
| Zig | `.zig` |
| AWK | `.awk` |

Adding a language means adding rows to one table (`src/langs.rs`) — extension,
tag, keyword sets, comment style, definition patterns. There is no grammar to
generate and no build step.

## How it works

```
walk  →  lex  →  parse  →  facts  →  resolve  →  stream JSON
```

1. **Walk.** Deterministic order, symlinks not followed, binary and oversized
   files skipped and counted. File content is hashed as it is read.
2. **Lex.** One pass per file with a per-language table: strings, comments,
   numbers, identifiers. No syntax tree is built.
3. **Parse.** Block and scope tracking finds definitions (function, method,
   class, struct, interface, trait, module, macro) and the calls inside them.
4. **Facts.** Each file's symbols, imports, calls and flows are written to the
   cache keyed by its SHA-256, so an unchanged file is never lexed twice.
5. **Resolve.** Imports first (exact, then module-relative, then package root),
   then same-file, then same-directory, then a unique name across the tree.
   Anything ambiguous is left unresolved.
6. **Stream.** Sources, symbols, edges, imports and flows are written in chunks.
   The graph is never materialised whole in memory.

## Caching

Facts are cached in one binary pack, `<root>/.rust-graph/facts.pack`: a magic
header, the fact-format version, then one row per path with its SHA-256 and its
facts. A rebuild re-hashes every file, reuses the rows whose hash is unchanged,
and parses only the rest, then re-resolves the whole tree — resolution is cheap,
parsing is not.

The pack is versioned: a pack from another fact format is ignored rather than
trusted, so an upgrade re-parses instead of producing a half-old graph. A torn or
unreadable pack is treated the same way. `--no-cache` bypasses all of it and
writes none.

Two consequences worth knowing: rebuilding after touching one file is fast, and
`git status` in the analysed repository will show `.rust-graph/` as untracked
unless you add it to `.gitignore`.

## Performance

Numbers from this repository's benchmark, on one core of the development
machine. The trees are synthetic Python, JavaScript and C files generated by
`tests/benchmark.rs`, about 2.4 KB each. Real code varies. Reproduce with:

```bash
cargo test --release --target x86_64-unknown-linux-musl --test benchmark -- --nocapture --test-threads=1
```

| Measure | Target | Measured |
|:--|--:|--:|
| Static binary size | under 2 MB | 840,368 bytes |
| Cold start | under 50 ms | 0.28 ms |
| 1,000 files, median | under 300 ms | 224 ms |
| 10,000 files, peak RSS | under 64 MB | 9.8 MB |
| 10,000 files, time | none set | 2.2 s |

The 10,000-file graph is 72 MB of JSON. Peak RSS stays low because facts spool to
disk while the walk runs and the JSON is streamed out, so the graph is never held
in memory whole. A reader that loads that JSON pays for it again, and a JSON
parser in a scripting language can need several times the file size.

This table compares rust-graph with nothing. The one comparison with another
tool is the parser gate below.

### The fact layer

A tree on its own is not a result. `--deep` exists to produce the same facts the
one-pass lexer produces — symbols, calls, imports, flow events — with more
information about each. `src/deep_facts.rs` holds one table per language, and
`lang_for` is the list of languages a file may take the CST path for:

| language | tags | what the CST adds |
|:--|:--|:--|
| Python | `PY` | f-string fields, annotated assignments, assigned lambdas, multi-target patterns |
| JavaScript, TypeScript, TSX | `JS`, `TS` | template substitutions, tagged templates |
| Rust | `RS` | `impl` owners, `use` paths, struct-literal values |
| Go | `GO` | receiver-named methods, interface signatures |
| Java | `JAVA` | heritage clauses, typed parameters |

Every other grammar in `crates/deep-grammars` parses and is exercised by the
fuzz and conformance harnesses, but has no fact table yet, so its files stay on
the lexer path. That is the largest remaining gap and it is a coverage
decision, not a parser defect.

**The guard.** `pipeline::loses` parses a file both ways and compares the
records fact by fact — imports, symbols, calls, class bindings, and the shape of
every flow event (targets, and the names, calls and constants in each value).
Any loss keeps the lexer's record and is counted as a fallback; the JSON says
which path won per file (`path_choice`) and the counts (`paths`). `--compare`
prints both sides without choosing, which is how the numbers below were
measured.

Measured on 2026-11-09 (commands in `docs/DEEP_PARSER.md` §11):

| tree | files | calls deep / lexer | imports deep / lexer |
|:--|--:|--:|--:|
| Felicity (Python) | 68 | 5587 / 5547 | 710 / 710 |
| rust-graph itself (Rust) | 29 | 7630 / 7386 | 304 / 295 |
| webchat_harness runtime (JS) | 11 | 547 / 541 | — |

### Reproducing the parser gate

The deep scan (`--features parser`, the `rust-graph-deep-scan` binary) has a
real parser per language. Each language ships only if it parses real code at
least **2× faster than Tree-sitter** on the corpora in `bench/corpus`. The gate
is a test, `tests/parser_gate.rs`, and it fails when any corpus drops below 2×
or below the clean-file count its list states. To run it:

```bash
sh bench/build-tsdrv.sh      # once: fetches pinned Tree-sitter sources, builds target/bench/tsdrv
cargo test --release --features parser --test parser_gate -- --nocapture
```

The first command needs git, a C compiler and the network. It fetches
Tree-sitter v0.27.2 and each grammar at the commit pinned in the script, and
builds `bench/tsdrv.c` against them at `-O3`. The test then takes about
40 seconds for Python. Each list in `bench/corpus` names its files by absolute
path with their size and SHA-256. The test checks every file before timing and
fails if one is missing or changed. The Python lists name files that Ubuntu
26.04 installs. Elsewhere, put a copy of those files under a directory and set
`RG_CORPUS_ROOT=DIR`.

Each round runs both parsers on the whole corpus. Every file is parsed 5 times
and the fastest time counts. The two parsers alternate going first. The test
takes the median of 7 rounds and pins both to one CPU with `taskset` when it is
installed. `RG_GATE_CPU=N` chooses the CPU, `RG_GATE_ROUNDS=N` sets the number
of rounds, and `RG_GATE_ONLY=python-a` runs only the lists you name. You should
see one summary line per corpus, ending in `ok`. These are the 2026-11-08
numbers on the development machine (8 cores, pinned to CPU 7):

| Corpus | Files | Bytes | Deep scan | Tree-sitter | Speed-up | Clean files (deep / Tree-sitter) |
|:--|--:|--:|--:|--:|--:|--:|
| python-a (CPython 3.14 stdlib) | 200 | 3,423,925 | 36.7 ms | 318.0 ms | 8.7× | 200 / 200 |
| python-b (Ubuntu dist-packages) | 400 | 5,528,880 | 71.2 ms | 611.1 ms | 8.6× | 400 / 400 |

The ratio matters, not the milliseconds. On another machine, expect different
times and a similar ratio.

## Limits

- **It is less precise than tree-sitter.** There is no grammar and no syntax tree.
  One-pass lexers and block tracking get the common shapes right and can misread
  unusual ones, such as heavy macros, nested templates or code that builds code.
- **It is not a compiler.** There are no types, no overload resolution, and
  dynamic dispatch resolves only when a name has one candidate.
- **Resolution is a heuristic where it says so.** Treat `INFERRED` edges as leads.
- **Coverage varies.** Mainstream languages get fuller value flow than the long
  tail, where the graph may hold little more than symbols and calls.
- **Symbolic links are not followed**, and named pipes and devices are skipped.
- **Memory is not fully returned.** The allocator recycles small blocks through
  free lists rather than handing them back, so they stay bounded by the peak of
  live small data. Blocks above its largest class go to the system allocator.

## Allocator

`src/alloc.rs` is a small size-class allocator. A parse makes millions of
short-lived small strings, and musl's `malloc` is slow at that: replacing it took
1,000 files from 343 ms to 216 ms on the static build. It is the only `unsafe`
code path of any size, and it has its own unit tests.

## Design rules

1. **No crates, ever.** A dependency is a supply-chain surface and a build
   requirement. Everything needed is in this repository.
2. **Deterministic output.** Same tree, same bytes — no hash-map iteration order
   leaking into the file, no timestamps, no absolute paths.
3. **Never guess silently.** Ambiguity is reported, not resolved by coin toss.
4. **Bounded memory.** Streaming and spooling, not "it fits on my machine".
5. **Facts, not verdicts.** The graph says what the code contains; callers decide
   what it means.

## Prior art

[rust-graph-research.md](rust-graph-research.md) covers the tools surveyed before
and after this was written: licences, language coverage, schemas, incremental
updates, memory and MCP interfaces, what was rejected and why. No code from any
of them is included here.

## Contributing

Build, run the tests, and keep the output deterministic:

```bash
cargo test --release
cargo test --release --target x86_64-unknown-linux-musl
```

A language is a table row plus a fixture; a resolution strategy needs a test that
fails without it. If you add a dependency, the change will be rejected — that is
the point of the project, not a technicality.

## License

GNU Affero General Public License v3.0. See [LICENSE](LICENSE).

AGPL, not MIT, on purpose: this engine is meant to stay open. Anyone who
runs a modified rust-graph as a network service must publish their changes,
which is the point of building it in public and keeping the analyser that
consumes it private.
