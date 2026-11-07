# Prior art for rust-graph

Written before any engine code, except the section "Added after the engine
started", which says so. Every other repository below was read on its default
branch on 2026-11-07. Star counts, licences and last-push dates come from the
GitHub API. A claim names the file it was read from. Claims that come only from
a README say so, and anything not confirmed is marked **unverified**.

What this document does not contain: no benchmark of any of these tools was run
here, so every speed or memory figure quoted is the project's own. No code from
any of them was copied or executed. Files were fetched and read as text.

## What the engine needs

rust-graph has to serve a taint solver, so the questions that matter are:

- whether calls, imports and per-function value flow are all available
- whether one edit avoids a full re-index
- whether memory stays bounded on a large tree

It also has constraints most of the field does not share: Rust std only, no
crates, no tree-sitter, one pass per file, and a static binary under 2 MB.

## Corrections to the brief

The brief's descriptions did not all match the sources.

| Brief said | Source says |
|:--|:--|
| Arbor-Vitae: 12 edge kinds incl. Modifies/Reads, petgraph | The Arbor-Vitae paper lists call, inheritance, re-export and method-override edges, and **twelve MCP tools**. It names no repository and no petgraph. The petgraph design is in a different project, `Anandb71/arbor`, which has 9 edge kinds and no Modifies/Reads. Whether the two are related is **unverified**. |
| Codebase-Memory: 66 languages, XXH3 content hash | 66 is the arXiv v1 figure. The README now says 162 vendored grammars. `pipeline_incremental.c` compares mtime+size and includes a SHA-256 header. No xxh file appears in the repo's file listing (wyhash is vendored), so XXH3 is **unverified**, and it looks stale. The licence is MIT. |
| Joern models taint with Modifies/Reads edges | No such edges appear in the files read. Joern computes reaching definitions per method and emits data-dependence edges. Call behaviour comes from a semantics table of argument mappings. |
| orgraph-mcp: trace, find_path, get_context | Confirmed, plus search, find_entry_points, get_dependencies and reindex. The README says "6 tools" and then lists 7. |
| A local copy of Graphify | That path does not exist on this machine. a private tools directory holds only `indexer.py`. Graphify was studied from `Graphify-Labs/graphify` upstream instead, so branch v8 was **not** read. |

## The seven named tools

### 1. codelens-engine

`mupozg823/codelens-mcp-plugin`, crate `codelens-engine` 1.13.34. 3 stars, active.

- **Licence.** Apache-2.0 (crates.io, GitHub API). Shapes only; nothing vendored.
- **Languages.** 25 per the docs.rs README of 1.9.x, via tree-sitter grammars.
  The count for 1.13 is **unverified**.
- **Schema.** `call_graph/types.rs`: `CallEdge` holds the caller, callee name,
  qualifier, line, `resolved_file`, `confidence: f64` and `resolution_strategy`.
  `db/mod.rs` has `files`, `symbols`, `imports` and `calls` tables. There are no
  value-flow edges.
- **Cross-file resolution.** This is the best part. `call_graph/resolve.rs` runs
  a "6-stage confidence cascade": owner-qualified 0.98, same file 0.90, import
  map 0.95, re-export 0.93, import suffix 0.70, unique name 0.65, path proximity
  0.50, unresolved 0.25. Each edge records which strategy won.
- **Incremental.** `db/mod.rs` keeps `mtime_ms` and `content_hash` per file, with
  SHA-256 imported. A `notify` watcher is listed in `docs/architecture.md`.
- **Memory.** On-disk SQLite. The comment there says `mmap_size`/`cache_size` are
  tuned for a 1+ GB database. No RSS figure was found: **unverified**.
- **MCP.** 90+ tools per the README; `docs/architecture.md` has the registry.

### 2. Arbor-Vitae, and Anandb71/arbor

