# Unsolved — what still falls back, with the evidence

Measured 2026-11-15 on the committed tree (`cc6ac8a`) with the release/musl binary:

    cd /tmp/rg-grunt
    CARGO_TARGET_DIR=/tmp/rg-grunt-target cargo build --release --features "deep parser"
    RG_BIN=/tmp/rg-grunt-target/x86_64-unknown-linux-musl/release/rust-graph \
      python3 tools/loss_count.py          # the only accepted metric
    RG_BIN=... python3 tools/loss_shapes.py --corpus rust   # cluster a corpus

| corpus | files      | losses | per file |
|--------|-----------:|-------:|---------:|
| js     | 39 / 1854  |     83 |     2.13 |
| ts     | 53 / 2000  |     39 |     0.74 |
| python |  8 / 568   |      8 |     1.00 |
| rust   | 87 / 1816  |    105 |     1.21 |

`tools/phase4_gate.py` PASSES (js 39 ≤ 150, ts 53 ≤ 120, python 8 and rust 87
pinned, every fallback attributed, the construct record matches the sweep).

Both columns are floors. A file is charged one loss per **kind** — the import,
symbol, call and variable checks stop at their first miss — and one per unpaired
event. One corpus reports fewer ts losses than ts files (53 files, 39 losses),
which is the same fact from the other side: `deep_losses` can be 0 on a file that
still falls back, so `tools/loss_count.py` exits non-zero with
"fewer losses than files" on this tree.

## Clause totals (what the guard actually said)

| corpus | no event on a line both readers can mean | a name the lexer read is not in the CST's value | a call | a var binding | an import |
|--------|-----:|-----:|-----:|-----:|-----:|
| js     | 67 | 3 | 3 | 0 | 10 |
| ts     | 28 | 2 | 8 | 0 | 1 |
| python |  7 | 0 | 1 | 0 | 0 |
| rust   | 84 | 5 | 13 | 3 | 0 |

---

## 1. A multi-line expression is one event to the CST and another line to the lexer

67 js, 28 ts, 84 rust, 7 python — 186 of the 235 rows left (four fifths).
This is the largest remaining class and most of it is one mechanism.

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
for a false positive in both directions. **Not attempted.** Instead, the lexer
now continues JS/TS statements across leading binary/comparison operators on the
next line (`next_starts_with_ternary` in `src/parser.rs`: `||`, `&&`, `??`,
`==`, `!=`, `<=`, `<<`, `**`, `* `, `+ `, `- `, `/ `, `% `, `| `, `& `, `^ `,
`< `), keeping multi-line chains on one statement line without touching the guard.

Not every row in this clause is that mechanism. `000032_index.js:783-786`
(`exports.Browser = capabilities.Browser`) is inside a function body the CST
files under a different symbol, and `001165_lib.rs:176` is a `let` inside a
`#[pyfunction]` whose CST body carries no event at all. Those are counted here
because the guard reports them with the same sentence.

## 2. The lexer names an attributed item after its attribute (latent, ~58 files)

    /tmp/rt7:  #[rstest] #[case(1)] fn t() { … }   ->  lexer symbol `m.case`
    /tmp/rt8:  #[cfg(test)] mod tests { … }        ->  no owner; flows are `feed`, not `tests.feed`

