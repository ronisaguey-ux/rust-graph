# Unsolved — the log of questions that outlived the work that asked them

This file is the record of a question the corpus asked and the code answered with
a measurement, not with an opinion. A question that is closed keeps its numbers;
a question still open says where the disagreement is counted
(`docs/RUST_CONSTRUCTS.md`) and which module owns it. `docs/PAIRING.md` is the
design behind the guard; this is the history of the shapes that shaped it.

Current state, 2026-11-17: js 0 fallbacks of 1,854 files, ts 2 of 2,000, python
0 of 568, rust 3 of 1,816, measured on the release binary by
`tools/phase4_gate.py` (see `docs/MEASURE.md`). The construct record and the
gate pins carry that same sweep, so `phase4_gate.py` and
`construct_report.py --check` both exit 0 at these numbers. What is left, and
why none of it is a reader fix a single lane should take, is in
`docs/fable-handoff.md`.

## Match arms: a block that belongs to the arm — **closed 2026-11-15**

**The question.** A Rust `match` arm has two readings: the arm's block is a
statement list of its own, or the whole arm is one expression that becomes its
own frame (a function-like node whose body holds the statements). The two readers
disagreed because the CST wrote one and the lexer the other, and nothing said
which was right.

**The reproduction.** `/tmp/r034/000035_line_boundary.rs`, reduced to four lines:
the CST wrote `m c` and `LPAREN` against a lexer row carrying only the call on
the next line. One arm, one block, two readers, one line apart.

**The reading that lost, with its number.** Arms as their own functions: rust
345 → **417** fallbacks. `tools/baselines/arm_merge.json` is the record of the
attempt that followed, and of why it was reverted.

**The reading that won.** The arm body merges its statements on the arm's first
line, exactly as the lexer writes them. The change lives in statement
finalisation and is reached only for Rust's `=>`, so it cannot move the other
three corpora — which is the shape of a fix that is attributable to one change
rather than a bundle.

**Measured, on the full corpus, before it was believed** (`tools/phase4_gate.py
--json`, commit `8412e5a`): rust **308** fallbacks of 1,816 files, js 136, ts 91,
python 14. The arm change was 37 under the 345 the plan allowed at the time.

**What remains of it.** The two readers still stamp different lines for a match
that is an assignment's value versus one that ends a block — the CST folds the
arm block into the `match_expression` value and the lexer only sees the head. The
fold is load-bearing there and dropping it cost seven rust files, so it stays and
the disagreement is documented where it can be read: `docs/PAIRING.md`, "Match
arms: one expression, and the fold that is load-bearing".

**Landed alone, on purpose.** The change is one commit touching one file
(`aa4c019`, `src/deep_facts/rust.rs`, 8 lines added and 2 removed), so a
regression after it is attributable to it and not to a bundle it travelled in.
That is checked rather than promised: `python3 tools/commit_audit.py --check`
reads the history and refuses a pinned merge fix that touched a test, a baseline
or a document, that grew past 40 lines, or that was measured by a commit without
it in history (`tools/baselines/merge_fix.json` holds the pin).

**Closed by** the statement-finalisation change and five coverage rows in
`tests/rust_coverage.rs` ("match arms: block body, guard, call value and
wildcard", plus the guard, or-pattern, binding-at and range-pattern rows from
P5-10).

## A call through `?` or `.await` keeps its receiver — **closed 2026-11-15**

**The question.** `self.server_handle.await.unwrap()` and `file?.as_str()?` put a
postfix operator between the value and the member. Both readers see the call;
they disagree on what the call is *named*. The lexer wrote the receiver's path
(`self.server_handle.unwrap`, `file.as_str`), the CST wrote a path that starts
with a dot (`.unwrap`, `.as_str`) because `path_of` did not look through
`try_expression` and `await_expression` — the two nodes Rust's grammar builds for
`?` and `.await`. A leading dot is the record's spelling for a call with no
name, so the guard read the whole call as one the CST never wrote.

**The reproduction.** Four lines, `/tmp/rgrepro1/a.rs`:

```rust
fn f(x: Option<String>) -> Option<&str> {
    let y = x?.as_str()?;
    Some(y)
}
```

The lexer wrote `c(n:x;[])`, the CST `n:x`: `?` is not a value, so whoever reads
the chain must keep the name on either side of it.