Arbor-Vitae is a Zenodo record (10.5281/zenodo.20549779, CC BY 4.0), with no
repository linked. It covers 6 languages and 12 MCP tools and gives no memory
figures. Its self-annotated impact F1 of 0.816 uses n=8. No code could be read,
so licence, schema detail and incremental behaviour are **unverified**.

`Anandb71/arbor` (162 stars, MIT, active) carries the petgraph design:

- **Schema.** `crates/arbor-graph/src/edge.rs`: Calls, Imports, Extends,
  Implements, UsesType, References, Contains, FlowsTo, DataDependency, each with
  a confidence field.
- **Graph.** `graph.rs`: `StableDiGraph<CodeNode, Edge>` plus `id_index`,
  `name_index`, `file_index` hash maps and centrality maps.
- **Languages.** `crates/arbor-core/src/languages/` has c, cpp, csharp, dart, go,
  java, python, rust, typescript.
- **Incremental.** `docs/ARCHITECTURE.md`: a content hash per file and a two-tier
  AST/node cache, plus `index --changed-only`.
- **Memory.** Same document: "~1KB per node, so 100k nodes ≈ 100MB", an estimate.
  The whole graph is resident, which is what we must not do.
- **MCP.** Present; the tool list was not read: **unverified**.

Stable indices fit an interactive editor that mutates a live graph. rust-graph
does not hold a live graph, so the DiGraph shape does not transfer. The separate
`FlowsTo`/`DataDependency` kinds confirm that value flow belongs in the schema.

### 3. Codebase-Memory

`DeusData/codebase-memory-mcp`, 48,254 stars, active. arXiv:2603.27277.

- **Licence.** MIT. Vendoring would be legal, but it is C on tree-sitter, so
  there is nothing to take but design.
- **Languages.** README: 162 vendored tree-sitter grammars compiled in, plus a
  hybrid LSP resolver for 11 languages.
- **Schema.** README, "Edge types (selected)": `CALLS`, `CALL_REFERENCE`, `USAGE`,
  `IMPORTS`, `DEFINES`, `IMPLEMENTS`, `INHERITS`, `HTTP_CALLS`, `SPAWNS`,
  `CONFIGURES` and more, and `DATA_FLOWS` "with arg-to-param mapping + field
  access chains". That last edge is the closest thing in the MCP field to what
  taint needs. Keeping `CALLS` apart from `USAGE`, where a unique target is not
  proven, is the right discipline.
- **Incremental.** `src/pipeline/pipeline_incremental.c`: mtime+size classifies
  files, changed files' nodes are deleted with edges cascading, only those files
  are re-parsed into a temporary buffer, then merged into the disk DB.
  `src/watcher/watcher.c` polls git status and HEAD at 5s + 1s per 500 files,
  capped at 60s, and commits baselines only after a successful reindex.
- **Memory.** README: "RAM-first pipeline: LZ4 compression, in-memory SQLite,
  single dump at end", with `CBM_MEM_BUDGET_MB` and `index_max_rss_mb`.
  `src/graph_buffer/graph_buffer.c` heap-allocates each node and edge. So the
  full graph is resident during a build and capped by a budget, and is not
  streamed. It trades memory for speed the opposite way to our target.
- **MCP.** `src/mcp/mcp.c`: `index_repository`, `search_graph`, `query_graph`,
  `trace_path`, `get_code_snippet`, `get_file_outline`, `get_graph_schema`,
  `get_architecture`, `search_code`, `list_projects`, `delete_project`,
  `index_status`, `detect_changes`, `manage_adr`, `ingest_traces` and others
  (README: 17). `get_graph_schema` is a good idea: the agent asks what exists.
- **Pipeline.** Discover, then per-language passes (`pass_definitions.c`,
  `pass_calls.c`, ...), then store, then serve.

### 4. Joern

`joernio/joern`, 3,625 stars, active.

- **Licence.** Apache-2.0. Shapes only.
- **Languages.** README description: C/C++, Java, binary, JavaScript, Python,
  Kotlin. Each has its own frontend rather than a shared grammar table.
