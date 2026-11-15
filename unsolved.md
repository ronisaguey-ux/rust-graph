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

## 5. TypeScript: a `.d.ts` declaration is not a runtime call — 4 of 43

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

## 7. Rust: `this` is a binding, not `self` (1–2 rows)

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
