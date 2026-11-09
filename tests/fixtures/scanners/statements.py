"""Value-bearing statements where only the first child used to be read.

`raise E(x) from c(y)`, `assert a(v), b(v)` and `del xs[k(v)]` each carry more
than one call; `return f(a), g(b)` and `x = (n := g())` are expression lists
and named expressions. The one-pass lexer records all of them, so the CST walk
must too.
"""


def fail(x, y):
    raise ValueError(make(x)) from cause(y)


def check(v):
    assert validate(v), explain(v)
    items = load(v)
    del items[index(v)]
    return build(v), render(v)


def walrus(rows):
    if (count := len(rows)) > 0:
        return count
    return 0


def defaults(a=load(1), b: int = parse(2), *, c=build(3)):
    return a, b, c