- **Schema and dataflow.** `passes/reachingdef/ReachingDefPass.scala` solves
  reaching definitions per method, in parallel, and adds data-dependence edges.
  `ReachingDefProblem.scala` is the classic transfer function,
  `gen(n) ∪ (x − kill(n))`. The comment "Killing an identifier should also kill
  field accesses on that identifier" is worth keeping. The pass bails out above
  `maxNumberOfDefinitions = 4000` per method rather than running unbounded.
  `DefaultSemantics.scala` gives call behaviour as data:
  `F(Operators.assignment, List((2, 1), (2, -1)))` means argument 2 flows to
  argument 1 and to the return value. `queryengine/Engine.scala` then solves
  source-to-sink tasks across calls on top of those per-method facts, keeping a
  cache of known paths (`ResultTable`).
- **Incremental.** Nothing was read that re-indexes one edited file: **unverified**.
- **Memory.** Built around an in-memory graph on the JVM; figures **unverified**.
- **MCP.** None in the files read.

The lesson shaped the split: per-function flow facts come first, and the
interprocedural solve is a separate layer over them. That is exactly the
boundary between rust-graph (facts) and its reader (rules and solver).

### 5. orgraph-mcp

`tss-pranavkumar/orgraph`, PyPI `orgraph-mcp` 0.1.34. 0 stars.

- **Licence.** MIT per PyPI and GitHub. It has an `orgraph/_vendor/` directory
  with `extract.py`, `cluster.py` and `cache.py`, whose origin was not traced:
  **unverified**, so nothing from it should be reused even under MIT.
- **Languages.** Tree-sitter, with `orgraph/extract/scip.py` for SCIP input. The
  count is **unverified**.
- **Schema.** `orgraph/graph/schema.py`, Kuzu DDL: Repository, File, Directory,
  Module, Function, Class. Function rows carry `confidence`, `community_id`,
  `cluster_id`, `http_method` and `http_path`.
- **Incremental.** The `reindex(force)` tool re-indexes changed and deleted files
  (README).
- **Memory.** **Unverified.**
- **MCP.** The most useful surface here for its size. `orgraph/mcp/tools.py`:
  `find_path` is a shortest path over CALLS with `max_hops` default 15, capped at
  20, and `trace` depth is capped at 5. Bounded queries that report "No path
  found within N hops" are honest in a way "not found" is not.

### 6. codegraphx

`ethandev147/codegraphx`, Go, MIT, 1 star. It matches the brief (17 languages,
`get_impact_radius`). `techcraze00/CodeGraphX` is an unrelated JavaScript project.

- **Licence.** MIT.
- **Languages.** 17, tree-sitter; `internal/config/language.go` configures a
  grammar per language in TOML.
- **Schema.** `internal/graph/model.go`: File, Class, Function, Type, Test nodes;
  CALLS, INHERITS, IMPLEMENTS, CONTAINS and others.
- **Incremental.** `internal/builder/update.go`, in order: changed files from git
  diff, dependents by BFS over importers to depth 2, skip anything whose content
  hash (SHA-256) is unchanged, re-parse the rest. On a git failure it falls back
  to a full build. `internal/incremental/watcher.go` uses fsnotify with a 200 ms
  debounce per path.
- **Memory.** `internal/graph/memory.go` keeps a resident graph beside the store.
  Figures **unverified**.
- **MCP.** Seven tools (README). `internal/analysis/impact.go`: `ImpactRadius` is a
  reverse BFS with default depth 2.

Re-parsing dependents is a design smell for us: if per-file facts are independent
of other files, a dependent never needs re-parsing, only re-resolving.

### 7. Ferrograph

`GarthDB/ferrograph`, MIT, crate 2.0.0, 4 stars, active.

