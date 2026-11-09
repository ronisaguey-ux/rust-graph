# deep-grammars

The Python, JavaScript/TypeScript grammars of the deep scan, written in
`grammar!`, with the hand-written step lexer each one needs and the fixtures
that pin their behaviour.

| Language | Module | Grammar entry | Lexer |
|:--|:--|:--|:--|
| Python | `src/python.rs` | `grammar! { PYTHON { ... } }`, `layout = NEWLINE INDENT DEDENT` | `PYTHON.lex` (`fn(&mut Cursor) -> Kind`) |
| JavaScript | `src/javascript.rs` | `grammar! { TYPESCRIPT { ... } }` (one grammar for all three) | `lex_jsx` (`JAVASCRIPT`) |
| TypeScript / TSX | `src/typescript.rs` re-exports `javascript.rs` | same | `lex_typescript` / `lex_jsx` |

TypeScript is a superset of JavaScript, so there is one grammar definition
named `TYPESCRIPT`; the three `Language` statics differ only in the lexer
(`<` as a type-assertion operator versus the start of a JSX element). Splitting
it would duplicate ~550 rules to change one token's meaning.

The runtime (`syntax`, `lexer`, `pairs`, `parser`, `tree`, `budget`) is
compiled from the `rust-graph` package's `src/deep` **by path** in `src/deep.rs`,
because the separate `deep-runtime` crate it is being split into is not in this
worktree yet. The grammars keep the DSL's default `runtime = crate::deep`, so
pointing them at that crate later is one line. Nothing in the runtime is
duplicated or modified.

## Fixtures

| Directory | Files | What they are |
|:--|--:|:--|
| `fixtures/python/` | 10 | real-shaped modules: dataclasses and typing, Flask-style routes, asyncio workers, argparse CLI, `unittest`, models with `__slots__`, a scraper, matrix math, layered config, TypedDict/Protocol/`type` aliases |
| `fixtures/javascript/` | 10 | Express server, private-field class, Redux reducer, promise/retry helpers, `EventEmitter` queue, regex log parsing, DOM helpers, `stream` transform, ESM module, statistics functions |
| `fixtures/typescript/` | 11 | branded types and generics, service class, env config with mapped types, router, store, `Result` guards, collection helpers, HTTP client with retries, recursive-descent parser, telemetry, generic linked list |
| `fixtures/malformed/` | 12 | per language: a broken function signature (`def 1f` / `function 1f` / `export function 1f`), an unterminated string, an unclosed brace, a stray keyword (`else` with no `if`) |

43 files, 31 valid and 12 malformed.

## Tests

```bash
cargo test -p deep-grammars
```

On this box (no C toolchain) the proc-macro crate has to be linked against the
host libc, so the host target needs the linker wrapper; the musl target already
uses `rust-lld`:

```bash
PATH="$HOME/.cargo/bin:$PATH" \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$HOME/.local/bin/rust-lld-cc" \
cargo test -p deep-grammars
```

`tests/fixtures.rs` (6 tests) asserts, per fixture: the parse completes; the
root is the language's root; a valid file has **zero** error nodes and zero
bad-token leaves; a malformed file keeps its root, spans the file, and carries
at least one error **inside** it (the cascade rule); and the top-level node
kinds equal the expectation table exactly. The tables list fixtures by name, so
a new file without an expectation fails rather than being skipped.

`tests/lexers.rs` (9 tests) covers the lexers directly: single/double/triple/
raw/template strings and bad (unterminated) strings, `#`/`//`/`/* */` comments,
hex/binary/octal/underscored/float/imaginary numbers, JavaScript regex-versus-
division, TypeScript's `>joined` generic closer, and Python's
`NEWLINE`/`INDENT`/`DEDENT` including the bracket and backslash rules.

## Coverage

**Python — modelled.** Modules; `import`/`from ... import` with relative
dots, aliases, parentheses and `*`; decorators; `def`/`async def` with
positional-only `/`, `*args`, `**kwargs`, defaults, annotations and PEP 695
type parameters; classes with bases, keywords and type parameters; `if/elif/
else`, `while`/`else`, `for`/`async for`/`else`, `with`/`async with` (multiple
items), `try/except/except*/else/finally`, `match`/`case` patterns (literals,
captures, sequences, mappings, classes, `as`, `|`, guards); `return`, `yield`,
`yield from`, `await`, `raise ... from`, `assert`, `del`, `pass`, `break`,
`continue`, `global`, `nonlocal`; lambdas; list/set/dict/generator
comprehensions with multiple `for`/`if` clauses; the whole operator precedence
table (comparisons, `not in`, `is not`, `**`, bitwise, boolean, walrus,
conditional); attribute/call/subscript chains and slices; `type X[T] = ...`.