**The reading that lost, with its number.** Leaving the two postfix operators
opaque: rust **48** files and 51 losses of 1,816. The class is wider than the
`?` shape — the same hole lost `.await` receivers, and it is the same arm in
`path_of` that TypeScript's `!` already had.

**The reading that won.** `path_of` unwraps `try_expression` and
`await_expression` the way it already unwraps `parenthesized_expression` and
`non_null_expression`: a postfix operator does not rename the value it is applied
to, so the chain keeps its receiver. One arm in `src/deep_facts/go.rs`; the two
node kinds only Rust's grammar builds, so no other language's path can reach it.

**Measured, on the four corpora, before and after** (`tools/loss_count.py`,
release/musl binary):

| corpus | before | after |
| --- | ---: | ---: |
| js | 5 / 1854 (7 losses) | 5 / 1854 (7) |
| ts | 36 / 2000 (19) | 36 / 2000 (19) |
| python | 8 / 568 (8) | 8 / 568 (8) |
| rust | 48 / 1816 (51) | **36 / 1816 (39)** |

**Closed by** the `path_of` arm above, in the commit that carries this section
(`fix(rust): a ? or .await keeps its receiver`). No test pinned the leading-dot
spelling the CST used to write, so nothing had to be re-pinned for it; the
existing `tests/deep_facts.rs` rows still pass.

