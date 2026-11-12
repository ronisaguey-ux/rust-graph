# Solved — the last 24 hours

Baseline at 2026-11-11 19:00 (`cbd922b`): own `src` 0 fallbacks / 33 files, Felicity 0 / 66,
cargo 1451 green, pytest 889. Corpus sweep: python 242/568, rust 396/1816, js 1527/1995,
ts 1375/2000.

Now (2026-11-12 03:10, HEAD `603d060` + one uncommitted python fix): own `src` **0**,
Felicity **0/66**, cargo **1451**, python **166/568**, rust **358/1816**, js **943/1995**,
ts **453/2000**.

## Commits, newest first

| when | commit | what it fixed | measured |
|---|---|---|---|
| 11-11 19:36 | `72b3807` | an object literal keeps its boolean values | own-tree goal closed: 0/33 |
| 11-11 21:37 | `9127ff1` | a declaration header is not a call | ts 1375 → 869 |
| 11-11 21:59 | `62f6d92` | a chained assignment names the binder it writes | py 242 → 224 |
| 11-11 23:04 | `a0958e1` | the chain is walked to its end; every binder is named; `...` in an argument list is one argument of value 0 | py 224 → 210 |
| 11-11 23:13 | `e1afb86` | a declaration header is not a call **on the declared name** | ts 869 → 862 |
| 11-11 23:22 | `602e206` | `null` and `undefined` are names in JavaScript | js 1391 → 1388 |
| 11-11 23:34 | `9259510` | `(0, f)(x)` is a tuple of values; `var a = b = v` keeps its binder | js 1388 → 1099 |
| 11-11 23:42 | `12eb279` | a method header in a `.d.ts` is not a call | ts 862 → 820 |
| 11-11 23:55 | `7ca631b` | a signature wrapped over several lines; `code(cxt) {` is a method body | ts 820 → 806 |
| 11-12 00:15 | `1ad978f` | a type property and an export statement are declarations | ts 806 → 545, js 1098 → 943 |
| 11-12 00:23 | `a2d6879` | a wrapped type (`x?:` then `| A | B`) is still a declaration | ts 545 → 512 |
| 11-12 00:30 | `422fbbe` | `namespace`, `global`, `| A` continuation, `private x;` | ts 512 → 503 |
| 11-12 00:38 | `8d9795d` | `export type Check = …` declares a symbol | ts 503 → 487 |
| 11-12 00:47 | `27792bd` | a field with no type ends at its semicolon | ts 487 → 453 |
| 11-12 01:29 | `603d060` | the last expression of a block is not a statement | rust 373 → 358 |

Earlier in the same run (15:00-19:00 on 11-11): the own-tree campaign closed — 30 commits
from `e33880f` to `81ffc27` covering `with`/`except` bindings, comprehension targets,
keyword statements, closure param lists, generic variants, struct pattern fields, match
arm commas, `pub(crate)` import lists, f-string offsets, and the guard's acceptance rules.
That is the history that took own `src` from 21 fallbacks to **0** and Felicity to **0/66**.

## Also done today, outside rust-graph

- Box was 100% disk full (`commit-condom` mirrors, 119G). Freed 31G, swap flushed, the
  stremio container that was spinning at 130% CPU restarted.
- Stremio moved off this box: node + docker + `stremio/server` on **mastercrust**
  (192.168.86.37, LAN), the addon's systemd unit installed there, and the TV's addon URL
  swapped in the Stremio account from `192.168.86.49:3819` to `192.168.86.37:3819`.
- Stremio addon repo pushed (`a8e0474`): proxy the bytes, remember a warm swarm, drop the
  fallback that handed over dead candidates.
