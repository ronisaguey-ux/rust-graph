# Unsolved — what still falls back, with the evidence

Measured 2026-11-15 on the committed tree (`c0b107e`) with the release/musl binary:

    cd /tmp/rg-grunt
    CARGO_TARGET_DIR=/tmp/rg-grunt-target cargo build --release --features "deep parser"
    RG_BIN=/tmp/rg-grunt-target/x86_64-unknown-linux-musl/release/rust-graph \
      python3 tools/loss_count.py          # the only accepted metric
    RG_BIN=... python3 tools/loss_shapes.py --corpus rust   # cluster a corpus

| corpus | files      | losses | per file |
|--------|-----------:|-------:|---------:|
| js     | 58 / 1854  |    103 |     1.78 |
| ts     | 55 / 2000  |     43 |     0.78 |
| python |  8 / 568   |      8 |     1.00 |
| rust   | 93 / 1816  |    107 |     1.15 |

`tools/phase4_gate.py` PASSES (js 58 ≤ 150, ts 55 ≤ 120, python 8 and rust 93
pinned, every fallback attributed, the construct record matches the sweep).

Both columns are floors. A file is charged one loss per **kind** — the import,
symbol, call and variable checks stop at their first miss — and one per unpaired
event. One corpus reports fewer ts losses than ts files (55 files, 43 losses),
which is the same fact from the other side: `deep_losses` can be 0 on a file that
still falls back, so `tools/loss_count.py` exits non-zero with
"fewer losses than files" on this tree.

## Clause totals (what the guard actually said)

| corpus | no event on a line both readers can mean | a name the lexer read is not in the CST's value | a call or constant | the CST wrote the value as the constant 0 | target list differs |
|--------|-----:|-----:|-----:|-----:|-----:|
| js     | 61 | 11 | 2 | – | 0 |
| ts     | 14 | 11 | 3 | – | 4 |
| python |  0 |  4 | 2 | – | 1 |
| rust   | 12 | 36 | 14 | 12 | 1 |

---

## 1. A multi-line expression is one event to the CST and another line to the lexer

61 js, 14 ts, 12 rust, 0 python. This is the largest remaining class and most of
it is one mechanism.

    /tmp/opencode/sweep-local/js/000013_css-tokenizer.js:167
        return isBetween(code, 0 /* CodePoint.Null */, 8 /* CodePoint.Backspace */)
            || code === 11 /* CodePoint.LineTabulation */
            || isBetween(code, 14, 31)
            || code === 127 /* CodePoint.Delete */;
    loss: event x line 169 ... no event on a line both readers can mean