**Half of it, on one side only.** `path_of` is the CST. The lexer half of `.await`
was still open when this section was written: the `?` arm of the chain already
skipped its operator, but `.await` was consumed as a *segment*, so the same call
reached the record as `self.server_handle.await.unwrap` on one side of the guard
and `self.server_handle.unwrap` on the other. The `?` shape was answered here;
the `.await` shape only closed on 2026-11-16, in `src/expr.rs` (see "`.await` is
a postfix operator, not a member" below).

## A trait object names its own types — **closed 2026-11-15**

**The question.** `&dyn Fn(ElId, ElId) -> Vec<ElId>` is a type, and a type in a
closure's parameter list reaches both readers as part of the closure's value.
The lexer wrote every name in it — `Fn`, `ElId`, `Vec` — and the CST wrote only
`dyn`: `function_trait`, the node the grammar builds for `Fn(…) -> T`, had no
form, so it formed as the constant and the names inside it left the record. The
file was then blamed for names the CST had dropped.

**The reproduction.** One line, `/tmp/rgrepro1/a.rs`:

```rust
fn f() {
    let stacked = |under: &dyn Fn(ElId, ElId) -> Vec<ElId>| { d() };
}
```

The lexer wrote `m:[n:under,n:dyn,n:Fn.Vec,n:ElId]`, the CST
`m:[n:under,m:[n:dyn]]`.

**The reading that lost, with its number.** Leaving `function_trait` unread:
rust **36** files and 39 losses of 1,816, six of them closures whose parameter is
a `dyn Fn`.

**The reading that won.** A `function_trait` forms as the merge of its children,
the same shape `dynamic_type` already gives its bounds. One arm in
`src/deep_facts/rust.rs`; the node is only ever built by Rust's type grammar, so
no other language can reach it.

**Measured, on the four corpora, before and after** (`tools/loss_count.py`,
release/musl binary):

| corpus | before | after |
| --- | ---: | ---: |
| js | 5 / 1854 (7 losses) | 5 / 1854 (7) |
| ts | 36 / 2000 (19) | 36 / 2000 (19) |
| python | 8 / 568 (8) | 8 / 568 (8) |
| rust | 36 / 1816 (39) | **28 / 1816 (31)** |

**Closed by** the `function_trait` arm, in the commit that carries this section.
The argument list of the trait (`Fn(ElId, ElId)`) still forms as the constant, so
a name that appears *only* in the argument list is not yet in the record; nothing
in the corpus asks for it, and saying so here is cheaper than guessing.

## A function type is not a call — **closed 2026-11-15**

**The question.** `pick: fn(&Corners) -> f64` is a function *type* in a closure's
parameter list. The lexer read the group as a call — `fn` as the callee, `Corners`
as its argument — and then read the `->` as a member separator, so `f64` arrived
as a *name of the value*. The CST wrote the parameter's type name and nothing
else, and the file was blamed for a call and a name the source never makes.

**The reproduction.** `/tmp/rgrepro1/a.rs`, four lines:

```rust
fn f() {
    let corner = |prop: &str, pick: fn(&Corners) -> f64| {
        prop.len()
    };
}
```

The lexer wrote `t:[m:[n:prop,n:str],m:[n:pick,c(0;[n:Corners])]]`, the CST
`m:[n:prop,n:str,n:pick,n:Corners]`.

**The reading that lost, with its number.** `fn` as a callee: rust **28** files
and 31 losses of 1,816. `rust_type_group` excused `dyn Fn(..)` and `impl Fn(..)`
by the word two tokens before the group; for `fn(..)` the keyword is the token
*in front* of it, so the same reading was never applied.

**The reading that won.** `fn` followed by `(` opens a type in the value reader:
the parameter list is consumed and the `-> T` after it is consumed as a type,
with the two words written to the record only when they are names — which they
are not. One branch in `src/expr.rs`, reached only for Rust's `fn`.

**Measured, on the four corpora, before and after** (`tools/loss_count.py`,
release/musl binary):

| corpus | before | after |
| --- | ---: | ---: |
| js | 5 / 1854 (7 losses) | 5 / 1854 (7) |
| ts | 36 / 2000 (19) | 36 / 2000 (19) |
| python | 8 / 568 (8) | 8 / 568 (8) |
| rust | 28 / 1816 (31) | **27 / 1816 (30)** |

**Closed by** the `fn` branch, in the commit that carries this section. It clears
one file: the `fn`-typed parameter is rare in the corpus, and the six closures
whose parameter is a `dyn Fn(..)` were cleared by the `function_trait` arm above.

## A name has one spelling inside a macro too — **closed 2026-11-15**

**The question.** A closure parameter named `this` — legal Rust, since `this` is
not a keyword — is spelled `self` by the record: the one-pass reader renames any
`this` to `self`, and so does the CST's `path_of`. Inside a macro's argument list
the CST kept the source's spelling instead, so `this.accent(…)` reached the
record as `this` while the same closure's parameter reached it as `self`.

**The reproduction.** `/tmp/rgrepro1/a.rs`:

```rust
fn f() {
    let render = |this: &Prompt| {
        let a = format!("{} {}", this.accent("x"), this.bold(m));
        a
    };
}
```

The lexer wrote `m:[n:format,t:[0,c(n:self;[0]),c(n:self;[n:m])]]`, the CST
`m:[n:format,m:[0,c(n:this;[0]),c(n:this;[n:m])]]`.

**The reading that lost, with its number.** The token-tree path builder as the one
place that keeps the source's spelling: rust **27** files and 30 losses of 1,816,
three of them in the one file that names a closure parameter `this`.

**The reading that won.** A token-tree path is spelled like every other path in
the same reader: `rust_tt_path` writes `self` for the word `this`, the way
`path_of` and the shared form arm already do. One branch in
`src/deep_facts/rust.rs`.

**Measured, on the four corpora, before and after** (`tools/loss_count.py`,
release/musl binary):

| corpus | before | after |
| --- | ---: | ---: |
| js | 5 / 1854 (7 losses) | 5 / 1854 (7) |
| ts | 36 / 2000 (19) | 36 / 2000 (19) |
| python | 8 / 568 (8) | 8 / 568 (8) |
| rust | 27 / 1816 (30) | **26 / 1816 (27)** |

**Closed by** the `rust_tt_path` spelling, in the commit that carries this
section. Two spellings of one name cannot be told apart from two names, so the
rename belongs in the reader that renames, not in the guard.

## A turbofish closes where its angle brackets do — **closed 2026-11-16**

**The question.** `f(a, None::<Vec<Entry<T>>>, g(b))`. A turbofish names a type,
not a value, so the value reader skips it. The tokenizer reads `>>` as one
operator, and the skip counted only a lone `>`, so the list never closed and the
skip ran to the end of the statement — every later argument and call went with
it. `f::<<T as Tr>::Out>` is the same hole pointing the other way: one token opens
two lists.

**Measured** (`tools/loss_count.py`, release/musl binary): rust 23 / 1,816 files
(24 losses) → **22 / 1,816 (23)**. js 2 / 1854 (4) and ts 34 / 2000 (19) did not
move.

**Closed by** `d4242ab`, one block in `src/expr.rs`: the skip is a depth counter
that reads `<<` as two openers and `>>` as two closers, and a turbofish stamps its
call on the line of its `::` when the parenthesis is written further down.

## `.await` is a postfix operator, not a member — **closed 2026-11-16**

**The question.** `start.await.status`. The chain keeps its receiver across `?`
(see above), but `.await` was still consumed as a *segment*, so the lexer named
`start.await.status` where the CST, reading through an `await_expression`, names
`start.status`. A member the source never writes is a name the guard blames on
the wrong reader.

**Measured**: rust 22 / 1,816 (23 losses) → **21 / 1,816 (22)**. js and ts
unchanged.

**Closed by** `6d4fcc7`, `src/expr.rs`: `.await` is skipped beside `?` for Rust
only, and no segment is pushed for it.

## An awaited target is not named `.await.inner` — **closed 2026-11-16**

**The question.** `self.rcvs.lock().await.inner = ..`. The target reader
(`target_path`, `src/parser.rs`) joined every token it walked, so the *place*
being assigned was spelled `.await.inner` — a path the CST never writes, since the
CST names the target `.inner` through its `await_expression`.

**Measured**: rust 21 / 1,816 (22 losses) → **20 / 1,816 (21)**. js and ts
unchanged.

**Closed by** `1535e3c`, `src/parser.rs`: the target walk steps over `.await` the
way it already steps over a subscript.

## A let condition is stamped where it starts — **closed 2026-11-16**

**The question.** `if let Some(x) =` followed by a value whose first token is
several lines down. The condition event was stamped on the value's first token,
while the CST stamps its `let_condition` on the `let`; one condition became two
events, one line apart, that no line could pair.

**Measured**: rust 20 / 1,816 (21 losses) → **18 / 1,816 (19)**. js and ts
unchanged.

**Closed by** `87b7722`, `src/parser.rs`: `expression_event_at` takes the line its
caller names, and a stripped let condition is stamped on its `if`/`while` token.
`expression_event` keeps the old spelling for every other caller.

## A brace inside a let pattern is not the body — **closed 2026-11-16**

**The question.** `if let Self::Enabled {` newline `core,` newline `..` newline
`} = self`. That brace lists the fields of a pattern; opened as the body, the
fields became statements of their own and the condition lost its value to the
line below them. A `let` in a condition is followed by a pattern until its
binding `=`.

**Measured**: rust 18 / 1,816 (19 losses) → **16 / 1,816 (15)**. js and ts
unchanged.

**Closed by** `4e5cd32`, `src/parser.rs`: `rust_let_pattern_open` answers whether
the statement so far ends inside a condition's pattern (from the last `let` that
follows `if`/`while`/`&&`/`||` to a top-level `=`), and `if`/`while`/`else` brace
openers consult it before taking the brace as a body.

