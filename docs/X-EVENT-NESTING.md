# X-Event Nesting Invariant and Unmasking Law

Both readers (`src/expr.rs` on the token stream and `src/deep_facts/` on the CST) write flow events into `Symbol::events` and call rows into `Symbol::calls`. `src/pipeline.rs::loses_in` gates every file in five ordered phases before allowing the CST record to replace the lexer record. Because the first four phases exit on their first miss while the fifth reports every uncovered event on a symbol, fixing a `call` disagreement without matching the x-event nesting invariant causes fallback file counts to drop while total loss rows multiply.

---

## 1. Phase-Masking and the Row-Delta Prediction Law

`pipeline::loses_in` (`src/pipeline.rs:1596-1867`) runs in strict order:

| Phase | Fact kind | Exit rule | Reported `losses` on failure |
|---:|---|---|---:|
| 0 | `import` | Returns at the first missing import | `1` |
| 1 | `symbol` | Returns at the first missing named symbol | `1` |
| 2 | `call` | Returns at the first missing non-keyword, non-positional call | `1` |
| 3 | `var` | Returns at the first under-counted typed binding `(name, class)` | `1` |
| 4 | `event` | Runs `uncovered()` per paired symbol; returns on the first symbol with misses | `details.len()` ($k(F) \ge 1$) |

Let $U$ be the set of files whose last Phase 0–3 loss is resolved by an attempt, and let $k'(F)$ be the number of uncovered flow events in file $F \in U$ after the change (assuming no previously passing file regresses):

- Each file $F \in U$ with $k'(F) = 0$ moves from `1 file / 1 row` to `0 files / 0 rows` ($\Delta\text{files} = -1,\; \Delta\text{rows} = -1$).
- Each file $F \in U$ with $k'(F) = 1$ moves from `1 file / 1 row` to `1 file / 1 row` ($\Delta\text{files} = 0,\; \Delta\text{rows} = 0$).
- Each file $F \in U$ with $k'(F) \ge 2$ stays a fallback file and adds $k'(F) - 1$ net rows.

Summing over $U$ gives the exact corpus delta:

$$\Delta\text{files} = -|\{F \in U : k'(F) = 0\}|$$

$$\Delta\text{rows} = -|\{F \in U : k'(F) = 0\}| + \sum_{F \in U,\, k'(F) \ge 2} \bigl(k'(F) - 1\bigr)$$

**Pre-attempt prediction rule:** Before unmasking a `call` bucket on `<corpus>`, dump both records (`$BIN build <corpus> --stdout --lexer --no-cache` and `--deep`) and evaluate $k'(F)$ on every file in $U$. An attempt will keep $\Delta\text{rows} \le 0$ if and only if $\sum_{k'(F) \ge 2} (k'(F) - 1) \le |\{F \in U : k'(F) = 0\}|$. In particular, any CST call emitter or lexer call suppression must leave zero residual x-event mismatch on the edited lines themselves ($k'_{\text{same-line}}(F) = 0$).

---

## 2. Value Constructors and Call-Table Coupling (`src/expr.rs`)

`Emit` (`src/expr.rs:1-10`) encodes every expression into six JSON forms:

- `0` — constant or opaque literal (uninterpolated string, number, JS arrow argument).
- `["n","a.b"]` — identifier or member path.
- `["m",[e1,e2,...]]` — combined values (operators, string interpolations, braced maps/blocks, adjacent shell words).
- `["t",[e1,e2,...]]` — tuple or bracketed list, element by element.
- `["c",id,recv,[arg1,...]]` — call expression; `id` is the 0-based index in the enclosing symbol's `calls` table, `recv` is `0` or a receiver expression, and `[arg1,...]` are the argument values.
- `["k","name",e]` — named/keyword argument inside a call's argument list (`=` in Python/R/etc., `:` in `RB`, `CR`, `SWIFT`, `DART`, `EX`, and C# attributes).

**Coupling invariants:**
1. **One `["c",...]` per `Call` row:** Every `Emit::new_call(path, line)` pushes one entry to `Symbol::calls` and writes one `["c",id,recv,[args]]` node. If a match-arm pattern before `=>` or `->` is truncated (`src/expr.rs:721-735`), `self.calls` is truncated back to `calls_at_start` in lockstep.
2. **No duplicate callee operand:** A bare or qualified callee consumed into `["c",id,...]` is truncated out of `out` (`src/expr.rs:2905-2908`, `3319`) and never appears a second time as a sibling `["n",path]` beside its own call.

---

## 3. The Six Call-Nesting Shapes Across All 45 Languages

`src/langs.rs` partitions the 45 `corpus45` languages into six call-emission regimes in `src/expr.rs`:

### 3.1 Parenthesized and Receiver-Chained Calls (30 languages)
Languages: `awk`, `c`, `cpp`, `csharp`, `d`, `dart`, `elixir`, `erlang`, `go`, `groovy`, `java`, `javascript`, `julia`, `kotlin`, `lua`, `matlab`, `nim`, `pascal`, `php`, `python`, `r`, `rust`, `scala`, `solidity`, `sql`, `swift`, `typescript`, `zig` (plus parenthesized calls in `fortran`, `vbnet`, `visual-basic`, `cobol`, and `objective-c`).

- **Nested call in argument position (`f(g(x), y)`):** Inner calls sit inside the outer call's `[args]` array; outer `id` is allocated before inner `id`:
  ```json
  ["c",0,0,[["c",1,0,[["n","x"]]],["n","y"]]]
  ```
  with `calls = ["f", "g"]`.
- **Chained member call (`a.b(x).c(y)`):** `Emit::chain` (`src/expr.rs:3252-3322`) folds left-to-right, placing each completed call into the **receiver slot** of the next member call:
  ```json
  ["c",1,["c",0,["n","a"],[["n","x"]]],[["n","y"]]]
  ```
  with `calls = ["a.b", ".c"]` (or `["Repo", "Repo().find"]` when the first callee starts with an uppercase letter).
- **Direct call application (`f(x)(y)`):** `calls = ["f", ".call"]` and `["c",1,["c",0,0,[["n","x"]]],[["n","y"]]]`, except in `JS`/`TS`/`JSX`/`TSX` where `f()()` keeps only the inner `["c",0,...]` (`src/expr.rs:3256-3264`).

### 3.2 Command-Call Right-Nesting (`command_calls: true` — 6 languages)
Languages: `bash` (`SH`), `crystal` (`CR`), `perl` (`PL`), `powershell` (`PS1`), `ruby` (`RB`), `tcl` (`TCL`).

When `Emit::ident` finishes `chain` on `line` (`src/expr.rs:2658-2660`), `Emit::juxtaposed` (`src/expr.rs:2888-2941`) inspects the next token `next`:
- If `next` is on the **same `line`**, is a value token (`Kind::Ident | Kind::Str | Kind::Num`), and is not in the language's `NOISE` list (and for `SH`, is not a `<<` heredoc, `[0-9]+[<>]` FD redirect, `for`/`select` list item, digit-sigil `$1`, or `/`/`:`-joined composite path), `juxtaposed` rewrites `["n",path]` into `["c",id,0,[args]]` by calling `self.list(&mut args, close, true)`.
- Because `list` invokes `argument -> mix -> ident -> juxtaposed` **recursively**, a run of bare words `w0 w1 x` on one line **right-nests** one `["c",...]` per word whose successor is a value token:
  ```json
  ["c",0,0,[["c",1,0,[["n","x"]]]]]
  ```
  with `calls = ["w0", "w1"]`.
- **Operator / flag break rule:** Any `Kind::Op` token (including `-` in `-flag`, `|`, `;`, `(`, `[`, `{`, `>`, `<`) has `is_value == false` for the word **before** it, leaving that word as `["n",w]` inside `["m",[...]]`; the identifier *after* `-` (`flag` in `-flag arg`) starts a new right-nested `["c",id,0,[...]]` if followed on the same line by a value token.

### 3.3 Curried Juxtaposition (`apply_calls: true` — 4 languages)
Languages: `elm` (`ELM`), `fsharp` (`FS`), `haskell` (`HS`), `ocaml` (`ML`).

`Emit::juxtaposed` (`src/expr.rs:2910-2936`) loops **flatly** over adjacent atom tokens (`Ident | Str | Num | '('`) without recursing into `juxtaposed` on `Ident` arguments; only parenthesized sub-expressions `(g x)` recurse through `Emit::group`:
- `f x y` -> `["c",0,0,[["n","x"],["n","y"]]]` with `calls = ["f"]` (one flat call, never right-nested).
- `f (g x) y` -> `["c",0,0,[["c",1,0,[["n","x"]]],["n","y"]]]` with `calls = ["f", "g"]`.

### 3.4 S-Expression Prefix Calls (`prefix_calls: true` — 1 language)
Language: `clojure` (`CLJ`).

`Emit::group` (`src/expr.rs:1520-1539`) converts every `(head arg1 arg2)` whose first token is `Kind::Ident` (joining `/` and `.` into `ns.name`) into `["c",id,0,[args]]`, nesting at each parenthesized sublist:
- `(f (g x) y)` -> `["c",0,0,[["c",1,0,[["n","x"]]],["n","y"]]]` with `calls = ["f", "g"]`.

### 3.5 Keyword-Introduced Calls (`call_words` — 3 language rows, 4 corpora)
Languages: `cobol` (`COBOL`: `"perform"`, `"call"`), `fortran` (`F90`: `"call"`), `vbnet` and `visual-basic` (`VB`: `"call"`).

`Emit::ident` (`src/expr.rs:2010-2022`): when `word` is in `lang.call_words` and the callee identifier is **not** followed by `(`, consumes both words and reads the rest of the statement via `list(&mut args, close, false)` into `["c",id,0,[args]]` (lowercase callee due to `case_insensitive: true`). When followed by `(` (`CALL f(g(x))`), `ident` consumes `call` and lets `f(g(x))` parse through standard parenthesized `chain()`.

### 3.6 Objective-C Bracketed Message Sends (`OBJC` — 1 language)
Language: `objective-c` (`OBJC`).

`Emit::objc_message` (`src/expr.rs:1560-1684`) reads `[recv part1:a part2:b]` as `["c",id,receiver,[a,b]]` with selector `"part1:part2:"`, recursing via `token_range_value` on both `receiver` and each colon-delimited argument (`[[x inner] outer:y]` nests the inner message in the outer message's `receiver` slot).

*(The 45th `corpus45` directory, `assembly`, has `langs::by_content(...) == None` and produces zero symbols, calls, and events.)*

---

## 4. Guard Coverage Algebra (`src/pipeline.rs::atoms` & `covers_value`)

`atoms()` (`src/pipeline.rs:3646-3707`) flattens a derived `Shape` into an ordered multiset of atoms:
- `c(0;[])` (empty receiver **and** empty arguments) pushes **zero** atoms (neither `"call"` nor `"0"`).
- Any `c(recv;[args])` with `recv != "0"` or non-empty `args` pushes `"call"`, then `atoms(recv)` if `recv != "0"`, then `atoms(arg)` for each argument.
- `m:[...]` and `t:[...]` flatten associatively into their children's atoms (`m:[a,m:[b,c]]` and `m:[a,b,c]` yield identical atoms).
- `k(name):val` pushes `"k:name"` followed by `atoms(val)`.

`covers_value(deep, lexer)` (`src/pipeline.rs:3463-3598`) accepts a CST event value against a lexer event value if and only if:
1. `lexer.value == "0"`, or `deep.value != "0"` and:
2. Every `n:<path>` (up to dot-prefix match `a` vs `a.b`) and `k:<name>` in `wanted` appears in `found` (except non-`k:` names when `lexer` is `m`/`t` and `deep` is `c`), and
3. `count(found, "call") >= count(wanted, "call")` and (unless `found.len() >= wanted.len()`) `count(found, "0") >= count(wanted, "0")`.

Consequently, whenever the lexer emits $m$ nested non-empty `["c",...]` nodes on a statement (such as a right-nested command call `w0 w1 x` with $m = 2$), the CST emitter on that statement must produce at least $m$ non-empty `["c",...]` nodes carrying all `n:`/`k:` atoms, or Phase 4 rejects the line with `LEX_OVER_EMIT:call`.
