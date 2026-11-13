# Event-count disagreements, one entry each

`the CST lost a event` is one line in the fallback histogram and it covers
everything from a one-event difference to a hundred, in both directions. That is
not a queue item, it is a folder. This is the folder broken up.

Produced by `tools/event_taxonomy.py`, run over fifty files per language. An
entry is `language / direction / size / what-else-disagrees`, and each one is
taken on its own: the fix for `CST_writes_more` is not the fix for
`lexer_writes_more`, and the fix for `large` is not the fix for `small`.

    tools/event_taxonomy.py --corpus /tmp/js50 --lang js --binary <built binary>

## JavaScript

| entry | files | delta | examples |
| --- | ---: | ---: | --- |
| `js/CST_writes_more/large/symbols+edges` | 2 | +257 | `000039_7517.b4d8b77497bdb47f.js` cst 547 / lexer 340 |
| `js/CST_writes_more/large/symbols` | 2 | +80 | `000002_index.js` cst 1288 / lexer 1231 |
| `js/CST_writes_more/large/edges` | 1 | +39 | `000031_source-map.js` cst 1603 / lexer 1564 |
| `js/CST_writes_more/small/symbols` | 8 | +34 | `000001_borderTopWidth.js` cst 22 / lexer 13 |
| `js/CST_writes_more/small/events_only` | 6 | +18 | `000036_expand.js` cst 21 / lexer 15 |
| `js/lexer_writes_more/small/events_only` | 4 | −7 | `000009_set.js` cst 18 / lexer 19 |
| `js/lexer_writes_more/small/symbols` | 1 | −1 | `000034_accessible-description.js` cst 61 / lexer 62 |
| `js/lexer_writes_more/small/symbols+edges` | 1 | −1 | `000035_date.js` cst 35 / lexer 36 |

## TypeScript

| entry | files | delta | examples |
| --- | ---: | ---: | --- |
| `ts/lexer_writes_more/large/symbols` | 8 | −394 | `000002_vm.d.ts` cst 2 / lexer 90 |
| `ts/lexer_writes_more/small/symbols` | 11 | −83 | `000003_blob.d.ts` cst 2 / lexer 15 |
| `ts/lexer_writes_more/small/events_only` | 2 | −15 | `000035_Exception.d.ts` cst 0 / lexer 13 |
| `ts/CST_writes_more/small/symbols` | 3 | +17 | `000047_not.ts` cst 13 / lexer 2 |
| `ts/CST_writes_more/small/symbols+edges` | 1 | +9 | `000045_instanceof.test.ts` cst 75 / lexer 66 |
| `ts/CST_writes_more/small/events_only` | 1 | +6 | `000038_parseJson.ts` cst 182 / lexer 176 |

**The one entry that is a bug and not a difference.** Every `lexer_writes_more`
TypeScript row above is a `.d.ts` file, and the shape is not a rounding
difference: `000002_vm.d.ts` is 90 symbols to the lexer and 2 to the CST. Ambient
declarations (`declare module`, `interface` members) are declared API, and the
CST writing two of them means it read the file as almost empty. That is the first
TypeScript entry to take, and the reproduction is `000006_files.d.ts`, where the
lexer finds 41 and the CST finds none.

## Python

| entry | files | delta | examples |
| --- | ---: | ---: | --- |
| `python/CST_writes_more/small/edges` | 6 | +37 | `000003_runpy.py` cst 232 / lexer 227 |
| `python/CST_writes_more/small/events_only` | 8 | +23 | `000007_dis.py` cst 668 / lexer 665 |
| `python/lexer_writes_more/small/edges` | 4 | −24 | `000002_inspect.py` cst 2666 / lexer 2673 |
| `python/lexer_writes_more/small/events_only` | 16 | −43 | `000001_hashlib.py` cst 134 / lexer 136 |

Python has no `large` entry in either direction: its disagreements are one to
three rows on big files, which is what a correct pair of readers looks like when
they split a statement differently. Nothing to take here yet.

## Rust

No entry: the fifty-file subset has no event-count disagreement at all, in
either direction. That is why rust is the language a JS change is measured
against — it holds still.