## Applying a constructed value is an anonymous call — **closed 2026-11-16**

**The question.** `CFUNCTYPE(c_void_p, c_size_t)(addr)`. The value the
constructor built is then applied, and an application has no name — exactly like
`f(x)(y)`. The reader glued the class to the call anyway, so the record named
`CFUNCTYPE().call`, a method no source spells and one the resolver would tie to
any method named `call`.

**Measured**: python 8 / 568 (8 losses) → **7 / 568 (7)**; rust stayed at
16 / 1,816 (15), js and ts unchanged.

**Closed by** `0c1a926`, `src/expr.rs`: when the callee path is empty the class
that built the value is not glued to the target. A member called on that same
value still carries its class (`Repo(db).find` stays `Repo().find`).

## Still open

The codes, counts and owning modules are regenerated into
`docs/RUST_CONSTRUCTS.md` and `docs/READERS.md` from
`tools/baselines/constructs.json`; this file does not repeat them, so it cannot
go stale beside them. The open questions behind those codes are, in the order
they cost the most rust files (rust rows of the 3 fallbacks measured 2026-11-17,
in parentheses; a question with no live rust row keeps the row count it had):

1. **The CST has no node for a call the lexer read**
   (`CST_MISSING_KIND:call`, 2 rows): a grammar gap, so `src/deep/` and the
   grammar tables, not the guard. Both rows are the same file twice over: the
   source repeats a `fn` header line, so the whole block is in recovery and the
   call inside it has no node to be read from.
2. **Two events on one line that cannot be paired** (`PAIR_AMBIGUOUS:1`, no rust
   row today, one ts row): the classifier decides the file; the readers have to
   agree first.
3. **The lexer invents a name the CST never writes** (`LEX_OVER_EMIT:name`, 1
   rust row and 1 ts row): the value reader's merges. The fix belongs in
   `src/expr.rs`, where both readers' value shapes are decided for every
   language. The rust row is a `match` keyword inside a `json!` token tree; the
   ts row is an object literal the CST read as four empty parts.