`c_style_head` reads the attribute's `(` as the parameter list, so `#[derive(..)]
pub enum E {` opens a symbol `derive`, `#[case(1)]` names the function `case` and
owns its body, and `#[cfg(test)] mod tests {` opens `cfg` instead of the module.

Stripping the attribute fixes it — `/tmp/rt6` (001201_vr.rs) goes from 4 losses
to 0 and the `tests.` owner appears — **but the corpus count rises**: rust 141 →
199 (attribute stripped in `open_block` and `c_style_head`) and 141 → 207
(`c_style_head` alone). The reason is in the guard, not the lexer: it pairs a
lexer symbol with a CST symbol by name and `continue`s when nothing pairs, so
every event filed under a bogus `derive` / `case` / `cfg` symbol was never
compared at all. Correct naming exposes ~58 real losses that the metric never
saw.

The next fix is therefore the guard — compare an unpaired lexer symbol's events
against the file's rows instead of skipping the symbol — and only then the
lexer. Both edits were reverted with `git checkout -- src/parser.rs`.

**Update (2026-11-18):** `skip_modifiers` and `function_head` (`261a215`, `320ab65`)
now skip leading `#[...]` attributes for plain `fn` and `mod`, and `h-argon`
closed the remaining `const fn` and `extern "C" fn` cases so `c_style_head` never
reads a Rust attribute's `(` as a function name. The guard's skip of unpaired
lexer symbols (`src/pipeline.rs`) remains open.

## 3. Rust: a call whose name the CST cannot place — 13 of 105

Two shapes, all "the CST wrote no call where the lexer read one":

* **An associated path called like a function.** `FnMut` (`000947_raw.rs:218`),
  `Helper` (`000965_mod.rs:438`), `UntilResolved` (`000995:135`), `Custom`
  (`001002:145`), `PendingStart` (`001040:182`), `Bezier` (`001457:1464`), and
  `Some` twice (`001629:333`, `001644:669`). The lexer reads `Path(args)` — after
  a `::`, or the `Some` of a match arm — while the CST writes a tuple or struct
  expression with no `call_expression` to read.
* **A field chain whose fold lands elsewhere.** `self.max_deque.clear`
  (`001183_stochastics.rs:188`), `self.update` (`001214_rsi.rs:66`), and
  `history_event.Attributes.WorkflowExecutionUpdateAcceptedEventAttributes`
  (`001016:796`). The guard folds a leading-dot path to its receiver — that is
  how `Path(x).resolve()` is spelled on both sides — and that receiver is not the
  name the CST kept.

Five of the thirteen also carry the owner `derive`, which is §4: the
`#[derive(..)]` head is taking the scope as well as the name.

Two var rows survive next to these: `var report.price = Some`
(`001121_proptests.rs`, twice) and `var NODE_LIMIT = Limit` (`001815_sample.rs`).

Tried and reverted, twice, on the fold itself: fold only when the receiver is
itself a call (`receiver.starts_with('c')`) — js 128 → **510** losses / 291 files
and `loss_count` then reports more files than losses, breaking the metric; fold
unless the receiver is literally `"0"` — js 128 → **260**, rust 112 → 140. The
repair has to be CST-side (write the member name the lexer writes), which moves
every literal-receiver call in four corpora and needs its own measurement.

## 4. Rust: the symbol clause is only an attribute name now — 5 of 105

`000976_log_export.rs:233` `symbol instrument kind 1`, and `symbol case` on
`001119_oculus_sot.rs:93`, `001263_hma.rs:384`, `001273_oculus_sot.rs:93`,
`001417_hma.rs:384` (two files, each held twice by the corpus).

Both are §4: `#[instrument(...)]` and `#[case(1)]` open a symbol named after the
attribute, because `c_style_head` reads the attribute's `(` as a parameter list.
The clause is counted here only because the symbol check fires before the event
check. The value-mismatch shapes that used to hold it — a match arm's folded
block, a turbofish inside a struct literal, an `if let` struct pattern, the
literal receiver — were closed by `f1a9112`, `65436fb` and `d446c97`, and the
clause went 36 → 5 on the same corpora.

## 5. TypeScript: a `.d.ts` declaration is not a runtime call — 4 rows

`is_type_property_line` now excuses a *call* on a `.d.ts` member line the same
way it already excused an event (js 128 → 122, ts 45 → 43, rust 112 → 111 in the
sweep that landed it). What is left in this family is the type expression
itself:

`/tmp/opencode/sweep-local/ts/001886_Config.ts:492`:

    ): Config<
      [Arg] extends [ReadonlyArray<Config<any>>] ? {
          readonly [K in keyof Arg]: [Arg[K]] extends [Config<infer A>] ? A : never
        }
        : [Arg] extends [Iterable<Config<infer A>>] ? Array<A>

