# Prior Art Report: Project Ultimate Edition, Phase 1

Status: Phase 1 complete. No runtime code has been written. Everything below is
meant to be checked, so each claim carries its evidence grade.

| Grade | Meaning |
|:--|:--|
| **[M]** | Measured on this machine for this report (method in Appendix A) |
| **[S]** | Read in source. Tree-sitter refs are to `master` at `f342e62` (2026-11-07) |
| **[C]** | Cited from a third party and not independently verified |
| **[E]** | My estimate or judgement. Not a result |

## 1. Summary

**Recommendation: do not port Tree-sitter's GLR design.** Build a deterministic,
token-level LL/PEG parser with Pratt expressions, a bracket-matching pre-pass, and
a flat arena tree. Generate it from a first-party, std-only proc macro. Defer GLR
indefinitely.

Three measurements drive that:

1. **Speed [M].** On the same 3.55 MB of Rust, rust-analyzer's hand-written LL
   parser (lexing, parsing, flat tree) ran at **66.8 MB/s**. Tree-sitter ran at
   **11.6 MB/s**. That is one language, against hand-tuned code. A generated parser
   has not been shown to reach it, and that is the main open risk.
2. **Size [M/E].** Tree-sitter's generated C for 39 of our languages is **651.7 MB
   of source**. Six grammars I compiled yield 0.13-0.17 bytes of object per source
   byte, which implies **roughly 98 MB of grammar tables** [E] against a 10 MB
   budget. LR tables at this breadth do not fit.
3. **Failure containment [M].** The cascade bug reproduces at HEAD. Its cause is
   structural: recovery and lexing are entangled with parser state. A bracket-match
   pass over a fixed token stream removes that class of failure by construction.

**What will not be true, even if this all works:** Tree-sitter stays better at
incremental re-parsing, queries, editor integration and grammar ecosystem. "Faster
than Tree-sitter" here can only mean batch, single-threaded, full-file parsing on
stated bytes. I will not claim "fastest in existence" or "no weaknesses", because
no measurement can support either.

### Corrections to the brief