4. **Grammar recovery over budget** (`CST_DEGRADED:recovery`, no rust row
   today): error nodes are the CVD parser's recovery, not a reader
   disagreement, and the threshold is the only knob that is honest here.
5. **The lexer invents a callee** (`LEX_OVER_EMIT:call`, no rust row today):
   call detection in the same reader; a pattern, a type or a range read as a
   call.
6. **The two readers disagree about a call's targets**
   (`CST_SHAPE_MISMATCH:targets`, no rust row today): the bucket is named when
   the reason reads "target list differs" (`reason_code` in `src/pipeline.rs`),
   and it is a reader/guard agreement, not a grammar gap — the CST writes one
   target list, the lexer another, and neither is wrong on its own.
7. **The CST has no node for a symbol the lexer read**
   (`CST_MISSING_KIND:symbol`, no rust row today): the generic "the CST lost a
   symbol" reason, so the owning module is the language's table and walk arm
   (`src/deep_facts/<lang>.rs` and its `src/deep/` kinds), never the guard.

Entries 6 and 7 were missing from this list while the corpora paid for them:
they are the two codes that `reason_code` produces with no question written
here, which P8-08 cannot close until each fallback is attributed. Neither costs a
rust file any more — the rust corpus moved under them while the C and Go tables
below kept them near the top, which is the reason the rust order above is not the
order the C family pays in.

This list is ordered by **rust** cost, and the C family does not pay in that
order. Measured 2026-11-16 with both readers forced on (C 554 fallbacks of
1,849 files, Go 5,828 of 12,004). Re-measured 2026-11-17 through
`tools/auto_audit.py --lang c|go`, which sets `RG_AUDIT=1` itself — no worktree,
no `lang_for` flip: C 295 of 2,010 files (544 losses, 137 of them
`CST_DEGRADED:recovery`), Go 7,015 of 15,612 (12,133 losses). The two sweeps are
different corpus builds, so the counts are not comparable with each other; the
table below is the 2026-11-16 one.

| files | code | C | Go |
|---:|---|---:|---:|
| 1610 | `CST_MISSING_KIND:call` | 195 | 1415 |
| 1030 | `LEX_OVER_EMIT:name` | 71 | 959 |
| 950 | `CST_MISSING_KIND:symbol` | 9 | 941 |
| 879 | `LEX_OVER_EMIT:call` | 21 | 858 |
| 833 | `CST_SHAPE_MISMATCH:targets` | 5 | 828 |
| 688 | `PAIR_AMBIGUOUS:1` | 60 | 628 |
| 174 | `CST_DEGRADED:recovery` | 128 | 46 |
| 73 | `CST_MISSING_KIND:constant` | 51 | 22 |

`CST_DEGRADED:recovery` is 128 of C's 554 in that sweep, and 137 of the 295
measured on 2026-11-17 — grammar recovery, not a reader disagreement, and no
current target covers it. Reading the table in rust order would spend the next
pass on the wrong two codes.

## 2026-11-23 — Neon pass: 15 Kinds tables, member/scope paths, and targeted reader fixes

Measured on `corpus45` with `RG_AUDIT=1 RG_LOSSES=1` (`x86_64-unknown-linux-musl`, `--features "deep parser"`, `--no-cache`):

- **15 empty-table languages wired in `src/deep_facts/mod.rs`**: Dart, Haskell, Solidity, Ruby, PHP, Lua, Crystal, F#, Tcl, R, PowerShell, Zig, Awk, Pascal, and Shell/Bash gained `Kinds` tables (`definitions`, `calls`, `imports`) and statement-level `enter` arms, paired with unified `generic_path_import`, `hs_import`, and `lua_local_declaration` in `src/deep_facts/go.rs`.
- **Shared `member`/`scope` path resolution (`src/deep_facts/go.rs`)**: added `constant`, `name`, `variable_name`, `self`, `parent` leaves and `member`, `safe_member`, `scope`, `member_access_expression`, `scoped_property_access_expression` to `path_of`, `form`, `member_name`, and `assignment_targets` — dropping call/event losses simultaneously across Crystal, Solidity, PHP, Ruby, Lua, and R.
- **Targeted fixes**: Java `for_header` (classic `variable_declarator` + enhanced-for `for (E e : iter)`), F# `#if`/`#nowarn` preprocessor comments (`batch_f/fsharp.rs`) and `open` modifier skip (`src/parser.rs`), PHP closure `use (...)` (`src/parser.rs`), Shell non-assignment arguments (`src/parser.rs`), C/C++/ObjC goto labels (`src/parser.rs`), Java `my`/`local`/`our`/`let` noise skip (`src/expr.rs`), Crystal `lib`/`fun`/`alias`, Solidity `pragma`/`mapping`/`returns`/`state_variable`, Awk `BEGIN`/`END`/`print`/`pattern_rule`, Zig `var_declaration`/`builtin_call`.
- **Reverted per gate rule**:
  1. `in_command_call` guard in `src/expr.rs` (`juxtaposed`): raised Tcl `684 -> 817` files; reverted.
  2. `mapping_body` named-parameter rule in `crates/deep-grammars/src/batch_d/solidity.rs`: raised Solidity `159 -> 165` files; reverted.

