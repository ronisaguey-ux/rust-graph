# Unsolved — every remaining fallback, attributed

Baseline (HEAD `fc1fc07`, 2026-11-14), full sweep over the four local corpora:

| corpus | fallbacks | files |
|--------|----------:|------:|
| js | 111 / 1995 | 109 with a first loss |
| ts | 99 / 2000 | 81 |
| python | 14 / 568 | 14 |
| rust | 308 / 1816 | 306 |

Method: `RG_LOSSES=1 rust-graph build <corpus> --no-cache --stdout 2>loss_<lang>.txt`, one line per
disagreement, then the *first* loss of each file is attributed to a construct. Nothing here is left as
a general "the CST lost an event": a construct with two or more files is a table row, and every file
that does not share a row is listed individually by file:line, its source line, and the reader
disagreement it hit.

Two reader-side facts that explain many rows:
- a *definition symbol* the lexer opens and the CST does not is a callable shape the CST does not name
  (arrow chosen by a ternary, minified UMD `!function(e,t){…}`);
- a *module-level var/import fact* the CST never records is a fact the deep reader drops, not one the
  lexer invents — those rows are fixed in `src/deep_facts`, not in the lexer.

### JS — 109 fallbacks

| files | construct | example |
|------:|-----------|---------|
| 14 | call whose path the readers spell differently | `000011_ImportScriptsChunkLoadingRuntimeModule.js:196` |
| 14 | definition the lexer opens and the CST does not | `000039_7517.b4d8b77497bdb47f.js:1` |
| 12 | module-level var fact the CST never records | `000041_snapshot.B_Jk1wbt.js:1` |
| 10 | declarator | `000092_conversion.js:528` |
| 9 | parenthesised expression statement | `000222_74fad3407b748f3e.js:1` |
| 6 | control-flow header | `000031_source-map.js:2` |
| 6 | import fact the CST never records | `000122_express.js:1` |
| 6 | return statement | `000251_d3-array.js:506` |
| 5 | continuation line beginning with `}` | `000086_ReferenceLine.js:188` |
| 3 | arrow function | `000893_1028.812a1281fd9f7ba1.js:1` |
| 2 | method call on `this` | `000091_Compilation.js:1638` |
| 2 | prefix increment of a member | `000488_index.js:3371` |
| 2 | array literal or computed-key statement | `000721_errors.js:79` |

Remaining 18 files, each named by its own line (no catch-all bucket):

- `000022_mode.js:26` — `options.uid : process.getuid && process.getuid()` — no shared line for the event
- `000270_Request.js:167` — `Object.assign(this, args);` — no shared line for the event
- `000275_use-pagination.js:61` — `pageSize - spaceUnderActive` — no shared line for the event
- `000344_prepared-router.js:162` — `0 && (module.exports = {` — no shared line for the event
- `000358_options.js:33` — `placeholderPattern,` — no shared line for the event
- `000569_middleware.js:26` — `c.header(X_HONO_DISABLE_SSG_HEADER_KEY, "true");` — no shared line for the event
- `000606_offscreen_compiled.js:9` — `$jscomp.setPrototypeOf=$jscomp.ASSUME_ES6||$jscomp.TRUST_ES6_POLYFILLS&&typeof Object.setPrototypeOf` — a name the lexer reads is missing from the CST value
- `000777_VariantSchema.js:78` — `static [TypeId] = fields;` — target list differs
- `000908_matrix.js:302` — `a = 1;` — no shared line for the event
- `001028_react-jsx-dev-runtime.react-server.development.js:236` — `for (` — no shared line for the event
- `001090_fullscreen-user-rules.js:3147` — `target[protoStoreKey] || (target[protoStoreKey] = []) : // functions are added to the instance store` — no shared line for the event
- `001120_google-analytics.js:70` — `handleCallback(data, "eventCallback");` — no shared line for the event
- `001227_metrika-yandex-watch.js:37` — `asyncCallbackFromOptions(options);` — no shared line for the event
- `001399_import.js:52` — `exports.importJWK = importJWK;` — no shared line for the event
- `001494_circle.js:147` — `polar = abs(delta - pi) < epsilon,` — a name the lexer reads is missing from the CST value
- `001669_regeneratorRuntime.js:4` — `module.exports = _regeneratorRuntime = function _regeneratorRuntime() {` — no shared line for the event
- `001735_source-node.js:74` — `remainingLines[remainingLinesIndex++] : undefined;` — no shared line for the event
- `001737_ComponentLogger.js:99` — `args.unshift(namespace);` — a name the lexer reads is missing from the CST value

### TS — 81 fallbacks