- **Why Rust-only.** `Cargo.toml` depends on `tree-sitter-rust` and `ra_ap_ide`
  (rust-analyzer), and `src/pipeline/` has `borrows.rs`, `lifetimes.rs`, `owns.rs`,
  `traits.rs`. The value is Rust semantics, which do not generalise.
- **Schema.** `src/graph/schema.rs`: File, Module, Function, Struct, Enum, Trait,
  Impl and others. `README` lists Imports edges and call-graph construction.
- **Graph DB.** `src/graph/store.rs`: CozoDB, `mem` or `sqlite` backends, queried in
  Datalog (`?[id] := *dead_functions[id]`).
- **Incremental.** A `watch` command re-indexes; the granularity is **unverified**.
- **Memory.** **Unverified.**
- **MCP.** Server over stdio; tools not enumerated here.

Is the graph DB worth copying? No. Datalog is pleasant, but Cozo pulls in a large
dependency tree (the `Cargo.toml` comments pin rayon just to keep it compiling),
which rules it out for a std-only, under-2 MB binary. The reader needs one fixed
query (taint), not an ad hoc language.

## Graphify and its ports

`Graphify-Labs/graphify` (143,653 stars, Apache-2.0 per the GitHub API; one
third-party page says MIT, and the API is taken as correct). Shapes only.

- `graphify/build.py` imports networkx and builds the whole graph with
  `build_from_json`, so the entire graph is resident as Python objects. That
  matches the RAM blow-up in the brief. No measurement of it was found, so the
  cause is inferred from the code, not proven.
- `graphify/cache.py` is the best cache design read: entries are namespaced by
  package version, schema and grammar fingerprint, because keying purely on
  content "means extractor fixes shipped in a new release keep serving stale
  pre-fix results". We take that idea: the cache key includes the engine version.
- README: each edge is `EXTRACTED` or `INFERRED`; communities via Leiden; output
  is `graph.json`, `graph.html` and a report. About 40 languages.

Go implementations exist, seen as listings only, source **not read**:
`techinpark/graphify-go` (MIT, 18 languages), `sjhorn/graphify` (Louvain),
`dobbo-ca/graphify-go` (tree-sitter through cgo). Rust ports:
`Graphicle-AI/graphicle` (Apache-2.0, "Graphify, ported to Rust"), and
`crabbuild/compass` ("inspired by Graphify", licence NOASSERTION).

## The wider field

Found through GitHub search by topic and stars (25 queries, 128 repositories with
15+ stars), crates.io, PyPI, arXiv and web search. Unless a file is named, the
row rests on the README or the GitHub listing.

### Taken from

| Project | Licence | Why it matters |
|:--|:--|:--|
| `github/stack-graphs` (arXiv:2211.01224) | Apache-2.0 | Each file's subgraph is built with no knowledge of any other file, and resolution happens at query time. This is the key to incrementality. Its last push was 2025-09. Shape only. |
| `sdsrss/code-graph-mcp` | MIT | BLAKE3 Merkle tree over files: a no-change run takes ~28 ms by its own figure. Skipping unchanged subtrees is a good later step. |
| `optave/ops-codegraph-tool` | Apache-2.0 | Three-tier change detection: journal, then mtime+size, then hash. We use the last two. |
| `sourcegraph/scip`, `scip-code/scip` | Apache-2.0 | Language-agnostic symbol schema. Confirms symbol ids should be stable strings, not node indices. |
| `tirth8205/code-review-graph` | MIT | Measured cold build: ~40 s on ~3,000 files (its own figure). A useful reference for what a tree-sitter + Python stack costs. |

### Rejected

