# Targets — approved by the owner, 2026-11-25

These are the official acceptance gates for every round. A language is not "done"
because its file percentage looks small: the corpus median file is 94 lines, so a
fallback on a 40-line file and a fallback on an 1,800-line file count the same in the
file percentage and nothing like the same to a reader.

## Gates

| gate | value | why |
|---|---|---|
| **LOC-weighted fallback per language** | **≤ 2%** | the real measure of how much code a CST reader cannot record |
| file-weighted fallback per language | ≤ 5% | kept as a floor, not the goal — it flatters an empty file |
| corpus-wide LOC lost | ≤ 0.5% | stops a few huge languages hiding behind the small ones |
| largest single FRCE class | ≤ 20% of remaining losses | keeps the work targeted instead of sprawling |
| FRCE unclassified | < 3% | a fallback whose reason nobody can name is not a lead |

Every attempt reports both weightings, on both corpora (normal + 700+ LOC), before
and after. A change that improves one weighting and worsens the other is a revert.

## FRCE must keep improving, every round

FRCE is not a ribbon on the report; it is the thing that makes velocity possible.
Knowing *why* a fallback happened is what turns "php is at 21% on large files" into
"code 55 DifferentialMissingFact, 15 files, 24,634 LOC" — and one attempt against
that class took php's large corpus from 21.13% → 9.86% file% and 18.80% → 7.61% LOC%.

So every round carries a standing FRCE requirement:

1. **Run it.** `--frce-report` on both corpora, every attempt; the attempt names the
   class it is attacking and shows that class before and after.
2. **Classify everything.** Unclassified stays under 3%. A new shape that no code
   covers gets a new code (56+), a one-line description and a sample — in the same
   commit that needed it.
3. **Keep the samples honest.** `--frce-dump` samples are what a lane turns into a
   reproduction; three per class, with file, line, column and the offending source
   line.
4. **Prefer the biggest class.** The class holding the most LOC is the next attempt,
   not the one that looks easiest.
5. **Re-rank after every merge.** A class that shrinks can hand the lead to another;
   the table is the plan, so it is regenerated, never remembered.

## Reporting

`report.json` carries, per language: file%, LOC%, both corpora, the FRCE table and
the unclassified share. `neon_blockers.json` carries any FRCE miss with its code,
template and sample. Numbers are also stated in plain text in the final message,
because attachments are not always downloadable.
