//! The Python reader's coverage table.
//!
//! The mirror of `tests/ts_coverage.rs`: every construct the plan claims the
//! Python reader covers is one row, and one row is one assertion — both readers
//! read the smallest source that exercises the construct and the guard finds
//! nothing to fall back for.
//!
//! A construct that does not read cleanly yet is *asserted to fail*, not
//! skipped: `KNOWN_GAPS` names the exact disagreement, so closing one without
//! moving the row up fails the suite.
#![cfg(all(feature = "deep", feature = "parser"))]

#[path = "../src/facts.rs"]
mod facts;
#[path = "../src/json.rs"]
mod json;
#[path = "../src/langs.rs"]
mod langs;
#[path = "../src/lexer.rs"]
mod lexer;
#[path = "../src/expr.rs"]
mod expr;
#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/index.rs"]
mod index;
#[path = "../src/resolve.rs"]
mod resolve;
#[path = "../src/sha256.rs"]
mod sha256;
#[path = "../src/deep/mod.rs"]
mod deep;
#[path = "../src/deep_facts/mod.rs"]
mod deep_facts;
#[path = "../src/pipeline.rs"]
mod pipeline;

use pipeline::gate::*;

use deep_integration::Lang;
use facts::Facts;

fn lexer_facts(source: &str) -> Facts {
    let language = langs::by_content("py", source.as_bytes()).expect("a Python row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::Python).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::Python)
}

/// One row per construct that reads cleanly today.
const COVERAGE: &[(&str, &str)] = &[
    (
        "indentation suites: if, for, with and try/except/else/finally",
        "def outer(n):\n    if n > 0:\n        total = n\n        for i in range(n):\n            total += i\n        with open(\"x\") as fh:\n            data = fh.read()\n        try:\n            risky()\n        except ValueError as err:\n            handle(err)\n        else:\n            ok()\n        finally:\n            done()\n    return total\n",
    ),
    (
        "decorators: a plain one and a stacked one with arguments",
        "@cache\ndef load(path):\n    return read(path)\n\n@app.route(\"/x\", methods=[\"GET\"])\n@login_required\ndef view(request):\n    return respond(request)\n",
    ),
    (
        "comprehensions: list, dict, set and generator",
        "def f(rows):\n    squares = [r * r for r in rows if r > 0]\n    table = {k: v for k, v in pairs(rows)}\n    seen = {r.name for r in rows}\n    gen = (r for r in rows if r)\n    return squares, table, seen, gen\n",
    ),
    (
        "f-strings: interpolation, format spec, conversion and a call",
        "def greet(name, count, mapping):\n    plain = f\"hello {name}\"\n    nested = f\"total {count + 1:>4} for {name!r}\"\n    deep = f\"{mapping['k']}\"\n    called = f\"{load(name)} ready\"\n    return plain, nested, deep, called\n",
    ),
    (
        "lambda: a plain lambda and one with a default argument",
        "def f(xs):\n    double = lambda v: v * 2\n    key = lambda row, i=0: (row[i], -row[1])\n    return sorted(xs, key=key), list(map(double, xs))\n",
    ),
    (
        "walrus: a binding in a while condition and in an if",
        "def f(chunks):\n    out = []\n    while (line := chunks.pop()) is not None:\n        if (n := len(line)) > 10:\n            out.append((n, line))\n    return out\n",
    ),
    (
        "async forms: async def, async with, async for and await",
        "async def fetch(client, url):\n    async with client.session() as session:\n        async for row in session.stream(url):\n            await handle(row)\n    return await collect()\n\nasync def main():\n    result = await fetch(Client(), \"http://x\")\n    return result\n",
    ),
    (
        "match statement: subject and literal, sequence and mapping arms",
        "def f(x):\n    match x:\n        case 1:\n            return 'one'\n        case [a, b]:\n            return a\n        case {'k': v}:\n            return v\n    return None\n",
    ),
    (
        "match arm with a guard and a class pattern",
        "def f(x):\n    match x:\n        case Point(x=0, y=0):\n            return 'origin'\n        case Point(x=a) if a > 0:\n            return a\n    return None\n",
    ),
];

/// Constructs the reader does not cover yet, with the exact disagreement.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[(
    "f-string: an escaped brace pair reads the literal text as a name",
    "def f(count):\n    joined = f\"{'{'}literal{'}':>{count}}\"\n    return joined\n",
    "the CST writes the whole f-string as one constant; the lexer reads the escaped braces as a nested literal and `literal` as a name (wanted m:[0,n:literal,m:[m:[0,m:[n:count]]]] against m:[0,0,n:count])",
)];

#[test]
fn every_claimed_python_construct_reads_the_same_on_both_paths() {
    let mut failed: Vec<String> = Vec::new();
    for (name, source) in COVERAGE {
        let lexer = lexer_facts(source);
        let deep = deep_facts(source);
        assert!(!lexer.symbols.is_empty(), "{name}: the lexer read no symbol");
        let verdict = pipeline::loses(&lexer, &deep);
        if verdict.lost() {
            failed.push(format!("{name}: {verdict:?}"));
        }
    }
    assert!(
        failed.is_empty(),
        "the Python reader falls back on {} construct(s):\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn the_known_python_gaps_are_still_gaps() {
    let mut closed: Vec<&str> = Vec::new();
    for (name, source, _) in KNOWN_GAPS {
        let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
        if !verdict.lost() {
            closed.push(name);
        }
    }
    assert!(closed.is_empty(), "these rows now read cleanly, move them to COVERAGE: {closed:?}");
}