| files | construct | example |
|------:|-----------|---------|
| 21 | call whose path the readers spell differently | `000261_number.d.ts:1` |
| 17 | declarator | `000196_fetch.d.ts:50` |
| 9 | class member carrying a modifier (computed key or annotation) | `000404_invalid-stream-part-error.ts:9` |
| 6 | module-level var fact the CST never records | `000345_PreflightResponseCache.ts:1` |
| 5 | continuation line beginning with `}` | `000640_PersistedQueue.d.ts:240` |
| 4 | arrow function | `000291_Utils.d.ts:150` |
| 3 | definition the lexer opens and the CST does not | `000114_Schema.ts:5020` |
| 2 | parenthesised expression statement | `000074_dom.d.ts:97` |
| 2 | control-flow header | `000594_config.ts:67` |

Remaining 12 files, each named by its own line (no catch-all bucket):

- `000038_parseJson.ts:31` — `parseJson.position = 0 as number` — a name the lexer reads is missing from the CST value
- `000117_types.d.ts:2441` — `$$eval<` — no shared line for the event
- `000365_memoizer.ts:169` — `kind satisfies never;` — no shared line for the event
- `000636_RpcClient.ts:239` — `never,` — no shared line for the event
- `000867_keyword.ts:85` — `gen.assign(` — a name the lexer reads is missing from the CST value
- `000940_index.ts:1307` — `getRandomValues<` — no shared line for the event
- `001019_custom-provider.ts:71` — `return {` — a name the lexer reads is missing from the CST value
- `001284_Sink.ts:803` — `fold<readonly [S, number], In, E, R>(` — no shared line for the event
- `001414_ElementHandle.d.ts:170` — `evaluate<Params extends unknown[], Func extends EvaluateFuncWith<ElementType, Params> = EvaluateFunc` — no shared line for the event
- `001587_JSHandle.d.ts:53` — `evaluate<Params extends unknown[], Func extends EvaluateFuncWith<T, Params> = EvaluateFuncWith<T, Pa` — no shared line for the event
- `001681_schemas.d.ts:256` — `-readonly [k in keyof T["shape"]]: ZodMiniOptional<T["shape"][k]>;` — no shared line for the event
- `001933_index.ts:1` — `/**` — other

### PYTHON — 14 fallbacks

| files | construct | example |
|------:|-----------|---------|
| 4 | module-level var fact the CST never records | `000084_util.py:1` |
| 2 | return statement | `000117__pyio.py:1962` |

Remaining 8 files, each named by its own line (no catch-all bucket):

- `000124_pydoc.py:2539` — `time.sleep(.01)` — a name the lexer reads is missing from the CST value
- `000170_calendar.py:97` — `_months.insert(0, lambda x: "")` — a call/constant the lexer reads is missing from the CST value
- `000219__parser.py:196` — `self.dict: dict[str, Any] = {}` — target list differs
- `000231___init__.py:175` — `raise Exception` — no shared line for the event
- `000242_imaplib.py:1510` — `gmtoff = -(time.timezone, time.altzone)[dst]` — a name the lexer reads is missing from the CST value
- `000310_mod_generics_cache.py:12` — `some_b: 'B'` — a call/constant the lexer reads is missing from the CST value
- `000358___init__.py:504` — `memmove = CFUNCTYPE(c_void_p, c_void_p, c_void_p, c_size_t)(_memmove_addr)` — other
- `000499_request.py:798` — `setattr(self, '%s_open' % type,` — a name the lexer reads is missing from the CST value

### RUST — 306 fallbacks

| files | construct | example |
|------:|-----------|---------|
| 95 | call whose path the readers spell differently | `000158_compress.rs:231` |
| 79 | declarator | `000033_lib.rs:177` |
| 29 | definition the lexer opens and the CST does not | `000031_check.rs:44` |
| 13 | module-level var fact the CST never records | `000138_incremental.rs:1` |
| 12 | `extern` block header | `000662_main.rs:23` |
| 8 | labelled loop header | `000122_incremental.rs:578` |
| 8 | async/await | `001081_worker_versioning_tests.rs:164` |
| 7 | Rust item head inside a block | `000922_metrics.rs:57` |
| 6 | call whose argument is a match or closure | `001012_complete_workflow_state_machine.rs:63` |
| 5 | macro/assertion call statement | `001162_matching_core.rs:245` |
| 4 | arrow function | `000071_visual_basic.rs:323` |
| 4 | control-flow header | `001480_reviewed_entry.rs:51` |

Remaining 36 files, each named by its own line (no catch-all bucket):