| language | before (files/rows) | after (files/rows) | delta files | status |
|---|---:|---:|---:|---|
| java | 177 / 243 | 93 / 142 | -84 | PASS (<120) |
| fsharp | 134 / 134 | 114 / 114 | -20 | PASS (<120) |
| crystal | 212 / 213 | 118 / 141 | -94 | PASS (<120) |
| dart | 1278 / 1278 | 694 / 1248 | -584 | open |
| php | 1258 / 1636 | 718 / 4525 | -540 | open |
| awk | 488 / 1087 | 308 / 1049 | -180 | open |
| ruby | 1566 / 1603 | 1408 / 3320 | -158 | open |
| pascal | 829 / 8075 | 779 / 8075 | -50 | open |
| bash | 476 / 1010 | 436 / 468 | -40 | open |
| solidity | 178 / 178 | 142 / 202 | -36 | open |
| r | 656 / 664 | 622 / 7194 | -34 | open |
| haskell | 519 / 520 | 507 / 508 | -12 | open |
| zig | 205 / 1182 | 198 / 918 | -7 | open |
| powershell | 266 / 267 | 262 / 263 | -4 | open |

Cross-language regression check on the 4 original corpora: **0 movement** (`python` 23/25, `javascript` 23/23, `typescript` 28/36, `rust` 26/27). Self-parse of `rust-graph/src`: **0 fallbacks**.

### Round 2 additions (2026-11-23)

- **Brace-context detector (`src/parser.rs`)**: added `BraceContext` (`Block | StructLiteral | TableConstructor`) behind a fast-path token check with unit test `brace_context_keeps_braced_initialiser_one_statement` (`256 passed, 0 failed`). Measured on full `corpus45/go`: `436/806 -> 432/514` (`-4` files, `-292` rows).
- **Ruby (`src/deep_facts/go.rs`)**: direct-child argument collection (including `label_argument` and `simple_symbol`) for paren-less `call`/`command_call` -> `1408/3320 -> 1401/3183` (`-7` files, `-137` rows).
- **Lua (`src/deep_facts/go.rs`)**: `table_constructor` (`named_field`, `keyed_field`) + `local_declaration` assignment event emission -> `569/1083 -> 559/837` (`-10` files, `-246` rows).
- **Objective-C (`src/deep_facts/c_family.rs`)**: allow whitespace between `#` and `include`/`import` in `c_includes` -> `265/376 -> 246/358` (`-19` files, `-18` rows, `import` losses `29 -> 0`).
- **C# (`src/lexer.rs`)**: consume C# 11 `u8`/`U8` UTF-8 string literal suffix -> `532/848 -> 529/869` (`-3` files, `call` losses `128 -> 98`).
- **Erlang (`src/parser.rs`)**: recognize single-quoted atom function heads (`'name'(Args) ->`) -> `1268/1357 -> 1266/1355` (`-2` files, `-2` rows).
- **Solidity (`src/deep_facts/mod.rs`)**: `using_directive` / `event_definition` / `error_definition` statement emission -> `142/202 -> 141/189` (`-1` file, `-13` rows).
- **Tcl (`src/deep_facts/mod.rs`)**: bare word/subcommand emission on `call_command` -> `684/684 -> 684/743` (`call` `681 -> 676`, zero crashes).
- **Self-parse & regression**: `rust-graph/src` `0` losses; 4 original corpora (`python` 23/25, `javascript` 23/23, `typescript` 28/36, `rust` 26/27) unchanged.