| Project | Licence | Reason |
|:--|:--|:--|
| `tree-sitter/tree-sitter` | MIT | Forbidden by the brief. Also a C runtime plus one grammar per language, which alone exceeds the size budget (`codebase-memory-mcp` vendors 162). |
| `semgrep/semgrep` | LGPL-2.1 | Copyleft engine, and its README places cross-file dataflow in the proprietary platform. |
| `Fraunhofer-AISEC/cpg` | Apache-2.0 | JVM library; a full CPG is far heavier than the facts we need. |
| `antgroup/YASA-Engine` | Apache-2.0 | Unified IR for taint, but TypeScript runtime; wrong shape for a small binary. |
| `pascal-lab/Tai-e`, `soot-oss/SootUp` | LGPL | Java bytecode only, copyleft. |
| `meta/pyre-check` (Pysa), `vimeo/psalm`, `find-sec-bugs` | MIT / MIT / LGPL | Single-language; wrong shape. |
| `python-security/pyt` | GPL-2.0 | Dead since 2020, copyleft. |
| `yinwang0/pysonar2` | Apache-2.0 | Python-only type inference on the JVM. |
| `Technologicat/pyan`, `koknat/callGraph` | GPL | Copyleft; `callGraph` is regex per language, which is close to our approach but with no value flow. |
| `kinlab-ai/kin` | Apache-2.0 | Archived by its authors. |
| `AhmadHassan-BTed/OpenHeart` | MIT | Succinct trees and mmap artifacts, with 10 compile phases. Claims **unverified**, and far more machinery than one-pass facts. |
| `repowise-dev/repowise`, `giancarloerra/SocratiCode`, `justrach/devswarm` | AGPL-3.0 | Licence. |
| `vitali87/code-graph-rag`, `FalkorDB/code-graph`, `bgts-ai-org/bgts-context-engine` | MIT | Need a graph database or heavy stack. |
| `Ataraxy-Labs/sem`, `Ataraxy-Labs/weave` | Apache-2.0 | Entity-level diffs and merges; a different problem. |
| `FreePeak/LeanKG`, `Muvon/octocode`, `CodeBendKit/codeseek`, `aeroxy/ast-bro`, `sunerpy/codegraph-rust`, `aouicher/graphmind`, `0ximu/mu`, `greysquirr3l/coraline`, `williamfzc/gossiphs` | various | Rust plus tree-sitter, mostly with SQLite or embeddings. All fail the no-crates rule. Nothing read beyond what the named tools already showed. |
| `techcraze00/CodeGraphX`, `Cranot/roam-code`, `nikolai-vysotskyi/trace-mcp`, `srclight/srclight`, `harshkedia177/axon` | various | MCP servers on Python or Node runtimes; same surface as those above with a runtime we cannot ship. |
| `aider-AI/aider` | Apache-2.0 | Repo map ranks symbols for prompts. No graph output. |
| `JonathanSalwan/Triton`, `bap`, `bincat`, `polytracker`, `MATE` | various | Binary or instrumented taint. Different layer. |
| `sourcegraph/scip-python`, `scip-typescript`, `scip-go`, `microsoft/lsif-node` | various | One compiler-grade indexer per language. Precise, but needs each toolchain. |

### Added after the engine started

The survey above missed projects a serious reader would expect. These rows were
added on 2026-11-07, **after** the engine was written, so they did not shape it,
and the first line of this document does not cover them. Each row rests on the
project's README and GitHub metadata, fetched read-only through the GitHub API,
plus the licence file where the API returned NOASSERTION. No source file was
read and nothing was run. "Not stated" means the README does not say. A cell
marked **unverified** is my general knowledge, not something read here.

Program-analysis engines:

| Project | Licence | Languages | Graph or schema | Incremental | Memory | MCP | Verdict |
|:--|:--|:--|:--|:--|:--|:--|:--|
| `github/codeql` | MIT for queries; the CLI and engine are separately licensed (README) | Not stated in README | Relational database queried in QL (**unverified**) | Not stated | Not stated | Not stated | Rejected: the engine is not open source. |
| `meta/infer` | MIT | Not stated (795-byte README) | Not stated | Not stated | Not stated | Not stated | Rejected: OCaml toolchain, and needs each language's compile step (**unverified**). |
| `facebook/mariana-trench` | MIT | Android and Java (**unverified**) | Not stated | Not stated | Not stated | Not stated | Rejected: single ecosystem. |
| `opengrep/opengrep` | LGPL-2.1 | 30+ (README) | Pattern engine; `--taint-intrafile` | `--incremental-output` is output streaming, not incremental analysis | Not stated | Not stated | Rejected: copyleft, OCaml, and its advertised taint is intrafile. |
| `Bearer/bearer` | Elastic License 2.0 (README) | Free CLI subset; Pro adds C#, Kotlin, Elixir, VB.Net, Rust, Swift | Not stated | Not stated | Not stated | Not stated | Rejected: licence. Its inter-file analysis is in the paid product. |
| `plast-lab/doop` | NOASSERTION in the API; licence not confirmed, **unverified** | Java | Datalog rules over Soufflé | Not stated | Not stated | Not stated | Rejected: Java only. |
| `souffle-lang/souffle` | UPL-1.0 | A Datalog engine, not a language front end | Datalog relations | Not stated | Not stated | Not stated | Rejected: needs a C++ compile step. The fixed-point idea is textbook, and the reader's solver already iterates. |
| `SVF-tools/SVF` | Own "SVF Release License" file; terms not read, **unverified** | LLVM IR (**unverified**) | Call graph, ICFG, class hierarchy, constraint graphs (README) | Not stated | Not stated | Not stated | Rejected: needs LLVM. |
| `secure-software-engineering/phasar` | MIT (licence file header) | LLVM IR (**unverified**) | IFDS/IDE solver, call graph, alias information (README) | Not stated | Not stated | Not stated | Rejected: needs LLVM. |

Indexers and incremental frameworks:

| Project | Licence | Languages | Graph or schema | Incremental | Memory | MCP | Verdict |
|:--|:--|:--|:--|:--|:--|:--|:--|
| `kythe/kythe` | Apache-2.0 | Not stated in README | Published Kythe schema | Not stated | Not stated | Not stated | Rejected: one compiler-backed indexer per language, as with SCIP. |
| `facebookincubator/Glean` | BSD (licence file; API says NOASSERTION) | Some indexers are not in the open source release (README) | Angle query language over stored facts | Not stated | Not stated | Not stated | Rejected: a server and a Haskell stack (**unverified**). |
| `rust-lang/rust-analyzer` | MIT or Apache-2.0 (README) | Rust | Language server | Yes, through salsa (**unverified**) | Not stated | No; LSP | Rejected: one language. |
| `salsa-rs/salsa` | Apache-2.0 per API; MIT option **unverified** | A framework | Memoised queries | Its purpose | Not stated | No | Rejected: a crate. Per-file facts keyed by content hash give us the part we need. |
| `tree-sitter/tree-sitter-graph` | Apache-2.0 | Any tree-sitter grammar | DSL that builds graphs from syntax trees | Not stated | Not stated | No | Rejected: tree-sitter is forbidden. Last push 2024-12. |
| `ast-grep/ast-grep` | MIT | Tree-sitter grammars (README) | Syntax patterns, no graph | Not stated | Not stated | Not stated | Rejected: tree-sitter, and no cross-file facts. |
| `sourcegraph/zoekt` | Apache-2.0 | Any; text search with no parsing (README) | Text index | Not stated | Not stated | Not stated | Rejected: a different problem. |
| `CoatiSoftware/Sourcetrail` | GPL-3.0 | C, C++, Java, Python (README) | SourcetrailDB | Not stated | Not stated | No | Rejected: archived, copyleft. |

Agent-facing code graphs:

