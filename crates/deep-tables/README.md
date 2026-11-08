# deep-tables

LALR(1) table generation and table compression for the deep scan. Standard
library only; the sole dependency is `deep-abi`, whose frozen `Grammar<'a>` is
exactly what this crate produces.

```rust
use deep_tables::{Associativity, GrammarBuilder};

let grammar = GrammarBuilder::new()
    .name("expr")
    .terminal("NUM")
    .terminal("PLUS")
    .precedence(1, Associativity::Left, &["PLUS"])
    .rule("e", &["e", "PLUS", "e"])
    .rule("e", &["NUM"])
    .build()?;

let flat = grammar.as_grammar(); // deep_abi::Grammar<'_>
```

## Pipeline

| module | job |
| --- | --- |
| `lib.rs` | `GrammarBuilder`, symbol numbering, the `Compiled` façade |
| `first.rs` | nullability, FIRST and FOLLOW by fixpoint |
| `lr0.rs` | canonical LR(0) item sets, closure, goto |
| `lalr.rs` | LALR(1) lookaheads |
| `table.rs` | actions, gotos, precedence resolution, default reductions |
| `compress.rs` | row displacement, u16/u8 narrowing, the flat ABI tables |
| `report.rs` | byte-level size accounting |
| `grammars.rs` | the JSON, Python-subset and expression grammars used in tests |

### Which LALR method

**Spontaneous generation plus lookahead propagation** (Dragon book Algorithm
4.63), not DeRemer/Pennello and not the lane table. A dummy terminal `#` is
pushed through the closure of every kernel item; symbols it generates are
recorded as spontaneous lookaheads on the target kernel item, the edges it
crosses become propagation links, and the links are iterated to a fixpoint. It
is the same LALR(1) lookahead set the lane table produces, from the LR(0)
automaton already in memory. Propagation follows terminal transitions as well as
non-terminal ones, which is easy to get wrong: a shifted item still carries the
lookaheads its later reductions need.

## Symbol numbering

```
0                      end of input
1 ..= terminals        terminals, in `.terminal` declaration order
terminals+1 ..         non-terminals, in first-definition order
last                   the augmented start symbol S'
```

A runtime must translate `deep_abi::EOF_SYMBOL` to `0` before a lookup; the
special `$` name in `Compiled::symbol` is `0`.

## Cell encoding

The borrowed `deep_abi::Grammar` form is always 32-bit cells, so it is
self-describing:

```
cell = (symbol << 16) | payload
payload = 0                      error (an explicit one, if a tag matched)
payload = 0x8000 | state         shift
payload = production + 1         reduce (1 is the augmented rule: accept)
```

`action_index` and `goto_index` hold one `(offset, count)` pair per state:
`offset` is the row-displacement base, `count` is the number of explicit cells,
and `-1`/`0` marks an empty row. A lookup is
`slot = offset + symbol`, then `cell >> 16 == symbol`; on a tag miss the parser
falls back to `default_reduce[state]` (a production id plus one, `0` for none).
Tag verification is what replaces the check array a Bison-style table needs.
Rows therefore never share a displacement base: two rows at the same base would
answer each other's lookaheads with a tag that matches.

Default reductions are chosen per state as the most frequent reduce action, ties
to the lower production id. Explicit error cells — the ones non-associativity
creates — are kept in the row so the default cannot paper over them.

### Narrowing

`Tables::packed()` re-encodes the same information for embedding:

* cells become 16-bit (`8`-bit tag + `8`-bit payload) when there are at most 255
  symbols, 127 states and 126 productions, else stay 32-bit;
* index pairs become 8-, 16- or 32-bit, whichever holds the largest offset;
* `default_reduce` stays 16-bit and `production_len` is already `u8`.

`Packed::decode()` rebuilds the canonical tables, and the tests assert the packed
form answers every `(state, symbol)` exactly like the uncompressed reference.

## Size report

`cargo run --release -p deep-tables --bin size-report` compiles the three
representative grammars and prints raw, ABI and packed bytes per grammar. On this
machine, for the checked-in grammars:

| grammar | states | symbols | productions | raw | packed | ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| expr | 20 | 14 | 12 | 1040 B | 278 B | 3.74x |
| json | 27 | 20 | 18 | 2052 B | 346 B | 5.93x |
| python-subset | 97 | 48 | 46 | 18236 B | 1868 B | 9.76x |

About 19 bytes per state on a realistic grammar. A 400-grammar fleet at Python
size is roughly 750 KB of tables, well inside the 10 MB budget.

## Limits

* at most 32,767 states, 32,766 productions and 65,534 symbols (the tagged
  encoding's ceiling, tighter than the `u16` ABI);
* a production's right-hand side is at most 255 items (`production_len` is `u8`);
* `build()` fails on the first unresolved conflict, with the state, terminal
  (id and name) and the competing actions in the error.

## Deliberately left out

* No GLR and no error recovery: conflicts that precedence cannot resolve are an
  error, never a fork. Recovery belongs to the runtime.
* No canonical LR(1) mode and no IEOC/`%prec` overrides; precedence comes only
  from the rightmost terminal of a rule.
* No emitter that writes tables as Rust source — the crate produces the flat
  slices, and embedding them in a `static` is the caller's packaging step.
* FOLLOW is computed and exposed for diagnostics; LALR lookaheads do not use it.
* No grammar validation beyond duplicate/undefined names: unreachable
  non-terminals and useless productions are not reported.
