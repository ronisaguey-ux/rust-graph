# Anti-overfitting rules for a round

Every improvement so far was measured on the corpus it was tuned on, so the honest
question is not "did the number go down" but "did it go down for a reason that
exists in the language, or for a reason that exists in these repos".

## What the scanner found (2026-11-25)

`tools/overfit_scan.py` reads every string literal used in a comparison in the
parser and looks it up in a document-frequency index of the corpus
(`tools/corpus_token_df.py`). 4,804 literals are rare (rarest token in <= 25 files);
**1,134 of them are used in a decision**.

Three distinct smells, worst first:

1. **Corpus-named literals.** A literal that spells a repository or one of its
   exports - e.g. `lua_isuserdata`, `LUA_API`, `JEMALLOC_EXPORT` in
   `src/deep_facts/c_family.rs`. These fire on one repo's headers, not on C.
2. **Tree-sitter node names as heuristics.** `call_expression`,
   `direct_declarator`, `parenthesized_declarator`, `designated_field`,
   `nameof_expression`, `object_creation_expression` - grammar vocabulary borrowed
   from one particular grammar and matched against our own CST. Generates rules that
   only cover the files that happened to be in the corpus.
3. **Absent-in-corpus literals.** `property_attributes`,
   `specified_macro_item`, `bare_user_declaration`, `user_cast_head` appear in
   **zero** corpus files: dead rules, kept because a round once added them and no
   gate ever removed anything. Dead rules cost nothing today and will misfire the day
   the language changes shape.

## The rules a lane must follow

1. **Syntax classes only.** A rule keys on tokens, delimiters, keywords of the
   language's own grammar, or on structure (nesting, statement boundaries). Never on
   an identifier that exists in our corpus.
2. **No corpus vocabulary.** A literal that names a repo, a package, a macro or an
   exported symbol is rejected in review, even if it removes 30 rows.
3. **Keyword literals must be attested.** If a rule compares against a word, that
   word must appear in the language's own reserved-word list (or the grammar's
   `keywords.js`/`grammar.js`). Put the source in the attempt note.
4. **Fixtures are synthetic.** Every rule ships with a fixture written by hand from
   the language spec, not copied from a corpus file. A corpus-derived fixture is
   training data and cannot validate anything.
5. **No absolute caps tuned to a file.** `n_calls = 400`, `n_zeros = 1500` survive
   because they happen to fit today's corpus. Caps derive from file size (bytes,
   lines) and are stated as a function, not a constant.
6. **Dead rules get deleted.** A literal that matches nothing in the corpus twice in
   a row is removed in the same round that notices it.
7. **A rule that fires on <= 5 corpus files needs a construct name** - "C11 generic
   selection with a parenthesised controlling expression" - not a file name. If you
   cannot name the construct from the language spec, you are fitting a file.
8. **Tuning set only.** Holdout repos are excluded (see `docs/HOLDOUT.md`).

## Mechanically checkable

```bash
python3 tools/corpus_token_df.py                # rebuild the index (one pass, ~80s)
python3 tools/overfit_scan.py --rare 25          # ranked offenders
python3 tools/overfit_scan.py --json > /tmp/overfit.json
```

A round is not done if it *adds* a literal with `token_df` under 25. That check is
mechanical: `overfit_scan.py` before and after, diff the finding set, justify every
new entry or revert the attempt.

## Unexplained signal we already have

Round 16's c work went from 4.59% LOC to 0.01% by adding `augment_c()` with call
normalisation and `_Alignof` handling. That is a 400x improvement from two
constructs, which is exactly the shape of a rule that fits *musl and redis* rather
than C. It must be re-measured on the expanded corpus and on fresh repos before it
is believed - and if it collapses, the honest reading is that we learned two repos.