| Project | Licence | Languages | Graph or schema | Incremental | Memory | MCP | Verdict |
|:--|:--|:--|:--|:--|:--|:--|:--|
| `colbymchenry/codegraph` | MIT | Long badge list in README; count not taken | Project index under `.codegraph/` | Not confirmed | Not stated | Yes | Rejected: TypeScript runtime. Closest to the agent use case. |
| `Graphicle-AI/graphicle` | Apache-2.0 | Tree-sitter (README) | Files, functions, classes, calls, imports; one SQLite file | Not confirmed | Publishes peak-memory charts against Graphify; figures not checked | Yes | Rejected: tree-sitter and SQLite. Its one-program shape matches ours. |
| `abhigyanpatwari/GitNexus` | PolyForm Noncommercial 1.0.0 (licence file) | Per-language inventories in its docs | Tree-sitter into LadybugDB (README) | `analyze --watch` | Not stated | Yes | Rejected: licence. Not to be read for shape either. |
| `oraios/serena` | Per component: MIT and GPL-3.0-or-later (licence file) | 40+ through language servers (README) | LSP symbols, no stored graph | Not stated | Not stated | Yes | Rejected: needs a language server per language. |
| `potpie-ai/potpie` | Apache-2.0 | Not stated | "Context graph" with a daemon | Not stated | Not stated | Not stated | Rejected: a service, not a binary. |
| `mufeedvh/code2prompt` | MIT | Any; packs text | No graph | No | Not stated | Yes | Rejected: a different problem. |

Single-language call graphs:

| Project | Licence | Languages | Graph or schema | Incremental | Memory | MCP | Verdict |
|:--|:--|:--|:--|:--|:--|:--|:--|
| `scottrogowski/code2flow` | MIT | Python, JavaScript, Ruby, PHP (README) | Call graph it calls a "pretty good estimate" | No | Not stated | No | Rejected: needs each language's parser. Its honesty about dynamic languages is the right tone for our limits section. |
| `vitsalis/PyCG` | Apache-2.0 | Python | Call graph, FASTEN output | No | Not stated | No | Rejected: archived. |
| `nuanced-dev/nuanced-py` | MIT | Python | Call graph for agents | Not stated | Not stated | Not stated | Rejected: archived. |
| `sverweij/dependency-cruiser`, `pahen/madge`, `webpro-nl/knip` | MIT, MIT, ISC | JavaScript family | Module dependency graphs | Not stated | Not stated | knip: yes (`@knip/mcp`) | Rejected: single ecosystem and imports only. |

Not checked: `Trail-Code`, `sdl-mcp` and `compass` beyond the listing named
above. The local v8 copy of Graphify the brief mentions does not exist on this machine, so the Graphify rows rest on the public repository.

None of these additions changes the table at the end of this document. The
statement under "Not found" still stands with one caveat: Graphicle publishes
peak-memory charts, which I did not check for a 10,000-file figure.

### Not found

No project in this survey meets the size, crate and memory limits together. The
closest in spirit is `koknat/callGraph`. No tool read publishes a peak-RSS figure
for a 10,000-file tree, so there is no external baseline for our target.

## Answers to the six questions, for rust-graph

- **Licence.** MIT repository, so nothing Apache-2.0 enters it. Everything here
  is a re-implementation of ideas, written from this document.
- **Language coverage.** A data table of language families: comment and string
  syntax, definition keywords, block style (brace, indent, `end`). A one-pass
  lexer per family replaces grammars. This is less precise than tree-sitter, and
  the README must say so.
- **Graph schema.** Symbols; call edges; import edges; each edge tagged EXTRACTED
  or INFERRED with the resolution strategy, following CodeLens. Per-function flow
  facts (assignments, returns, call arguments) stand in for Modifies/Reads.
  Unresolved calls are reported, never guessed, following Codebase-Memory.
- **Incremental updates.** Facts depend only on one file's bytes, following stack
  graphs. A SHA-256 hit copies the file's facts from the previous pack; resolution
  re-runs over facts and never re-parses a dependent, unlike codegraphx. The cache
  key includes the engine version, following Graphify.
- **Memory.** Nothing resident scales with edges. Phase 1 spools per-file facts to
  disk. Phase 2 holds a compact symbol index and streams edges out. Every tool
  read keeps the graph in RAM or in a database; this is the point of departure.
