# Unsolved — the log of questions that outlived the work that asked them

This file is the record of a question the corpus asked and the code answered with
a measurement, not with an opinion. A question that is closed keeps its numbers;
a question still open says where the disagreement is counted
(`docs/RUST_CONSTRUCTS.md`) and which module owns it. `docs/PAIRING.md` is the
design behind the guard; this is the history of the shapes that shaped it.

Current state, 2026-11-15: rust 48 fallbacks of 1,816 files, js 36, ts 41,
python 8, pinned by `tools/phase4_gate.py` (see `docs/MEASURE.md`).

## Match arms: a block that belongs to the arm — **closed 2026-11-15**

**The question.** A Rust `match` arm has two readings: the arm's block is a
statement list of its own, or the whole arm is one expression that becomes its
own frame (a function-like node whose body holds the statements). The two readers
disagreed because the CST wrote one and the lexer the other, and nothing said
which was right.

**The reproduction.** `/tmp/r034/000035_line_boundary.rs`, reduced to four lines:
the CST wrote `m c` and `LPAREN` against a lexer row carrying only the call on
the next line. One arm, one block, two readers, one line apart.

**The reading that lost, with its number.** Arms as their own functions: rust
345 → **417** fallbacks. `tools/baselines/arm_merge.json` is the record of the
attempt that followed, and of why it was reverted.

**The reading that won.** The arm body merges its statements on the arm's first
line, exactly as the lexer writes them. The change lives in statement
finalisation and is reached only for Rust's `=>`, so it cannot move the other
three corpora — which is the shape of a fix that is attributable to one change
rather than a bundle.

**Measured, on the full corpus, before it was believed** (`tools/phase4_gate.py
--json`, commit `8412e5a`): rust **308** fallbacks of 1,816 files, js 136, ts 91,
python 14. The arm change was 37 under the 345 the plan allowed at the time.

**What remains of it.** The two readers still stamp different lines for a match
that is an assignment's value versus one that ends a block — the CST folds the
arm block into the `match_expression` value and the lexer only sees the head. The
fold is load-bearing there and dropping it cost seven rust files, so it stays and
the disagreement is documented where it can be read: `docs/PAIRING.md`, "Match
arms: one expression, and the fold that is load-bearing".

**Landed alone, on purpose.** The change is one commit touching one file
(`aa4c019`, `src/deep_facts/rust.rs`, 8 lines added and 2 removed), so a
regression after it is attributable to it and not to a bundle it travelled in.
That is checked rather than promised: `python3 tools/commit_audit.py --check`
reads the history and refuses a pinned merge fix that touched a test, a baseline
or a document, that grew past 40 lines, or that was measured by a commit without
it in history (`tools/baselines/merge_fix.json` holds the pin).

**Closed by** the statement-finalisation change and five coverage rows in
`tests/rust_coverage.rs` ("match arms: block body, guard, call value and
wildcard", plus the guard, or-pattern, binding-at and range-pattern rows from
P5-10).

## Still open

The codes, counts and owning modules are regenerated into
`docs/RUST_CONSTRUCTS.md` and `docs/READERS.md` from
`tools/baselines/constructs.json`; this file does not repeat them, so it cannot
go stale beside them. The open questions behind those codes are, in the order
they cost the most rust files:

1. **The lexer invents a name the CST never writes** (`LEX_OVER_EMIT:name`): the
   value reader's merges. The fix belongs in `src/expr.rs`, where both readers'
   value shapes are decided for every language.
2. **The lexer invents a callee** (`LEX_OVER_EMIT:call`): call detection in the
   same reader; a pattern, a type or a range read as a call.
3. **The CST has no node for a call the lexer read**
   (`CST_MISSING_KIND:call`): a grammar gap, so `src/deep/` and the grammar
   tables, not the guard.
4. **Two events on one line that cannot be paired** (`PAIR_AMBIGUOUS:1`): the
   classifier decides the file; the readers have to agree first.
5. **Grammar recovery over budget** (`CST_DEGRADED:recovery`): error nodes are
   the CVD parser's recovery, not a reader disagreement, and the threshold is
   the only knob that is honest here.
