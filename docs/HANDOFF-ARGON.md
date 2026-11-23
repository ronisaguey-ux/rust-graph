# Argon handoff: the parse budget (P6-13 → P6-15)

P6-13 sets the line at 3 ms per file at p95. This file is where a language that cannot reach the line
goes instead of failing quietly: for each one, the construct that blocks it, the delta it needs, what
has already been tried, and the code the next attempt has to touch. P8-10 completes the same document
with the snippet and reason string per blocked item.

## The finding in one paragraph

The line is not missed because of a construct. It is missed because of a constant: a file's parse
costs **0.05-0.12 µs/byte** in every language, so a language's p95 is its corpus's largest files
times that rate. `awk`'s five slowest files are its five biggest (273 KB at 0.056 µs/byte, 258 KB of
`.awk` at 0.057); `erlang`'s p95 file is `stdlib/src/ets.erl`, 133 KB at 0.116 µs/byte, parsed by the
one-pass lexer (1 of 4,335 files took the deep reader, so the tail is not the fallback path). Any
language whose corpus holds a 100-300 KB file is therefore over 3 ms, and the fix is one global
constant factor, not 26 per-language patches.

Where the constant goes, sampled with `RG_SAMPLE=1` on `corpus45/erlang` (4,335 files, 423 MB, 15 s;
see `docs/corpus45-budget.md` for the method and the table): lexer 29 %, the value walk
(`chain` + `mix` + `lexeme` + `ident`) 56 %, statement bookkeeping 11 %, `parse` body 6 % of the
parse. Allocation traffic is one block per token (53.2 M blocks, 4.4 GB in that run), which is
`lexeme` — a fresh `String` per word with a UTF-8 validation pass — plus `Call.path`.

So the handoff is not "fix language X". It is "take 30-40 % out of the value walk and the lexer",
and the languages below fall in order of how far over they are.

## What has already been tried, with its measured delta

| attempt | delta |
|---|---|
| guard stops building an event pool when `wanted` is empty (`d2a242790`) | jQuery default path 2,070 → 476 ms; guard phase 1.9 s → 0.24 s; RSS 22 → 7 MB |
| `add_symbol` key index instead of rescanning (`1a2a06324`) | 4k/8k/16k erlang functions 107/324/883 → 46/63/138 ms |
| OTP `-doc """…"""` as one token (`e6bc506fa`) | symbols 208,196 → 217,066; files with no symbols 100 → 8; p95 7.7 → 7.0 ms |
| `with_emit` moves the frame receiver instead of cloning (`7c4ef03e1`) | neutral within noise |
| deep Erlang reader (`42d5157ea`, `50f843c54`) | erlang losing files 1,819 → 1,336; rows 1,851 → 1,435 — a coverage win, no budget win |
| `event_value` (no `format!`, no intermediate `String`) | measured OLD vs NEW min-of-four: 16k atoms 16/16 ms, 16k numbers 7/8, `ets.erl` 10/10, 8k calls 32/36 — no win, reverted |

Two things this rules out: the event write (not the cost), and the guard/fallback path (not in the
tail). Everything left is the token-at-a-time value walk and the lexer itself.

## The entries

`p95` is the quiet-box number from `docs/corpus45-budget.md`; `delta` is the factor the language
needs (p95 ÷ 3 ms). Every entry shares the same reason string — `size × base rate, no pathological
construct` — except where a construct is named.

| language | p95 ms | delta | blocking construct | code an attempt must touch |
|---|---:|---:|---|---|
| d | 20.3 | 6.8× | size × base rate | `src/lexer.rs` (`next_token`), `src/expr.rs` (`chain`, `mix`) |
| fsharp | 18.3 | 6.1× | size × base rate | as above; fsharp's `|>` pipelines go through `mix` |
| zig | 15.9 | 5.3× | size × base rate | `src/expr.rs` (`lexeme` per word) |
| vbnet | 14.9 | 5.0× | size × base rate | `src/parser.rs` (`end_statement`), `src/expr.rs` |
| perl | 9.7 | 3.2× | size × base rate; sigils make every word a `lexeme` call | `src/expr.rs` (`lexeme`, `ident`) |
| julia | 8.6 | 2.9× | size × base rate | `src/expr.rs` (`chain`) |
| rust | 8.2 | 2.7× | size × base rate | `src/expr.rs` (`mix`, macro path), `src/parser.rs` |
| awk | 8.2 | 2.7× | size × base rate (273 KB at 0.056 µs/byte) | `src/expr.rs` (`lexeme`) |
| elixir | 8.1 | 2.7× | size × base rate; `do/end` blocks are long statements | `src/parser.rs` (`end_statement` scans) |
| swift | 7.9 | 2.6× | size × base rate | `src/expr.rs` (`chain`) |
| sql | 7.9 | 2.6× | size × base rate; keyword-dense | `src/lexer.rs` (classification) |
| go | 7.7 | 2.6× | size × base rate | `src/expr.rs` (`mix`) |
| erlang | 7.4 | 2.5× | size × base rate (`ets.erl` 133 KB) | `src/expr.rs`, `src/lexer.rs` |
| nim | 7.4 | 2.5× | size × base rate | `src/expr.rs` |
| python | 7.1 | 2.4× | size × base rate | `src/expr.rs` (`chain`, `lexeme`) |
| crystal | 6.6 | 2.2× | size × base rate | `src/expr.rs` |
| dart | 6.1 | 2.0× | size × base rate | `src/expr.rs` |
| objective-c | 6.1 | 2.0× | size × base rate | `src/expr.rs` (`[obj msg]` chains) |
| clojure | 5.9 | 2.0× | size × base rate; one 12k-token top-level form | `src/expr.rs` (`list`) |
| javascript | 5.9 | 2.0× | size × base rate | `src/expr.rs` (`chain`, `lexeme`) |
| cpp | 5.5 | 1.8× | size × base rate | `src/parser.rs` (`c_declaration`), `src/expr.rs` |
| csharp | 5.4 | 1.8× | size × base rate | `src/expr.rs` |
| pascal | 5.0 | 1.7× | size × base rate | `src/expr.rs` |
| java | 4.1 | 1.4× | size × base rate | `src/expr.rs` (`ident`) |
| scala | 3.7 | 1.2× | size × base rate | `src/expr.rs` (`chain`) |
| lua | 3.1 | 1.03× | size × base rate | `src/expr.rs` (`lexeme`) |

The last four are the cheapest to close and the first to fall when the value walk gets faster: `lua`
needs 3 %, `scala` 20 %, `java` 27 %. `d` and `fsharp` are not closable by a constant-factor pass and
are the two that need the reader redesigned (a byte-classified lexer and a non-recursive `mix`).

## Definition of done for whoever picks this up

1. Take `lexeme` off the allocation path (a borrowed `&str` through the value walk, or a scratch
   buffer in `Emit`), and measure it with the sampler — it is 14 % of the parse and one allocation
   per word.
2. Classify bytes once in `next_token` (a 256-entry table instead of match chains) and measure the
   29 %.
3. Re-run the per-language table on a quiet box (`docs/MEASURE.md`), tick the languages that fall
   under 3 ms, and delete their rows here.

No language is left "failing quietly": each row above is a measured number, not an estimate.