- **MCP interface.** Not in this engine's scope. The brief asks for a binary with
  a JSON output. The bounded-query design from orgraph is what a later server
  should copy.

## What we take and what we build

| Idea | From | Licence position | Our decision |
|:--|:--|:--|:--|
| Per-file facts independent of other files | stack-graphs paper | Idea from a paper; Apache-2.0 code not used | Build. Core of the cache. |
| Resolution cascade with a recorded strategy | codelens-engine | Apache-2.0: shape only | Build our own order and names. |
| EXTRACTED / INFERRED tags | Graphify, and the brief | Apache-2.0: concept only | Build. Required by the brief. |
| Proven call vs unproven usage | codebase-memory-mcp | MIT | Build. Ambiguous names stay unresolved. |
| Arg-to-param flow facts | codebase-memory-mcp `DATA_FLOWS`, Joern semantics | MIT; Apache-2.0 shape only | Build as per-function facts. |
| Reaching definitions, gen/kill | Joern; textbook | Textbook method | Build in the reader's solver, not in the engine. |
| Per-method definition cap | Joern | Shape only | Build a limit, and report it when hit. |
| Cache key includes the engine version | Graphify `cache.py` | Apache-2.0: idea only | Build. |
| mtime+size pre-check before hashing | Codebase-Memory, optave | Idea only | Skip. The brief says SHA-256 per file, and a size+mtime shortcut can miss edits. |
| Merkle tree of directories | code-graph-mcp | MIT | Skip for now. |
| petgraph StableDiGraph | Arbor | Crate forbidden | Skip. |
| CozoDB / Datalog | Ferrograph | Crate forbidden | Skip. |
| SQLite store | Most tools | Crate forbidden | Skip. A flat spool file instead. |
| Tree-sitter | Most tools | Forbidden by the brief | Skip. One-pass lexers. |
| Communities, graph.html | Graphify | Out of the brief's required outputs | Skip. |
| MCP server, bounded queries | orgraph, codegraphx | Out of scope here | Skip for now. |

## Sources

- [DeusData/codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp), [arXiv:2603.27277](https://arxiv.org/abs/2603.27277)
- [joernio/joern](https://github.com/joernio/joern)
- [mupozg823/codelens-mcp-plugin](https://github.com/mupozg823/codelens-mcp-plugin), [codelens-engine on docs.rs](https://docs.rs/crate/codelens-engine/1.9.35/source/README.md)
- [Anandb71/arbor](https://github.com/Anandb71/arbor), [Arbor-Vitae record](https://zenodo.org/records/20549779)
- [tss-pranavkumar/orgraph](https://github.com/tss-pranavkumar/orgraph), [orgraph-mcp on PyPI](https://pypi.org/project/orgraph-mcp/)
- [ethandev147/codegraphx](https://github.com/ethandev147/codegraphx)
- [GarthDB/ferrograph](https://github.com/GarthDB/ferrograph)
- [Graphify-Labs/graphify](https://github.com/Graphify-Labs/graphify), [techinpark/graphify-go](https://github.com/techinpark/graphify-go), [sjhorn/graphify](https://github.com/sjhorn/graphify), [dobbo-ca/graphify-go](https://pkg.go.dev/github.com/dobbo-ca/graphify-go)
- [github/stack-graphs](https://github.com/github/stack-graphs), [arXiv:2211.01224](https://arxiv.org/abs/2211.01224)
- [scip-code/scip](https://github.com/scip-code/scip)
- Aggregators used for discovery only: [optave/codegraph](https://awesome.ecosyste.ms/projects/github.com%2Foptave%2Fcodegraph), [sdsrss/code-graph-mcp](https://awesome.ecosyste.ms/projects/github.com%2Fsdsrss%2Fcode-graph-mcp), [LeanKG](https://docs.rs/crate/leankg/latest/source/leankg-bootstrap.md)
