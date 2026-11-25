# Holdout: what is off limits, and why

Owner decision, 2026-11-25. This document is the directive both lanes get on every
reseed. It is not advice; a lane that violates it produces void numbers.

## The order of operations (non-negotiable)

1. **Expand** the corpus until every language has 5+ distinct repositories
   (`tools/corpus_expand.py --target 5`).
2. **Freeze** a holdout from the expanded corpus, per language, by repository.
3. **Re-baseline** every language on the new tuning set. Everything measured before
   the freeze date is *historical and non-comparable* - it was measured on a
   different denominator.
4. Optimise the tuning set only.
5. When the gates pass on the tuning set, measure the holdout **once**.

Expanding after the freeze replaces the corpus under a manifest that already
claims a hash, which is how java went from 21k to 4k files and csharp nearly
doubled while nobody noticed.

## Why by repository, not by file

Files inside one repo share idioms, macros and style. A random 20% of *files* leaks:
you hold out 200 kubernetes files that use the exact macro the tuning set
special-cased, they pass, and you conclude you generalised.

Per language: sort repos by size (LOC), take every fifth repo. Sorting by size
matters because the metric is LOC-weighted; a random split can put every
3,000-line monster in the tuning half and leave a holdout of small easy files.

A language with fewer than 5 repos cannot be split and is flagged
**UNHOLDOUTABLE**. It stays entirely in the tuning set and is measured cold - it
makes no holdout claim at all. Two repos is a coin flip, and training on the single
remaining repo overfits worse than having no holdout.

## Temporal holdout for thin languages

For any language that cannot be split (and as a second, stronger check for the rest):
after the 80% gate passes, pull **fresh repositories that were never in the corpus**
and measure cold. That is a genuinely unseen sample, and it is stronger than
anything carved out of what we already trained on.

## One shot per peek

The holdout is measured once. The moment it is looked at, a rule is fixed and it is
re-measured, that set is training data and the number stops meaning anything.

- Do not measure the holdout per round.
- Do not open a holdout file to debug a loss.
- If the holdout fails, fix against the failure and know the next measurement is
  weaker. Rotate to fresh repos for a second clean read.

## The number to watch

Not the holdout absolute - the **gap** between tuning and holdout, tracked over
rounds. A stable 2% gap is a parser. A gap that grows from 1% to 8% across ten
rounds is us learning corpus45 instead of COBOL.

Also: the full corpus is a **progress report**, never a test. It contains the
tuning data, so it will always look good.

## Mechanics

```bash
# 1. expand to 5+ repos per language (before the freeze)
python3 tools/corpus_expand.py --target 5

# 2. split, copy the holdout for real, freeze it read-only, write the manifest
python3 tools/holdout_split.py \
    --corpus /tmp/opencode/lean/corpus45 \
    --train  /tmp/opencode/rg_train_corpus \
    --holdout /tmp/opencode/rg_holdout_corpus \
    --manifest /tmp/opencode/rg_holdout_manifest.json --freeze

# 3. re-baseline the tuning set
python3 tools/corpus_fingerprint.py --root /tmp/opencode/rg_train_corpus \
    --all --write-baseline tuning-baseline.json
```

The holdout tree is `chmod 0444/0555` (frozen), its manifest records a sha256 over
every path and file hash, and its repos are excluded from `--frce-dump` and from
every sweep, so no sample line can leak into a fixture. **If a line from a holdout
file lands in a fixture, that file is training data now.**

## Lane contract, verbatim, for every reseed

> Your corpus is the tuning set only. `rg_holdout_corpus` (and any repo listed in
> `rg_holdout_manifest.json`) is OFF LIMITS: do not read it, do not measure it, do
> not dump FRCE from it, do not copy a line of it into a fixture. Before your first
> measurement, run `python3 tools/corpus_fingerprint.py --root <your corpus> --all
> --baseline <baseline.json> --check` and paste the verdict. If it refuses, stop and
> say so loudly - do not measure. Report tuning numbers only; the holdout is measured
> once, by the host, after the gate.