The lexer stamps the chain on the line each `||` operand starts; the CST writes
one binary expression and stamps it where its own node starts. The guard pairs
an event only within one line (`covers`, `lines_near` in `src/pipeline.rs`), and
that window is load-bearing: it is what stops a duplicate target on another line
standing in for the event that lost its value (`covers`'s `Miss::Line` comment).

Widening the window is the only mechanical fix and it trades a false negative
for a false positive in both directions. **Not attempted.**

Not every row in this clause is that mechanism. `000032_index.js:783-786`
(`exports.Browser = capabilities.Browser`) is inside a function body the CST
files under a different symbol, and `001165_lib.rs:176` is a `let` inside a
`#[pyfunction]` whose CST body carries no event at all. Those are counted here
because the guard reports them with the same sentence.

## 2. Rust: "a name the lexer read is not in the CST's value" — 36 of 107

Three distinct shapes sit in the same clause because the guard reports the first
name it could not match.

* **A `match` arm's block is folded by the lexer, nested by the CST.**
  `000034_emit.rs:99`, `let layout = match self.checked.layout { … }` — the lexer
  writes `m:[n:match, …, m:[c(n:String;[]), m:[…]]]`, the CST nests one merge per
  arm (`m:[m:[n:None,c(n:String;[])],m:[0,…]]`). 4 of the 36 are this file, which
  the corpus holds four times.
* **A turbofish inside a struct literal.** `000946_mod.rs:101`,
  `Self { client, info, _res_type: PhantomData::<RT> }` — the lexer writes
  `m:[n:_res_type,n:PhantomData]`, the CST writes `m:[n:_res_type,0]`: a path
  with `::<…>` forms as the constant in `rust_form`'s field-initializer arm.
* **An `if let` struct pattern** (`000973_internal_flags.rs:82`):
  `if let Self::Enabled { core, .. }` — the lexer writes the path
  `n:Self.Enabled`, the CST's let-condition arm forms the value and no name.
* **The constant-receiver call of §3, one level out**
  (`001757_metrics.rs:143`, `let ax = 0i64.max(-dx) as usize;`): the call folds to
  `0`, so the cast's `usize` is the name that goes missing.

Enum bodies (`enum E { Start { kind: Kind }, Finish }` read as one tuple by the
lexer) were in this clause until `c3f0355`/`f1a9112`; `/tmp/rta/a.rs` is clean on
this tree, so the entry is gone rather than kept for sentiment.

## 3. Rust: "the CST wrote the value as the constant 0" — 12 of 107

`0i64.max(-ox)`, `0f64.max(iw.min(round(x)))` (`001559_displacement.rs:63`,
`001754_raster.rs:51`). The CST names the call `.max` with receiver `0`; the
guard's shape folds any leading-dot path to its receiver, because that is how a
`Path(x).resolve()` chain is spelled on the two sides. Folded, the CST's whole
value becomes the constant `0`.

Tried and reverted, twice:

* fold only when the receiver is itself a call (`receiver.starts_with('c')`):
  js 128 → **510** losses / 291 files, ts 45 → 48, and `loss_count` then reports
  "more files than losses", so the guard itself breaks;
* fold unless the receiver is literally `"0"`: js 128 → **260**, rust 112 → 140.

The right fix is CST-side (name the call `max` instead of `.max` when the
receiver is a literal, matching the lexer's spelling), but it moves every
literal-receiver call in four corpora and needs its own measurement.

## 4. The lexer names an attributed item after its attribute (latent, ~58 files)

# Historical bounce log (kept for the failed attempts)

Each entry: the shape, the smallest repro, what was tried and why it failed, and the next
idea worth trying. Nothing here is a guess dressed as a plan; the failed attempts are listed
so the next reader does not repeat them.

Baseline when this file was started (HEAD `cbd922b`): rust-graph own `src` 0 fallbacks,
Felicity 0, cargo 1451 green, pytest 889. Local corpus sweep — python **242/568**,
rust **396/1816**, js **1527/1995**, ts **1375/2000**.

---

## 1. `export { default as X } from './y'` records no import in the CST

Measure: `rg build /tmp/imp3 --no-cache --stdout --lexer` vs the same with `--deep`.
`/tmp/imp3/000007_index.js` has 12 such lines; the lexer records 12 imports
(`name default`, `alias X`), the CST records **0**.

Why it matters: roughly 428 of the 548 JS import failures and most of the TS equivalents
are this one shape.

Tried: nothing committed. The CST's `export_statement` handler does not treat a
`from`-with-clause as an import at all.

Next idea: in the JS/TS export handling, when an `export_statement` has a
`source`/`string` child and a named-export list, emit one `Import` per specifier with
`name` = the exported name (`default` or the identifier) and `alias` = the local alias,
matching the lexer's record exactly.

---

## 2. Struct literal vs block in the one-pass lexer (`deep_facts/mod.rs:240`)

Symptom: `Kinds { definitions: A, calls: B }` — the lexer opens a BLOCK at the brace, so
field keys become statement binders. This blocks `deep_facts/mod.rs`, `lexer.rs`,
`resolve.rs`, `pipeline.rs`.

Tried and reverted:

| attempt | outcome |
|---|---|
| guard `is_block` when braces open | no effect |
| trim the statement at the control keyword | no effect |
| bare path + `{` is not a block | repro fixed, corpus 7 → 11 |
| capitalised path is not a block | corpus 15 |
| token lookahead (`ahead: Vec<Token>` + `peek_token`, rule "Ident then `:`") | repro still failed, corpus 7 → 10 |

The buffer machinery itself works: `ahead` in `Parser`, `next_token()` drains it,
`peek_token(n)` fills it from `self.lexer.next()`, `run()` calls `next_token()`.
Trap: `peek_token` must pull from the lexer, never from `next_token`, or it loops forever.

Next idea: scan the whole brace body at depth 1 for a `:` that is not inside a nested
brace, and refuse the block reading only when the statement is a bare path that is not a
definition keyword (`struct`/`enum`/`trait`/`impl`/`mod`/`union`).

---

## 3. Python corpus: 242 of 568 still fall back

Classes (from the sweep): event missing a name the lexer read 133, no shared line 36,
other 33, value written as 0 22.

Not yet reduced to one repro. Next step: take the most common exact message in
`/tmp/opencode/sweep-python.json`, reduce it to a 5-line file, and fix that shape only.

---

## 4. `pos++;` as a statement is a fact the CST never wrote — SOLVED (2026-11-12)

Measure: `/tmp/rq6/a.js` — `function f(code, plus, minus, pos) { if (code === plus || code === minus) { pos++; } return pos; }`
The lexer writes `["x",3,["n","pos"]]` for the `pos++` line; the CST wrote no event at all.

Cause: `update_expression` has no arm in `form`, so it formed the constant `0`, and
`expression_statement` only emits an event when the value is not `0`.

Fix: emit the operand's name in `expression_statement` only — an update inside a larger
expression (`a[i++]`) is not a statement and the lexer does not write one.
Measured: js 863 -> 862, ts/python/rust re-measured, cargo and pytest green.

Correction for the record: two earlier measurements (js 1135 and 1137) blamed this arm,
but the real cause of those numbers was a different change made in the same window —
`require_module` refusing a `require` nested in an argument list, which cost 276 JS files
(`_interopRequireWildcard(require("react"))`). That change was reverted in `b7f25c9`; with
it gone the arm is a one-file improvement. Lesson: one change at a time, or the numbers
lie about which change did what.

---

## 5. A multi-line expression is one CST event and several lexer events

Measure: `/tmp/rq7/a.js` —
`const humanErr = (detail, status) => (A && B.humanErrorText) ? B.humanErrorText(detail, status) : \`x${status ? \`y${status}\` : ''}\`;`
spanning four lines. The lexer writes `x line 3` (the call) and `x line 4` (the template
fields) on the module symbol; the CST folds the whole body into the arrow's single
`r line 1` event, so lines 3 and 4 carry nothing and the file falls back with
"no event on a line both readers can mean".

Which reader is right: the CST. One expression is one fact, and the lexer's per-line
events are an artefact of reading line by line. But the guard treats the lexer's record as
the truth to be covered, so the difference is counted as a loss.

49 files in the JS corpus fail exactly this way (`no event on a line both readers can mean`
with empty targets), and the same shape appears in TS and Rust.

Options, none of them cheap:
1. Teach the guard that a lexer event is covered when every name it carries appears in a
   CST event that spans its line — requires line ranges per event, which the fact format
   does not carry today.
2. Make the lexer emit one event per statement, not per line — a change to the one-pass
   reader's core model, and the largest single change on this project.
3. Leave it and keep the fallback honest.

Do not attempt (1) or (2) without a full-corpus measurement plan: both touch the guard's
semantics, and every earlier "small" guard change moved hundreds of files.

---

## 6. The `{ get: all[name], enumerable: true }` object key — values now agree, the guard still says the event is missing

Measure: `/tmp/rq8/a.js` — `var __export = (target, all) => { for (var name in all) __defProp(target, name, { get: all[name], enumerable: true }); };`
46 JS corpus files fail this way (`a name the lexer read is not in the CST's value`).

What was tried (both reverted, unproven):
1. `property_identifier` added to the name arm of `form` — no change: the key `get` is an
   anonymous `get` token in this grammar, not a `property_identifier`.
2. The `pair` arm rewritten to take the FIRST child as the key and fall back to its raw
   text when `form` yields `0` — this works: the CST value becomes
   `m:[m:[n:get,n:all],m:[n:enumerable,n:true]]`, byte-identical to the lexer's.

Despite identical values the hybrid build still reports
`event x line 4 … (no event on a line both readers can mean): … CST no event of that marker
within one line`. Both records hold one `__export` symbol at line 2, the deep `--deep` run
writes the `x` event at line 4, and the symbols pair. So the loss is NOT in the value and
NOT in the pairing — the guard's `found` list is not seeing the event at all.

Next step before touching the value again: instrument `uncovered` for one file
(`eprintln!` the `found` rows and the paired symbol's raw events), because everything
above is inference and the values already match.

**Resolved (70863d2).** The instrumentation showed the object key was never the loss. `found`
held `x line 4` with the exact value the lexer wrote, but pairing ran in one loop: the first
want (`x line 3`, the `for (var name in all)` header) took the later line's row through the
"same marker, any line" fallback, and the statement that owned line 4 was then reported as
lost. Pairing is now two passes — every statement claims a row on its own line first, then the
leftovers fall back (a `for` header's row first, since the CST files the binder under `targets`
and the iterable under the value; `a_row_covers` checks both fields) — which also fixed the
same theft in Python and Rust.

Measured with the pair-key edit REVERTED (it is still a real value gap but it costs more than it
wins — see below):

| corpus | before | after |
|---|---|---|
| js | 851 | 838 |
| ts | 448 | 448 |
| python | 148 | 146 |
| rust | 360 | 345 |

The pair-key edit itself (`pair` takes its first child as the key, falling back to raw text)
makes the CST value byte-identical to the lexer's, but it turned `{ get: … }` keys into names
everywhere: js 851 -> 872 and python 148 -> 201. Not landed; it needs a rule that names a
property key only where the CST lost it.

---

## 7. An arrow function's symbol: the corpus and the fixture disagree about its name

Measure (four corpora, js/ts/python/rust): the `=>` block gets its own symbol, but *which* name
depends on how the arrow is bound, and the two oracles disagree.

| variant | js | ts | rust | fixture `deep_facts_every_disagreement_is_explained` |
|---|---|---|---|---|
| no separate symbol (before) | 753 | 447 | 345 | pass |
| always anonymous (`<anonymous>@N`) | **677** | **417** | 345 | **fail**: lexer-only `<anonymous>@21`, `@28` for `const variance = (values) => {` |
| anonymous only for a member target (`module.exports = (a) => {`) | 746 | 426 | 345 | pass |
| always named by the whole target path (`module.exports`) | 754 | 448 | — | pass |

The corpus prefers the variant the fixture rejects, by ~69 files, and the fixture's complaint is
exactly the case the corpus likes: `const variance = (values) => {` read as `<anonymous>@21` when
the CST names it `variance`. So either the fixture's expectation is narrower than the CST's real
behaviour, or the guard's symbol pairing is looser than its name comparison suggests — pairing by
name would have flagged those files too. Whichever it is, that is the next thing to measure: take
one of the 69 (`000096_fish.js` line 96) and print the paired symbols' names on both sides.

Landed: the member-target variant (js 753→746, ts 447→426, python 146, rust 345), because it is the
best state that keeps the suite green.

**Answered 2026-11-12 (c5e5996).** The question was the wrong question: no arrow's *name* was ever the
problem. Printing both sides on `000096_fish.js` showed the failure eight lines past the arrow, at
`if (conditionArg) parts.push(conditionArg);` — one line that both readers write **two** events for
(the condition and the call). The guard's same-line pass let `covers()` pair the call with the
condition's row, so the call's own row was left over and the file was blamed for an event that was in
the record all along. So the fixture's expectation was right and the *pairing* was loose.

Fix tried, measured, reverted: an exact-value claim before `covers()` in the same-line pass. python
146→**114**, js 725→721, ts 409 held, **rust 345→445** — a hundred rust files broke. Those files were
passing by the cross-line (|Δline| ≤ 1) mispair the change removes, so the change is more correct and
the corpus rewards the sloppiness. Rust's token-merge idiom (`m:[0]` against the CST's
`m:[c(…),n:LPAREN]`) has no exact partner, so the row it used to borrow is gone. A per-language gate
on the exact pass would buy python's 32 and keep rust's 345, but it hides the real defect: the lexer
writes `0` where the CST writes a merge. That is a lexer fix, not a pairing fix — next item.

Kept: the arrow itself is now **one constant** (`map(s => s.name)`), and Rust keeps the match-arm
path (`=>` drops the pattern, the arm expression stands). Measured js 746→**725**, ts 426→**409**,
python 146, rust 345 held, cargo 1463 / pytest 1169 green.

## P1-08 — row counts for each candidate construct, recorded before the code

The rule this table exists to enforce: a construct is a candidate only when the
number to explain is visible first. Both readers were run over the same fifty JS
files (`/tmp/js50`, the first fifty of the local js corpus) with
`build <dir> --deep|--lexer --no-cache --stdout` and the graph rows counted.

| construct (graph row) | CST rows | lexer rows | difference | why it is a candidate |
| --- | ---: | ---: | ---: | --- |
| symbols | 545 | 455 | **+90** | the CST resolves a definition the lexer only sees as a token |
| edges (kind `call`) | 464 | 448 | **+16** | calls recovered after a fallback, mostly inside object literals |
| flows | 533 | 444 | **+89** | the "event" rows: the CST writes one per statement, the lexer per expression |
| imports | 98 | 96 | +2 | re-export shapes |
| unresolved | 0 | 0 | 0 | nothing to explain — not a candidate |

Fallback reasons over the whole js corpus (1995 files, 646 files falling back),
counted from `/tmp/new_js.json`:

| reason | files |
| --- | ---: |
| the CST lost a event | 501 |
| the CST lost a symbol | 53 |
| the CST lost a call | 45 |
| the CST lost a var | 38 |
| the CST lost a import | 9 |

The two tables disagree on purpose. Flows differ by 89 on fifty files while only
13 of those files fall back: the CST writes more flow rows than the lexer nearly
everywhere and that is normal, so a flow-count difference is *not* by itself the
bug. The candidate is the file that falls back, not the row total. Symbols are
the opposite — the lexer is the lighter writer and the CST's extra 90 are rows
the default path only reaches when it takes the CST.

So the next code change is chosen from the 501 event rows, and the first thing
recorded about each is the CST row count and the lexer row count for the same
file, side by side, in this section. Not after.

## Phase 1 exit gate — the narrowed split, measured and rejected

The gate offered two doors: the narrowed split lowers js without raising ts,
python or rust, or the feature is removed and the phase closes as
measured-and-rejected. It is the second, and the numbers are on the record.

| measurement | with `event-split` | default build |
| --- | ---: | ---: |
| js, fifty files, files falling back | 18 | 18 |
| js reason histogram (event/call/var/symbol) | 13/2/2/1 | 13/2/2/1 |
| js rows, fifty files | 5743 | 5743 |
| ts rows, fifty files | 994 | 994 |
| python rows, fifty files | 22533 | 22533 |
| rust rows, fifty files | 9980 | 9980 |

The split changed nothing the guard compares, so it could not lower js, and with
no effect there was nothing for it to raise either. The feature, its `Segment` and
`Marker` types and its tests are deleted (`1082e0f`), the measure recipe no longer
mentions a flag that does not exist, and the default and deep suites both run
green without it. Phase 1 closes as measured-and-rejected: the idea was tested at
the smallest cost that could answer it, and the answer was no.

## P2-02 — exact-value-first pairing stays reverted

The idea was to claim a line for an exact value before anything else could. It was
measured and reverted on 2026-11: python 146→114 and js 725→721 against a rust
cost of 345→445. Nothing else on the scale moved enough to pay for that, and rust
going backwards on a hundred files is not a trade, it is a regression with a
side-benefit.

Current numbers, measured today on the same corpora:

| language | files | falling back |
| --- | ---: | ---: |
| js | 1995 | 646 |
| ts | 2000 | 397 |
| python | 568 | 145 |
| rust | 1816 | 345 |

The tree carries no trace of the experiment (no `exact_value`/`exact-first`
symbol in `src/`, `tools/` or `docs/`), so there is nothing dormant waiting to be
revived by accident. If it is ever retried, the rust column is the gate: a variant
that does not hold rust at 345 is rejected on the numbers, not on taste.

## The `for (; cond;)` header condition — 14 js files, not solved (2026-11-14)

Minimal reproduction after delta reduction of `000713_dispose.js`:

    function next() {
      for (; r.length > 0;) try {
      }
    }

Both readers were measured separately (`--lexer` and `--deep` on the same file):

    lexer: x line 2 n:r.length | b | x line 2 c(f) | j
    deep : b | b | x line 2 c(f) | j | j

The lexer writes the header condition; the CST does not, because every `for`-header
child after the second `;` is computed and thrown away in the `for_statement` arm of
`deep_facts` (`let _ = self.form(child)`). With a block body the guard tolerates the
difference; with a bare `try` statement as the body it does not — the symbol pairing
degrades to `paired next line 1` and all four events on the header line are reported
lost, so one file loses four times over.

What is NOT understood:

- Why the same missing event is accepted for `for (; c;) { … }` and rejected for
  `for (; c;) try { … }`. The pairing ("no event on a line both readers can mean")
  is what changes, not the reader.
- Whether the CST should write the header condition at all. Writing it changes the
  event list of every loop in the corpus, and the attempt to do the same in the
  lexer (`expression_event` on the condition when the init is empty) made two
  previously clean shapes fail — reverted, and not because of the numbers.

Everything here was measured, nothing here was landed.
