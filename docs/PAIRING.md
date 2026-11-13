# Pairing: how a lexer event is matched to a CST event

The guard does not ask "did the file parse". It asks a smaller question, once per
lexer event: is there a CST event in this symbol that carries the same fact? The
answer decides whether a file falls back. This is the order the question is asked
in, and why the order is the whole design.

Everything here lives in `uncovered()` in `src/pipeline.rs` — the tiers are the
three loops in it, in the order they run.

## Tier 1 — exact

Same marker (`a`, `r`, `x`), same line, same target list, same value.

This tier runs over every wanted row before any other tier is considered. That
ordering is not tidiness: pairing greedily, one want at a time, let a resembling
event on the next line consume the row a later want needed, and the file was then
blamed for an event that was in the record all along. Making tier 1 global-first
removed that whole class.

## Tier 2 — bounded structural, same line

Same marker, same line, target list identical, value covered.

"Covered" means every name and keyword atom the lexer wrote is present in the CST
value (`covers_value`). A path and its prefix count as the same name — the lexer
writes `rw` where the CST wrote `rw.Header` — but that tolerance runs *after* the
exact comparison, never before it: the bare name `Class` would otherwise consume
`Class.Node` and leave the real event with nothing.

Two conditions bound this tier:

- **shape distance at most one**: the value may differ by one atom, which is what
  a receiver chain or a merged operator looks like. It is not a fuzzy match.
- **no reordering**: a row is removed from the pool when it is used
  (`swap_remove`), so no row pairs twice, and the wants are consumed in the order
  the lexer wrote them.

## Tier 3 — plus or minus one line

Same marker, target list identical, value covered, and the line differs by at
most one. A statement the lexer reads as a bare expression is the CST's
assignment on that same line or the next (`for (var name in all)` against the
CST's `a … name = all`), so this tier exists for the shapes where the two
readers stamp a different line for one statement: a signature stamped where it
ends against where it starts, a header opened on one line and closed on the next.

Two hard rules hold it in place:

- **one line is the ceiling**, never "nearest line". A duplicate target three
  lines down must never stand in for the event that lost its value — that is the
  difference between a measurement and a guess.
- **only where the lexer under-emits**: an `x` want with no targets may pair with
  an `a` row on that line, because the lexer read a binding as a bare statement.
  The reverse never happens: a CST assignment is never allowed to consume an
  event the lexer wrote as a real assignment.

## After the tiers

What is left is either a real loss (reported with the nearest same-marker row as
its candidate, so the reason names the closest thing the CST wrote) or one of the
named artefacts that are set aside with a counter: an invented binder from an
annotation, a type read as a target, a keyword statement. Those are counted, not
silently swallowed, and each has its own counter in `Accepted`.

## What this buys

A fallback now means "the lexer wrote a fact the CST does not have, and no row
within one line and one atom covers it". That is a statement worth acting on. The
alternative — a single fuzzy pass — produces a number that moves when anything
moves and cannot be argued with.

## Measured: what the forgiving tiers buy

`strict-pair` keeps only tier 1. Built once beside the default binary and run on
the same fifty-file subsets in the same session:

| subset | default | strict-pair |
| --- | ---: | ---: |
| js | 16 / 50 | 32 / 50 |
| ts | 10 / 50 | 10 / 50 |
| python | 18 / 50 | 46 / 50 |
| rust | 4 / 50 | 46 / 50 |

Read it per language, not in total. On python and rust nearly every pairing is a
tier-2 or tier-3 pairing, so those corpora are the ones the tolerance is serving:
four rust files fall back today and forty-six would without it. On TypeScript the
flag changes nothing at all, which says its ten fallbacks are structural and no
amount of pairing tolerance reaches them. JS sits in between.

That is also the warning: a change that tightens the tiers has to be measured on
python and rust first, because those are where the cost lands.