The lexer reads `extends`, `readonly`, `keyof`, `infer`, `never` and the type
names as a tuple value and writes an `x` event; the CST writes nothing, because
there is no runtime fact. The lexer has no "inside a type" mode in the one-pass
reader — the type is only skipped where
a declaration line can be recognised (`is_type_property_line`), and this is the
continuation of an arrow type, not a line that says `type` or `interface`.

## 6. Python: eight files, eight constructs, no cluster

`tools/loss_shapes.py --corpus python` gives one loss per shape. Each is listed
here so the next agent does not re-derive them:

| file:line | construct |
|-----------|-----------|
| `000117__pyio.py:1962` | `return (None, …)` — a multi-line tuple return; the CST keeps only `None` |
| `000124_pydoc.py:2539` | `time.sleep(.01)` inside `_start_server`; the CST's row on 2538 is a boolean expression |
| `000170_calendar.py:97` | `_months.insert(0, lambda x: "")` — the lexer's lambda body is `m:[x,0]`, the CST's is `n:x` |
| `000219__parser.py:196` | `self.dict: dict[str, Any] = {}` — the lexer's target is the annotation's `dict`, the CST's is `self.dict` (lexer bug, 1 row) |
| `000242_imaplib.py:1510` | `gmtoff = -(time.timezone, time.altzone)[dst]` — the CST's value keeps one name, the lexer's two |
| `000358___init__.py:504` | `memmove = CFUNCTYPE(…)(_memmove_addr)` — a call on a call; the lexer's path is `CFUNCTYPE().call` |
| `000499_request.py:798` | `setattr(self, '%s_open' % type, proxy=url, …)` — keyword arguments are a merge to the CST, `k:` atoms to the lexer |
| `000505_traceback.py:300` | `return (self.filename, …)[pos]` — a subscript of a tuple expression; the CST keeps the first element |

## 7. Rust: `this` is a binding, not `self` (2 rows, counted as events)

`001008_history_update.rs:403`, `let mut this = self.project();` — the lexer's
target is `self`, the CST's is `this`. Both readers map `this` → `self` in
`path_of`/`Expr::name`, and only the CST's `rust_pattern_names` does not, so
fixing one side alone turns a zero into a loss and fixing both is two files for
one row.

## 8. Corpus duplication inflates every number

rust ≈45 %, js ≈7 %, ts ≈23 % of the corpus files are byte-identical copies of
another file (the same module appears two to six times). One unfixed construct
therefore costs two to six rows, and fixing one pays two to six times. The
"twins" the guard pairs (see `fix(guard): a duplicated definition's twin rows are
the file's`) exist for the same reason.

---

# Dead ends (kept so they are not re-tried)

All four were measured against the same corpora; the numbers are the before →
after of that corpus.

1. **Pair by the nearest line instead of the first name match** (`src/pipeline.rs`,
   symbol pairing). Fixes the twin case but re-pairs other twins wrongly: js 142 →
   155 losses, rust files 114 → 117. Reverted. What works is additive: keep the
   pairing and add every same-leaf-name CPS symbol to the comparison pool
   (`a68d479`) — every corpus improved (js 142 → 136, ts 73 → 71, python 11 → 8,
   rust 141 → 126).
2. **Pair an owner-less lexer symbol with an owner-qualified CST symbol**
   (`tests.feed` against `feed`). Same failure shape: js 155, rust files 117.
   Reverted.
3. **Strip a leading `#[…]` in `open_block` / `c_style_head`.** Correct, and it
   *raises* the count: 141 → 199 and 141 → 207. See §4: the guard skips unpaired
   symbols.
4. **Fold a leading-dot call shape only over a call receiver, or only when the
   receiver is not the constant `0`.** js 128 → 510 / 260, rust 112 → 140. See §3.

The earlier dead ends are still accurate and are not repeated here: `P2-02`
(exact-value-first pairing) stays reverted, the struct-literal-vs-block question
is settled by `deep_facts`' `rust_form` arms, and the arrow-function symbol
disagreement is closed by `nested` in the guard.
