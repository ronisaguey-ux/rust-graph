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