- `000040_mutate.rs:227` — `bytes[at + 1] = b'A';` — CST value is the constant 0
- `000134_cst.rs:193` — `all.sort_by_key(|&id| self.nodes[id as usize].start);` — a name the lexer reads is missing from the CST value
- `000254_mutate.rs:227` — `bytes[at + 1] = b'A';` — CST value is the constant 0
- `000348_cst.rs:193` — `all.sort_by_key(|&id| self.nodes[id as usize].start);` — a name the lexer reads is missing from the CST value
- `000483_mutate.rs:227` — `bytes[at + 1] = b'A';` — CST value is the constant 0
- `000577_cst.rs:193` — `all.sort_by_key(|&id| self.nodes[id as usize].start);` — a name the lexer reads is missing from the CST value
- `000707_mutate.rs:227` — `bytes[at + 1] = b'A';` — CST value is the constant 0
- `000801_cst.rs:193` — `all.sort_by_key(|&id| self.nodes[id as usize].start);` — a name the lexer reads is missing from the CST value
- `000918_future.rs:9` — `future: Pin<Box<dyn Future<Output = Result<R, BridgeError>> + Send + 'static>>,` — no shared line for the event
- `000928_runtime.rs:62` — `Trace = 0,` — no shared line for the event
- `000941_options.rs:81` — `WorkflowCommand {` — a name the lexer reads is missing from the CST value
- `000959_test_utils.rs:26` — `StartTimer {` — a call/constant the lexer reads is missing from the CST value
- `000984_queries.rs:504` — `core.complete_workflow_activation(WorkflowActivationCompletion::from_cmd(` — a name the lexer reads is missing from the CST value
- `000986_updates.rs:67` — `core.complete_workflow_activation(WorkflowActivationCompletion::from_cmd(` — a name the lexer reads is missing from the CST value
- `001023_timer_state_machine.rs:83` — `Self::from_parts(` — a call/constant the lexer reads is missing from the CST value
- `001073_heartbeat_tests.rs:111` — `core.complete_workflow_activation(WorkflowActivationCompletion::from_cmd(` — a name the lexer reads is missing from the CST value
- `001091_timers.rs:168` — `mock_cfg.expect_fail_wft_matcher = Box::new(move |_, cause, f| {` — a name the lexer reads is missing from the CST value
- `001096_child_workflows.rs:114` — `worker.register_wf(` — a name the lexer reads is missing from the CST value
- `001098_appdata_propagation.rs:36` — `worker.register_activity(` — a name the lexer reads is missing from the CST value
- `001100_activities.rs:755` — `core.complete_workflow_activation(WorkflowActivationCompletion::from_cmd(` — a name the lexer reads is missing from the CST value
- `001129_ids_generator.rs:282` — `value[18] = b'-';` — CST value is the constant 0
- `001256_dema.rs:166` — `self.value = 2.0f64.mul_add(self.ema1.value(), -self.ema2.value());` — CST value is the constant 0
- `001257_lr.rs:568` — `lr.update_raw(3.0f64.mul_add(x as f64, noise));` — a name the lexer reads is missing from the CST value
- `001283_ids_generator.rs:282` — `value[18] = b'-';` — CST value is the constant 0
- `001410_dema.rs:166` — `self.value = 2.0f64.mul_add(self.ema1.value(), -self.ema2.value());` — CST value is the constant 0
- `001411_lr.rs:568` — `lr.update_raw(3.0f64.mul_add(x as f64, noise));` — a name the lexer reads is missing from the CST value
- `001494_providers.rs:443` — `Resolved { root: String, scope: Option<Scope>, agent_scope: Option<Scope>, providers: Vec<&'static s` — a name the lexer reads is missing from the CST value
- `001499_bundle.rs:296` — `return copy_or_extract_local_bundle(sys, local);` — a call/constant the lexer reads is missing from the CST value
- `001513_js.rs:118` — `up[i] = b'0';` — CST value is the constant 0
- `001541_stop_baseline.rs:43` — `sessions(cache)?` — a call/constant the lexer reads is missing from the CST value
- `001664_live_accept.rs:76` — `Carbonized {` — a name the lexer reads is missing from the CST value
- `001679_dev_url.rs:78` — `(host, port)` — a name the lexer reads is missing from the CST value
- `001712_manifest.rs:103` — `packet` — a call/constant the lexer reads is missing from the CST value
- `001728_context_cli.rs:182` — `parts.push([` — a name the lexer reads is missing from the CST value
- `001744_ignores.rs:232` — `[` — a call/constant the lexer reads is missing from the CST value
- `001758_jsnum.rs:54` — `buf[i] = b'0';` — CST value is the constant 0

---

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