| Brief says | Finding |
|:--|:--|
| #4257 is a SIGSEGV on deep ambiguous input | **Wrong issue.** #4257 is a merged PR, "Fix cases where error recovery could infinite loop". The real stack-overflow reports are #5807 and #1939 (section 4) |
| #5910 is a 0.27.0 regression | **Incomplete.** It also affects **v0.26.13**, which backported the fix. v0.26.12 is clean [M]. Still open and reproducing at HEAD [M] |
| Tree-sitter overflows on deep nesting | **Not in parsing.** It parsed 1,000,000 nesting levels on an 8 MB stack with no crash [M]. The overflows were in tree *walkers* and `stack_node_add_link` |
| Tree-sitter uses a pool allocator | **Overstated.** Each heap subtree is a separate malloc; the "pool" is a 32-entry free list (`subtree.c:23`) [S]. There is no arena |
| Mesquite needed `regex` | **Only for queries.** `regex` serves `#match?` predicates and `streaming-iterator` the query API (`runtime/src/query_api.rs:127`) [S]. Its parser core does not need them |
| 42 languages | **43 reachable rows.** `rust-graph languages` lists 42 tags, but MATLAB (tag `M`) is reached by content detection from `.m` and is not listed [S] |
| Max 512 GLR forks | Not applicable under this design. For scale, Tree-sitter caps versions at 6 (+4 overflow), and that cap is itself an open bug (#6057) |
| Felicity's rust-graph backend | **Not found.** Zero references on `master` (`4c2ff7c`) or `audit-15` (`2b92928`) [M]. `felicity-run` was not inspected, per your constraint. See section 9 |

### A live bug in rust-graph, unrelated to this project

While benchmarking I found that **rust-graph takes 79 s on one 6,766-byte file**
[M]: rust-analyzer's `crates/hir-def/src/dyn_map.rs`, 1 of 200 sampled Rust files.
The other 199 together take 0.21 s. Peak memory stays around 9.5 MB, so it would
show up as a slow file and not a crash.

Cause, from gdb backtraces: `Resolver::method` (`src/resolve.rs:200-223`) walks base
classes with a depth cap (`MAX_FOLLOW = 8`) but no visited set. Cyclic bases with
branching cost about `branches^8`. A visited set should fix it.

**I did not change it**, because your constraints say the default build must keep
exactly its behaviour. I also don't know whether the sweep's deployed binary has
the same code, since I did not touch it. Worth checking for per-repo timeouts in
the holdout results.

## 2. OSINT landscape

GitHub searches ran through `gh`; the GitHub MCP token returned "Bad credentials".

| Project | What it is | Takeaway | Dependencies |
|:--|:--|:--|:--|
| tree-sitter (C) | Incremental GLR, 27k stars | The reference. Autopsied in section 3 | C runtime plus a generated parser per language |
| Mesquite | Safe Rust port of 0.25.10, created 2026-10, 0 stars | Self-reports 0.73x C parse time. Same algorithm, so this bounds what a port can gain [C] | `regex`, `streaming-iterator` |
| pest | PEG, 5.4k stars | Rejects left recursion in its validator (`meta/src/validator.rs:233`), and has a call limit. Copy both ideas | `ucd-trie`, others optional |
| rust-peg | PEG macro, 1.6k stars | Proof that PEG-in-a-macro is viable. Not read in depth | Not checked |
| LALRPOP | LR(1), lane-table construction | Emits table-driven or recursive-ascent code (`lr1/codegen/`). Good reference if LR is ever revisited | Several |
| rust-analyzer | Hand-written LL plus lossless tree | **The closest prior art to this design.** 8-byte events, `TokenSet`, forward parents | `rustc_lexer`, `rowan`, others |
| lelwel | Resilient LL(1) generator | Shows matklad's recovery can be generated, not just hand-written | Not checked |
| Rustemo, RustyLR | LR/GLR generators | Live projects; neither solves multi-language breadth | Not checked |
| semtree | Claims "faster than Tree-sitter" | Description only. Claim unverified, not evaluated | Not checked |

The search `pure rust parser generator` found no zero-dependency, multi-language,
production parser generator. I can only say my searches did not surface one.

**Literature used:** Tomita 1986 (graph-structured stack); Farshi's correction and
Scott & Johnstone's RNGLR (Tomita's original fails to terminate on hidden left
recursion); Wagner & Graham 1997 (incremental GLR, Tree-sitter's lineage); Ford 2004
(PEG); Warth et al. 2008 (packrat left recursion, super-linear in some cases);
Medeiros et al. 2014 (left-recursive PEG semantics); Medeiros & Mascarenhas 2018
(PEG recovery via labels; a 2019 follow-up reports *automatic* annotation achieving
acceptable recovery on 41-76% of files, which is a warning about fully automatic
PEG recovery); matklad 2023 (resilient LL).

## 3. Tree-sitter autopsy

### Memory layout [S]

- `Subtree` is an 8-byte union: either a pointer, or **inline leaf data in the
  pointer's own bits**, tagged by the low bit that alignment leaves free
  (`subtree.h`). Small tokens never touch the heap. This is its cleverest trick.
- `SubtreeHeapData` is **80 bytes** [M], with 32-bit atomic refcount. Children sit
  directly before the header.
- Measured whole-process cost is far above 80 bytes: **100-230 bytes per node** [M].
  A 12 MB file of 2M short Python statements peaked at **1.8 GB RSS, about 150x
  the input** [M]. This is its biggest exploitable weakness.
- Refcounts and ownership exist so that old and new trees can share subtrees.
  That is the price of incrementality, and batch scanning gets no benefit from it.

### Parse tables and lexing [S]

- Two table forms (`language.h:82`): dense `state * symbol_count + symbol` for large
  states, run-length groups scanned linearly for small states. A lookup is already
  an array load, so **generated `match` statements would not beat it** for dense
  states and would cost compile time and size. This answers the brief's jump-table
  question: the win is not available in that spot.
- **Lexing is driven by parser state.** Each state selects a lex mode, so tokens
  valid only in context are recognised correctly (`parser.c:510`). Recovery switches
  to the `ERROR_STATE` lex mode (`parser.c:606`). That is a real strength for
  context-sensitive syntax, and the mechanism behind #5910's shifted token boundaries.
- Input arrives through a read callback, decoded per character (`lexer.c:89-119`).
  "Zero-copy" holds for storage (nodes keep offsets), not for the scanning path.
- A keyword is lexed as an identifier, then re-lexed by a small keyword automaton
  (`parser.c:653`), which shrinks the main lex table.

### GLR stack [S]

- A graph-structured stack, with nodes pooled up to 50 (`stack.c:12`) and up to 8
  links per node (`MAX_LINK_COUNT`). A ninth link is **silently dropped**
  (`stack.c:249`).
- `stack_node_add_link` (`stack.c:200`) merges equivalent paths and **recurses**
  when predecessors are mergeable. #6038 confirms it is still recursive at HEAD.
- At most 6 versions, plus 4 overflow (`parser.c:77-78`). `ts_parser__condense_stack`
  (`parser.c:1814`) sorts by cost and discards the surplus with no diagnostic.

### Error recovery [S]

- **Costs** (`error_costs.h`): 500 per recovery, 110 per missing tree, 100 per
  skipped tree, 30 per skipped line, 1 per skipped byte.
- **`ts_parser__handle_error`** (`parser.c:1480`) runs
  `ts_parser__do_all_potential_reductions` (`:1136`), tries inserting each
  possible missing token, records a summary of up to 16 prior states, then recovers.
- **`ts_parser__recover`** (`:1291`) tries two strategies: pop back to a summarised
  state where the lookahead is valid, wrapping what was popped in `ERROR`; or wrap
  the lookahead in `ERROR` and skip it. Both can run, as competing versions.
- **`ts_parser__select_tree`** (`:842`) prefers lower error cost, then higher dynamic
  precedence, then a structural comparison.

This is a global cost optimisation over competing versions. It is elegant, but the
outcome depends on cost arithmetic. #5910 is a small accounting change to that
arithmetic moving the result from a confined error to a collapsed document.

### Incremental model [S]

Old-tree nodes are reused when the edit does not touch them and the parse state
allows it (`ts_parser__reuse_node`, `parser.c:753`), with `fragile_left/right`,
`lookahead_bytes` and external scanner state guarding validity. Balancing runs at the
end to flatten long repeats (`ts_parser__balance_subtree`).

Most of the per-node bookkeeping serves this. **Felicity scans whole files in batch,
so we drop it**, and I expect some of the speed gap comes from that [E].

## 4. Known failure modes

Reproduction status is at Tree-sitter `f342e62` unless stated.

| Class | Reference | Status | Evidence | Required defence |
|:--|:--|:--|:--|:--|
| Cascade collapse | #5910, open | **Reproduced [M]** | `{"a":{"b" "c"}}` gives `root=ERROR` at HEAD and v0.26.13, and `document > object > pair > object > ERROR` at v0.26.12 | Delimiter firewall (section 7) |
| Token misalignment after recovery | #5910 | **Reproduced [M]** | Recovery re-lexes inside strings: `(UNEXPECTED 'c')`, stray `string_content` | Tokens fixed before parsing |
| Preprocessor brace split | Not filed that I found | **Reproduced [M]** | `drm.h` (48 KB): `extern "C" {` and `}` in separate `#if` blocks make the **whole file one root `ERROR`**. Same on v0.26.12, so not #5910. Truncating the tail changes the result | Branch-aware bracket pairing |
| Macro confusion | tree-sitter-c #106, cpp #40 and #293 | **Reproduced [M]** | `openssl/cmp.h`: one `DECLARE_ASN1_DUP_FUNCTION(...)` line puts **38,727 of 48,041 bytes** in `ERROR` | Macro-invocation rule |
| Unclosed delimiters | None filed that I found | **Measured [M]** | 200,000 `(`: one flat `ERROR`, one node. Survives, but no structure | Accept. Report and fall back |
| Stack overflow, parser | #1939, 2022 | Cited [C] | Recursive `stack_node_add_link` on an ERB injection | No recursive merges exist here |
| Stack overflow, tree walkers | #5807, fixed by #6038 | Cited [C] | Real files hit 2-3k nesting, such as Linux `intel-ucode-defs.h` at 3,333 levels. Parsing alone passed my probes to 1M [M] | Every tree walk iterative. Depth tests |
| Recovery blow-up | #5967, closed | Cited [C] | 14-byte input: 16 s, 2 GB, stops only near 2^32 cost | Step budget per file |
| Infinite loop | #4257, merged | Cited [C] | | Progress assertion on every loop |
| Silent correct-branch drop | #6057, **open** | Cited [C] | Version cap discards the only viable branch | No version cap exists here |
| Empty-match extras | #6019, closed | Cited [C] | Unbounded memory on 2 bytes | Compile-time nullable-loop rejection |
| Uninterruptible post-pass | #6067, open | Cited [C] | Progress callback is not called during a long accept | Timeout covers every phase |

**Fuzzing practice [S].** Tree-sitter's `crates/cli/src/fuzz/` applies random edits
with allocation checks. rust-analyzer's `syntax/src/fuzz.rs` checks parser
invariants and reparse agreement. pest runs `cargo-fuzz`. The transferable idea is
**invariants, not crash-hunting**. Ours, in a std-only deterministic mutator:

1. Terminates within budget.
2. Lossless: the token texts concatenate to the input, byte for byte.
3. Node ranges nest and are monotonic.
4. A mutation inside a balanced group leaves sibling subtrees unchanged. This is the
   cascade test.

## 5. Measured baseline

Machine shared with the holdout sweep (load about 1-2, 8 cores), niced, single
thread, so treat differences under about 10% as noise. Tree-sitter was built at
`-O2` with no LTO, a fresh parser per file, parse time only, best of 5 per file.
**These samples are local, not the 5,000-repo corpus.** C is mostly system headers
and is biased toward macros.

| Language | Files | Bytes | TS time | TS MB/s | Files with no error | Bytes in `ERROR` | Peak RSS |
|:--|--:|--:|--:|--:|--:|--:|--:|
| Python (stdlib) | 200 | 3,423,925 | 303.9 ms | 11.3 | 200/200 | 0% | 9.2 MB |
| Rust (rust-analyzer) | 200 | 3,554,804 | 307.4 ms | 11.6 | 198/200 | 4 B | 13.5 MB |
| JavaScript (npm packages) | 200 | 2,280,767 | 364.6 ms | 6.3 | 200/200 | 0% | 47.3 MB |
| C (headers, some `.c`) | 189 | 1,407,625 | 176.6 ms | 8.0 | **121/189 (64%)** | **10.0%** | 7.6 MB |

**Same Rust bytes through other engines [M]:**

| Engine | Work done | Time | MB/s | vs Tree-sitter |
|:--|:--|--:|--:|--:|
| Tree-sitter | Lex, parse, tree | 307.4 ms | 11.6 | 1.0x |
| rust-analyzer parser | Lex 19.8 ms, parse plus flat tree 33.4 ms | 53.3 ms | **66.8** | **5.8x** |
| rust-graph | Entire pipeline, including resolve and JSON, 199 files | about 211 ms | 16.8 | Not comparable |

Caveats on the 5.8x. It is one language, hand-tuned over years, on a simple flat
tree I wrote for this bench, and it does not reproduce all of Tree-sitter's output
(no field names, no node API). rust-analyzer also needed `--ignore-rust-version`,
because it declares rustc 1.98 and this box has 1.97.1. It built and ran normally.

**Startup [M]:** Tree-sitter's first empty parse takes **0.03-0.05 ms**. There is
almost nothing to beat here. The lesson from Mesquite goes the other way: it decodes
tables on first use, at a median 19 ms and up to 619 ms [C]. We must keep everything
static.

**Stress probes, 8 MB stack, no crashes [M]:**

| Input | Size | Parse | Peak RSS | Result |
|:--|--:|--:|--:|:--|
| 1M nested parens (JS) | 2 MB | 1,043 ms | 442 MB | Depth 1,000,002, clean |
| 1M nested arrays (JSON) | 2 MB | 705 ms | 301 MB | Depth 1,000,001, clean |
| `1+1+...`, 2M terms (JS) | 4 MB | 2,285 ms | 913 MB | Depth 2,000,003, clean |
| 2M statements (Python) | 12 MB | 6,421 ms | 1.8 GB | Clean |
| 200K unclosed `(` (JS) | 200 KB | 77 ms | 51 MB | One `ERROR` node |

**A correction to my own process.** My first run produced Tree-sitter "crashes" and
much worse error rates. All of it was wrong: the crashes were a use-after-free in
my driver, the Rust sample included deliberately invalid `test_data/`, and the C
sample included C++ headers. Fixing the samples moved Rust from 165/200 to 198/200.
That matters for Phase 7, because **a raw corpus contains the same contamination**
(section 8).

## 6. Architecture decision: PEG/LL or GLR

| Criterion | GLR with LR tables | LL/PEG on tokens |
|:--|:--|:--|
| Binary size at 43 languages | About 98 MB of tables [E] | Likely within budget, unproven (below) |
| Speed evidence | 11.6 MB/s, 0.73x with a port [C] | 66.8 MB/s hand-written [M] |
| Ambiguity | Handled natively | **Hidden by ordered choice. Real weakness** |
| Context-sensitive lexing | Excellent, parser-driven | **Weaker. Needs lexer heuristics** |
| Cascade containment | Cost arithmetic | Structural |
| Termination and memory | Needs caps, which caused #6057 and #5967 | One budget |
| Grammar authoring | Conflict resolution, then years of tuning | Direct, but shadowing is silent |

**Choose LL/PEG.** The binary-size row alone rules out tables at this breadth.

**What this gives up, stated plainly:**

- `a * b;` in C, `<T>(x)` in TypeScript and C++'s vexing parse are real ambiguities.
  We pick a reading with bounded speculation and flag the node `AMBIG`. GLR would
  keep both. Felicity should treat `AMBIG` as uncertain.
- PEG's `a / ab` silently never matches `ab`. A compile-time lint catches literal
  prefix shadowing, but not every semantic case. Differential testing has to find
  the rest.
- Some context-sensitive tokens (JS regex against division, JSX, Ruby's heuristics)
  will be misread sometimes, because lexing is not parser-driven. Section 7's lexer
  modes reduce this; they will not eliminate it.

## 7. Proposed architecture

```text
&[u8] -> [1 Lexer] -> [2 Delimiter pass] -> [3 Generated parser] -> [4 Flat tree]
          SoA tokens    pair[] table          events, budgets         16 B nodes
```

**1. Lexer.** Slices over `&[u8]`, no per-token allocation. Struct-of-arrays:
`kind: Vec<u16>`, `end: Vec<u32>`. Per-language modes cover string interpolation,
heredocs, regex-against-division by previous token, and layout (`INDENT`/`DEDENT`
for Python and Nim, offside for Haskell, F# and Elm). Tokens are final before
parsing starts, so recovery can never move a token boundary.

**2. Delimiter pass, the central idea.** One linear pass builds `pair: Vec<u32>`,
mapping each opener to its closer. It gives three things:

- **A firewall.** A `{...}` body parses with its end bound to `pair[open]`, so an
  error inside cannot escape, whatever the grammar does. This is the #5910 fix.
- **O(1) skipping.** Exceeded depth, a failed speculation or an unknown construct
  jumps the whole balanced group, kept as a flagged node.
- **Preprocessor awareness.** `#if` snapshots the bracket stack, `#else` restores
  it, `#endif` continues. This handles the `drm.h` idiom.

Limit: keyword-delimited languages (Ruby, Lua, Pascal, COBOL) need pairing on block
keywords, which is less reliable because `end`-style matching needs more context
than brackets. Their firewall will be weaker. Genuinely unbalanced input falls back
to indentation heuristics and is reported.

**3. Generated parser.** Grammar rules become Rust functions that `match` on
`kind`.

- **Expressions:** Pratt loops, so `1+1+...` iterates and does not recurse.
- **Left recursion:** rejected at compile time, direct and indirect, like pest,
  apart from declared operators.
- **Backtracking:** a mark and truncation of the event buffer, O(1). No packrat
  memo by default; opt-in per rule.
- **Budgets, one per file:** steps at most `K * tokens`, recursion depth, wall clock.
  Exceeding any one degrades the file and never panics. (rust-analyzer panics at
  its step limit, which is wrong for a batch scanner.)
- **Recovery:** the firewall first, then compile-time FIRST/FOLLOW sets including
  ancestors (matklad's rule), then restart at statement level. Every loop must
  consume a token or exit.
- **C family:** `#` lines become directive trivia. A top-level `NAME(...)` with no
  `;` is a `macro_invocation`. That is the `cmp.h` fix, and it is safe because
  pairing bounds it. If a file stays too broken, it is logged as unsupported and
  falls back to the lexer.

**4. Tree.** One preorder `Vec<Node>`, indexed by `u32`, with no pointers.
`Node { kind: u16, flags: u16, start: u32, end: u32, subtree_end: u32 }` is 16 bytes.
`subtree_end` makes "skip this subtree" one add, so all walks are iterative loops
and #5807 cannot occur. Arenas are reused across files, so steady state allocates
nothing. I expect 5x or better memory reduction against Tree-sitter's 100-230 B
per node, once tokens and events are counted [E].

**Answers to the brief's questions:**

- *Arena without `bumpalo`:* yes, this is it. `Vec` plus `u32` indices, reset per file.
- *Rust ownership for incremental parsing:* it gives safety, not speed. Sharing
  subtrees across versions needs reference counting in any language. I recommend
  **no incremental parsing**. If it is ever needed, rust-analyzer's approach (reparse
  the nearest `{}` block, `reparsing.rs`) fits directly on the `pair` table.
- *File size:* `u32` offsets mean a 4 GiB per-file ceiling. No buffer limit below that.

**Grammar DSL.** `macro_rules!` is the wrong tool: splitting rules needs a tt-muncher
with poor compile-time scaling, and it cannot compute FIRST sets or check left
recursion. Use a **first-party proc-macro crate on the std `proc_macro` API only**,
no `syn` or `quote`. Zero third-party crates. Your example DSL works unchanged:

```rust
grammar! { Python {
    func_def: "def" IDENT "(" [params] ")" ":" block,
    call: IDENT "(" [args] ")",
} }
```

A proc macro must live in its own crate, so this needs a Cargo workspace and
`parser = ["dep:..."]` rather than the literal `parser = []`.

**Naming.** `src/parser.rs` already exists (the one-pass walker), so the new code
needs another name. I suggest `deep`.

**Binary size, honestly.** The full Rust lexer plus parser costs about **295 KB**
[M] (616 KB stripped bench binary less a 320 KB std baseline, so this includes some
bench code and `tracing`). That is against Tree-sitter's 1.11 MB for the same
language. But **43 x 295 KB = 12.7 MB, over budget**, if every language were that
large. Most are simpler, so I expect to fit [E], and it must be measured from the
first grammars. If it runs over, the fallback is compiling the same IR to compact
bytecode for lower-tier languages, at some speed cost.

## 8. Targets and gates: what I propose changing

Several targets cannot currently be passed or failed meaningfully.

| Brief target | Problem | Proposed |
|:--|:--|:--|
| 1,000 files under 500 ms | **Depends entirely on file size.** Tree-sitter would pass on 2.4 KB files (about 210 ms) and fail at 17 KB files (about 1.5 s) [E, from section 5 rates] | Same bytes as Tree-sitter, single thread, reported in MB/s. **Gate: 2x or better**; stretch 4x |
| Under 50 MB for 10,000 files | RSS follows the **largest file**, not file count. Tree-sitter already peaked at 47 MB here | Peak RSS at the 2 MB file cap, against Tree-sitter on the same input. Target under 50 MB, and lower than TS |
| Faster startup | Tree-sitter's is 0.03-0.05 ms | No regression: at most 0.05 ms, no decode step |
| Under 10 MB binary | Reasonable, unproven | Keep. Track per grammar |
| >95% parse success | **Undefined, and gameable.** A grammar accepting anything scores 100% | Definition below |

**Proposed definition of parse success, fixed before any corpus run:**

| Metric | Meaning | Bar |
|:--|:--|:--|
| M1 Completion | Finishes in budget with no panic | 100% of files |
| M2 Clean files | Zero `ERROR` nodes, the headline | >95%, Wilson 95% lower bound reported |
| M3 Byte coverage | Bytes outside `ERROR` | Reported with M2 |
| M4 Fidelity | Function and call spans agree with Tree-sitter, where a grammar exists | Reported per language, with a bar set after the first slices |
| M5 Rejection | Mutated invalid files actually yield errors | Must pass, so permissive grammars fail |

Consequences to expect, stated now so they are not surprises later:

- **Tree-sitter itself scored 64% M2 on my C sample** [M], which is header-heavy. A
  95% M2 bar for C and C++ may be unreachable on corpora like that. Phase 7's lexer
  fallback may be the honest result there. M3 will be the more informative number.
- **Sample size decides what can be claimed.** At 96% observed, 100 files gives a
  lower bound near 90%. Showing 95% takes roughly 1,600 files. Long-tail languages
  (COBOL, Elm, Tcl, AWK) may not have that many, and I have not counted, because
  counting needs the corpus. For those I will report the number, the denominator
  and the bound, and not claim a pass.
- **Corpus hygiene must be decided in advance.** My sample showed deliberately
  invalid fixtures and wrong-language files moving Rust from 99% to 82.5%. Excluding
  files after seeing failures would be cherry-picking, so exclusion rules
  (test fixtures, vendored, generated, minified, mis-detected) go in writing first.

**Scope of Phase 7 [E].** Tree-sitter grammar sources run 1,200-1,700 lines for
Python, JavaScript, C and Rust, after years of tuning, and rust-analyzer's parser is
about 6,400 lines. I expect three tiers of quality, not 43 equal grammars: full for
languages Felicity needs for taint, solid for the mid-tier, structural (definitions,
calls, imports) for the long tail. Every language gets an AST. They will not be
equally deep, and the report will show which is which. Visual Basic has only small
community grammars for comparison; Perl, Swift and SQL do not commit `parser.c`, so
they are excluded from the 651.7 MB figure.

## 9. Decisions needed from you

1. **Felicity integration (Phase 5).** I found no rust-graph integration on `master`
   or `audit-15`; Felicity has its own `rust/felicity_graph`. Where does the hook
   live? Also, "do not commit to Felicity" conflicts with the acceptance item
   "integrated into Felicity". **I suggest** delivering a versioned JSON contract
   plus a patch file for you to apply.
2. **Resolved call graph (Phase 5).** A parser cannot resolve calls. Dynamic dispatch
   and cross-file types stay unresolved, however good the tree is. A better tree
   improves `resolve.rs`, but "complete resolved call graph" is more than this will
   deliver. **I suggest** promising better resolution, with `unresolved` kept honest.
3. **Targets and metrics (section 8).** Please approve or amend, ideally before Slice 3.
4. **Language scope:** 42, or 43 with MATLAB? **I suggest** 43, since it is reachable.
5. **Build shape.** Workspace plus `dep:` feature changes `Cargo.toml` and
   `Cargo.lock`. Does "exactly as it is" cover those, or just behaviour, speed and
   size? **I suggest** the latter, verified by hashing the default binary, which is
   the first thing Slice 0 tests. The "separate binary" criterion would be a
   `[[bin]]` with `required-features`.
6. **Fallback trigger (Phase 4).** The lexer does not report syntax errors today, and
   `unresolved` only exists after resolve, once every file is lexed. Fallback on
   unresolved calls therefore needs a second pass, and `facts.pack` keys must include
   the parser version. This is a design change, not a flag.
7. **The `resolve.rs` stall.** Should I fix it on a separate branch? It changes
   default behaviour for the better, so it needs your go-ahead.

## 10. Build plan

Each slice is demonstrable alone and has a stop condition.

| Slice | Deliverable | Gate |
|:--|:--|:--|
| 0 | Workspace and feature skeleton | Default binary **hash unchanged**, or we rethink packaging |
| 1 | Runtime: lexer core, delimiter pass, events, arena, budgets, JSON by hand | All #5910 fixtures keep the root; 1M-depth input survives; fuzz invariants hold |
| 2 | `grammar!` macro; the JSON grammar through it | Same tree as Slice 1; speed loss recorded |
| 3 | Python end to end, with validation harness | **At least 2x Tree-sitter on the same bytes. If not, stop and rethink before breadth.** Bounded 200-file sample |
| 4 | Integration: config, modes, fallback, logging | `enabled = false` is byte-identical output |
| 5 | Breadth, in tier order | Binary size tracked per grammar |
| 6 | Full corpus run, after the sweep | Section 8 metrics, with denominators |

Python goes first: it has the hard layout problem, real taint value, and a measured
baseline. Slice 3 is where this design is actually proven or disproven. Everything
before it is engineering, and nothing in this report demonstrates that a *generated*
parser reaches a useful share of hand-written speed.

## Appendix A: Reproducing the measurements

Sources were shallow-cloned to `/tmp/ug-osint`. The harness is in
`target/ts-baseline/` (git-ignored, so not preserved by a clean; Phase 8 will
commit a proper version).

- `driver.c` links `lib/src/lib.c` with each grammar's `parser.c` and `scanner.c`
  (`gcc -O2`). Modes: `bench`, `probe`, `sexp`, `errs`. Versions: `f342e62`, `v0.26.12`
  and `v0.26.13`. C numbers matched across all three within noise: 121/189 clean
  everywhere, 10.04-10.06% error bytes.
- `ra-bench` calls rust-analyzer's `parser` crate. It builds a `Vec<(u16,u32,u32)>`
  tree from the output steps, inside the timed region. 200/200 clean.
- Samples: every Nth file, sorted, up to 200, under 2 MB. `parser.c` excluded.
  - Python: `/usr/lib/python3.14`, 555 candidates.
  - Rust: `rust-analyzer/crates`, excluding `test_data`, `tests`, `fixtures`, 838 candidates.
  - JavaScript: `~/.npm-global/lib/node_modules`, excluding `dist` and `.min`, 2,144 candidates.
  - C: `/usr/include` plus Tree-sitter `lib/src`, excluding C++, Node and similar trees.
- rust-graph: `taskset -c 7`, three runs, default release build, worktree HEAD `07ae7c3`.
- Grammar census: GitHub tree API, one grammar picked per language. Object ratios
  from `size` on `g_*.o`: python 0.134, javascript 0.143, c 0.159, rust 0.171,
  go 0.137, java 0.156. Applying them to much larger grammars is an extrapolation.
- Default `cargo build --release` here: 762,272 bytes (glibc). The README's 840,368
  is the static musl build, which I did not rebuild.

Not done: no Mesquite run (its numbers are cited), no C++ or other grammars beyond
the seven built, no #5967 or #6057 reproduction (both need other grammars or a
generator), no corpus access, and no inspection of `felicity-run`.

## Appendix B: Sources

**Tree-sitter issues and PRs:**
[#5910](https://github.com/tree-sitter/tree-sitter/issues/5910),
[#4257](https://github.com/tree-sitter/tree-sitter/pull/4257),
[#5807](https://github.com/tree-sitter/tree-sitter/issues/5807),
[#6038](https://github.com/tree-sitter/tree-sitter/pull/6038),
[#1939](https://github.com/tree-sitter/tree-sitter/issues/1939),
[#5967](https://github.com/tree-sitter/tree-sitter/issues/5967),
[#6057](https://github.com/tree-sitter/tree-sitter/issues/6057),
[#6019](https://github.com/tree-sitter/tree-sitter/issues/6019),
[#6067](https://github.com/tree-sitter/tree-sitter/issues/6067),
[commit 869638f6](https://github.com/tree-sitter/tree-sitter/commit/869638f6cf4af95170be8813c774efb8f01424b5).

**Macro confusion:**
[tree-sitter-cpp #40](https://github.com/tree-sitter/tree-sitter-cpp/issues/40),
[tree-sitter-cpp #293](https://github.com/tree-sitter/tree-sitter-cpp/issues/293),
[tree-sitter-c #106](https://github.com/tree-sitter/tree-sitter-c/issues/106),
[Doxide on parsing](https://doxide.indii.org/parsing/).

**Projects:**
[tree-sitter](https://github.com/tree-sitter/tree-sitter),
[Mesquite](https://github.com/mesa-hills-research/mesquite),
[pest](https://github.com/pest-parser/pest),
[rust-peg](https://github.com/kevinmehall/rust-peg),
[LALRPOP](https://github.com/lalrpop/lalrpop),
[rust-analyzer](https://github.com/rust-lang/rust-analyzer),
[lelwel](https://docs.rs/crate/lelwel/latest),
[Rustemo](https://github.com/igordejanovic/rustemo),
[RustyLR](https://github.com/ehwan/RustyLR),
[semtree](https://github.com/Fanaperana/semtree),
[tree-sitter-rust](https://github.com/tree-sitter/tree-sitter-rust/wiki).

**Papers and articles:**
[Wagner & Graham 1997](https://bibtex.github.io/PLDI-1997-WagnerG.html),
[Wagner dissertation](https://www2.eecs.berkeley.edu/Pubs/TechRpts/1997/5885.html),
[Warth et al. 2008](https://web.cs.ucla.edu/~todd/research/pub.php?id=pepm08),
[Medeiros et al., left recursion](https://arxiv.org/abs/1207.0443),
[Medeiros & Mascarenhas, recovery](https://arxiv.org/pdf/1806.11150.pdf),
[Automatic recovery annotation](https://arxiv.org/abs/1905.02145v1),
[matklad, resilient LL](https://matklad.github.io/2023/05/21/resilient-ll-parsing-tutorial.html),
[GLR overview](https://www.wikipedia.com/wiki/GLR_parser),
[Superion](https://arxiv.org/pdf/1812.01197),
[Tree-sitter fuzzing](https://github.com/tree-sitter/tree-sitter/blob/4212ea53c46842398554a901e959a322903a4e4b/test/fuzz/README.md),
[Brunsfeld, Strange Loop 2018](https://thestrangeloop.com/2018/tree-sitter---a-new-parsing-system-for-programming-tools.html).