**Python — not modelled.** f-string/format-spec *fields* are not parsed: a
string is one `string_literal` token (`f"{a!r:>{w}}"` is opaque). Docstrings
are ordinary expression statements, not attached as documentation. Comments are
trivia. The tree deliberately differs from CPython's AST and roughly follows
Tree-sitter's Python names.

**JavaScript / TypeScript — modelled.** Every statement kind (blocks, `if`,
`for`, `for-in/of`, `while`, `do`, `switch`, `try`, `throw`, `return`, `break`,
`continue`, labels, `with`, `debugger`, empty `;`); declarations (`var`/`let`/
`const`, destructuring, functions, generators, async functions, classes with
private `#` fields, static blocks, decorators, `abstract`/`override`/`declare`/
`readonly`/`accessor` modifiers, getters/setters, computed keys, class
heritage); expressions with precedence and associativity (assignment,
conditional, `??`, `||`, `&&`, bitwise, shifts, arithmetic, `**`, unary,
`await`, `yield`, `new`, `delete`, `typeof`, `void`, `in`, `instanceof`,
`as`, `satisfies`, non-null `!`, optional chaining, calls, member/subscript,
arrow functions, object/array literals, spread, sequences); modules
(`import`/`export`, default/named/namespace, `type` modifiers, attributes);
TypeScript declarations (`interface`, `type`, `enum`, `namespace`/`module`,
`declare`, generic parameters with constraints and defaults, union/intersection,
conditional and `infer`, `keyof`/`typeof`/`readonly`/`unique` operators, index
signatures, mapped types, tuples, function types, overload-style signatures,
type assertions, `implements`).

**JavaScript / TypeScript — partly modelled.** A template literal's `${...}` is
parsed as expressions: the substitution is a `template_substitution` node and a
call in it is a `call_expression`. A JSX element is parsed too, for
`JAVASCRIPT`/`TSX`: the opening element, its attributes, the children and the
closing element are nodes, `{...}` is a `jsx_expression` whose content is
ordinary code, `<A.B/>` is a `nested_identifier` and `<a:b/>` a
`jsx_namespace_name`. Text children and quoted attribute values stay tokens and
their entities are not decoded. A regular expression is one token. The tree is
Tree-sitter-shaped but not Tree-sitter's tree. Automatic semicolon insertion is
modelled as optional semicolons, so a restricted production (`return` then a
value on the next line) reads as one statement, not two.

## What the DSL could not express

These are properties of `grammar!` as implemented, not of the languages; each is
worked around in the hand lexer rather than by inventing syntax.

1. **Layout is a runtime feature, not a rule.** Indentation blocks cannot be
   written in the DSL; `layout = NEWLINE INDENT DEDENT;` names three tokens the
   runtime lexer infers, and the rules consume them (`INDENT ... DEDENT`).
   Without that setting Python could not be described at all.
2. **A mode is coarse, and the parser cannot drive one.** `modes`/`switch`
   gives the step lexer a stack of steps that a *token* pushes and pops, which
   is enough for a Python f-string (text, then code in a field, then text) and a
   heredoc, but there is no external scanner: the parser cannot ask for a mode
   at a parse position. Context-sensitive tokens that depend on the previous
   token — JS regex versus division, JSX versus `<T>`, `>` `>` versus `>>` —
   are still decided inside one `fn(&mut Cursor) -> Kind` from
   `Cursor::previous`. A JSX element also needs a state per open tag while a
   template's mode frames may already be open, so the JavaScript lexer keeps a
   second, hand-written frame stack beside the mode stack and the two
   interleave: a `{` in a tag or between children is ordinary code again with
   its own matching `}` as a token of the JSX state, and the flags say which
   frame was entered first. That is why a template's substitution and a JSX
   expression both parse, while a Python f-string's field needs only the mode
   stack.
3. **A decision sees at most three tokens.** `(a, b) =>`, `f<T>(x)` and
   `for (x of y)` are entered via `&(...)` predicates; there is no arbitrary
   lookahead and no conflict priority other than alternative order. A construct
   needing four tokens of context must be split into extra hidden rules.
4. **Predicates only start an alternative** (or an operator row), never appear
   mid-sequence, and there is no negative lookahead.
5. **One kind per rule.** Alternatives cannot be given their own node name, so
   `function` and `class` must be separate rules; there are no labelled or
   inlined alternatives and no precedence annotation on rules other than the
   `precedence` table.
6. **No empty alternative.** `[x]`/`x?` replaces one, by design. A trailing
   `x %? sep` is rejected where `sep` could also follow the list: the macro
   asks for the plain `%` form instead.
7. **Recovery is derived, not written.** There are no error productions or
   explicit recovery rules to tune: the macro derives `MISSING`/`ERROR`
   insertion from first/follow sets. A construct whose recovery should differ
   can only be reshaped by moving it between rules.
