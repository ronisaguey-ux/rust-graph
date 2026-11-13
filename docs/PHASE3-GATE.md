# Phase 3 exit gate

The phase that made the deep reader a second opinion rather than a replacement.
This is the checklist it exits on, the evidence for each line, and the one place
the plan's wording was corrected.

Measured 2026-11-13. Re-run the commands below; the numbers in `docs/CST.md` and
`docs/MEASURE.md` are the same measurements, kept there where they are used.

## The two readers are independent, not sharing

The plan's line says "two readers share the arena and trait". They do not, and
that is the stronger property: the one-pass lexer (`lexer`, `expr`, `parser`,
`facts`, `langs`) and the CST reader (`deep/*`) are separate types with separate
node models, and the guard is a function of two `Facts` records, so it can name
neither. A shared arena would make a fallback comparison meaningless — each side
would be scoring the other's work.

Evidence: `tests/layering.rs` walks `src` at test time and fails if any lexer-side
module names `deep`/`deep_integration`/`deep_facts` or any deep module names a
lexer module, with a companion test that proves the detector fires on a real
import and stays quiet on a comment. `guard_truth::the_guard_decides_from_two_records_and_no_grammar`
drives the guard with hand-built records and no parser in sight.

## Budgets are met

| promise | budget | measured |
| --- | --- | --- |
| parse time, p95 per file per language | < 2 ms | js 0.624, ts 0.274, python 1.896, rust 1.063 ms |
| memory, 2,000-file corpus run | < 300 MB peak RSS | js 129 MB (worst case), ts 17.9, python 14.4, rust 45.2 MB |
| allocations per corpus run | steady state | js 9, ts 7, python 5, rust 11 for 566–2000 files |
| nodes per token | < 2 | 0.96 js, 1.36 ts, 0.34 python, 1.14 rust |

Commands: `rust-graph-deep-scan conformance DIR --reps 3`, `bench LIST --reps 1`
and `probe FILE`; details and the per-file tables live in `docs/CST.md`.

## The fuzz pass is clean and no panic path exists

`tests/fuzz_recovery.rs` mutates five sources a thousand ways (byte flips,
truncations, unbalanced insertions, deletions, slice repetitions) from a fixed
seed and requires a tree or a clean refusal for every input; a panic fails the
test by construction. `Cst::degraded()` bounds recovery at one error node per
hundred so a guessed tree is refused instead of published.

## The guard is still green at the end of the phase

- `cargo test --workspace --features "deep parser"` — exit 0.
- `cargo test` (default build) — exit 0.
- Fallbacks: js 581/1995, ts 302/2000, python 145/568, rust 343/1816; losses
  js 1141, ts 638, python 196, rust 399 (`tools/loss_count.py`).

## What this gate does not claim

The phase compared two readers and bounded their disagreements; it did not make
either one complete. The losses above are the remaining disagreement, and the
queue in `~/.local/state/opencode/perfection-queue.md` is where they are worked
off, one cluster at a time, judged by `tools/loss_shapes.py` and
`tools/loss_count.py` rather than by a file count that only moves on the last
loss of a file.
