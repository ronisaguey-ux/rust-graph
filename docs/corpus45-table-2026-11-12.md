# rust-graph — 45-language corpus table (measured 2026-11-12, 80-core Vast box)

Corpora: /home/roni/Roni_workspace/corpus45 (45 languages, 10,093 files).
Engine: rust-graph debug musl build, `build <dir> --no-cache --stdout`.
Only four languages have a CST reader, so only four can show a fallback: the rest are
lexer-only by design and read 0 because nothing is compared.

| language | files | fallbacks |
|---|---|---|
| javascript | 300 | 104 |
| typescript | 300 | 78 |
| python | 300 | 67 |
| rust | 90 | 40 |
| all other 39 languages (c, cpp, java, go, csharp, kotlin, lua, dart, ...) | 8,803 | 0 (no CST reader) |

Four-corpus comparison (larger local corpora, same binary):

| corpus | files | fallbacks |
|---|---|---|
| js | 1,995 | 725 |
| ts | 2,000 | 409 |
| rust | 1,816 | 345 |
| python | 568 | 146 |
| felicity | 300 | 16 |

Reason histogram, js (725): 571 "the CST lost a event", 65 symbols, 43 calls, 37 vars.
The dominant shape is one line holding several events (the condition and the call in
`if (x) f(x)`): the CST writes one per expression, the one-pass lexer one per statement.
Fixing that class is worth roughly 1,100 files across js/ts/rust/python.
