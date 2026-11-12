# Unsolved — items bounced off repeatedly

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

Reverted both edits so the tree stays at a measured state (82f054f, js 851).
