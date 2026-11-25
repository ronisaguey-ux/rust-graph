# Anti-overfitting rules for a round

Every improvement so far was measured on the corpus it was tuned on, so the honest
question is not "did the number go down" but "did it go down for a reason that
exists in the language, or for a reason that exists in these repos".

## What the scanner found (2026-11-25)

`tools/overfit_scan.py` extracts every string literal used in a comparison and looks
it up in a document-frequency index of the corpus (`tools/corpus_token_df.py`,
110,096 files, 2.67M distinct tokens). Of 4,204 rare literals, 1,042 are used in a
decision. Each one is classified, because "rare" alone is meaningless here:

| class | findings | in a guard | meaning |
|---|---:|---:|---|
| `cst_kind` | 1,174 | 169 | a node kind our grammar defines - expected, harmless |
| `orphan_kind` | 1,716 | 589 | snake_case our grammar never emits - dead rule or leak |
| `source_identifier` | 601 | 250 | UPPER/camelCase vs source text - the overfit risk |
| `message` | 713 | 34 | a sentence, not a decision |

The `source_identifier` bucket is dominated by **dialect names** - `TSX` (45 uses),
`SWIFT` (31), `JSX` (23), `SCALA` (8), `OBJC` (3) in `src/parser.rs` - which are the
program's own `Lang` selectors and legitimate. Real language keywords with low
corpus frequency (`__attribute`, `willSet`) are also fine.

**Genuine overfitting, the whole of it:**

1. `src/deep_facts/c_family.rs:2335-2345` - `lua_isuserdata`, `LUA_API`,
   `JEMALLOC_EXPORT`. Repo symbols from musl/redis/Lua headers, compared against
   source text, present in **zero** corpus files. Pure fitting to a header we had.
2. `crates/deep-grammars/src/batch_d/css.rs:578` - `#1a73e8`, a specific CSS colour.
   A rule keyed on one hex value.
3. 1,716 `orphan_kind` literals the grammar never emits - rules that cannot fire.
   Dead weight, not damage, but they hide real rules in the noise and they misfire
   the day a shape changes.

That is the strip list. It is small, which is good news: the parser is not riddled
with corpus knowledge, it is littered with dead vocabulary and three symbols.

## Strip list (applied when both lanes are back)

- `c_family.rs`: drop the three header symbols, re-measure c on the expanded corpus.
- `css.rs`: replace the colour literal with a hex-colour shape (`#` + 3/6/8 hex).
- `orphan_kind`: remove literals with no producer; a literal must be traceable to a
  `Kind` the grammar defines, or to a keyword list, or it goes.

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
