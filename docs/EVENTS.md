# The event model

What a fact looks like, who decides how many of them a line holds, and why the
answer is the parser and not the lexer.

## The row

One row is one flow record: a symbol, the line the expression sits on, and the
events it carries.

```
{"symbol":"a.ts:f","line":3,"events":[["a",3,["a","b"],["t",[["c",0,0,[["n","x"]]],0]],0]]}
```

`symbol` is `<file>:<name>`, `events` is a list of `[kind, line, target, value,
constant]`. The kinds describe what the statement does with its value: `a` a
binding, `r` a return, `x` an expression with no target, and so on. The value is
the form below — how the values combine, never what they mean. A reader decides
what is dangerous; the graph records only the shape.

## The form

```
0                     a constant
["n","a.b"]           a name or attribute path
["m",[e,...]]         values combined (operator, format string, mapping)
["t",[e,...]]         a tuple or list, element by element
["c",id,recv,[arg]]   a call; `id` indexes the symbol's call table
["k","name",e]        a keyword argument, only inside a call's arguments
```

Two invariants hold across every language, and both have a test that fails when
they do not:

- **a call is atomic** — one `c` per source call, and the callee never also
  stands beside it as a name (`tests` in `src/expr.rs`);
- **an arrow is one constant in the JS family**, while a Rust `=>` opens a match
  arm and the arm's expression is the value. Getting these two swapped cost
  rust a hundred files the last time it was tried.

## Why the CST is the authority on how many events a line holds

The lexer is one pass with no grammar. It splits a statement at `;` and at a
header word, and inside a template literal, a multi-line arrow body or an object
literal it is guessing where the value ends. The CST has already parsed the file,
so it knows: a `return` holds one expression however many commas are inside it,
and a function expression is a value rather than the call its parameter list
looks like to a token stream.

That is the whole reason the fallback exists. The default path is the lexer, and
it is fast; when a file's CST record is the heavier one, or the lexer found
nothing at all, the CST record is used instead. The `deep_fallback` field on a
source names the reason in plain words (`the CST lost a event`), and the count of
those files is the language coverage number this work moves.

## Measuring a change

    tools/measure_subsets.sh           # js, ts, python, rust against their baselines
    tools/diff.py <before.json> <after.json>

A change is only a win when every subset holds or gains, and no file that was
clean goes back to falling back. Totals alone are not evidence: five files gaining
a symbol while five lose an event is a flat number and a worse graph, which is why
the harness keeps per-file counts.

## Segments and markers are gone

An earlier design added `Segment` and `Marker` — one row per top-level expression
inside a statement instead of one per statement — behind the `event-split`
feature. It was measured on the fifty-file JS subset and made no difference at
all: 18 files fell back with the feature and 18 without it, with an identical
reason histogram, because the guard compares rows the split never changed. The
feature, its types and its tests were deleted in `1082e0f` rather than left
dormant behind a flag. If a future change needs finer rows, it starts from
measurement, not from resurrecting that branch.
