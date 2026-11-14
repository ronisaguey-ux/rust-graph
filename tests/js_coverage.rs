//! The JavaScript reader's coverage table.
//!
//! Every construct the plan claims the reader covers is one row here, and one
//! row is one assertion: both readers read the file and the guard finds nothing
//! to fall back for. That is the difference between "the grammar has a node for
//! it" and "the fact extractor turns that node into the same record the
//! one-pass reader does", and the second is the only one a consumer sees.
//!
//! The fixtures are the smallest source that exercises the construct, not a
//! corpus excerpt, so a failure names the construct instead of a file.
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
    let language = langs::by_content("js", source.as_bytes()).expect("a JavaScript row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::JavaScript)
}

/// One row per claimed construct.
const COVERAGE: &[(&str, &str)] = &[
    (
        "block statement",
        "function f(x) {\n  let y = x;\n  return y;\n}\n",
    ),
    (
        "if / else if / else",
        "function f(x) {\n  if (x) {\n    return 1;\n  } else if (x > 2) {\n    return 2;\n  } else {\n    return 3;\n  }\n}\n",
    ),
    (
        "for-in statement",
        "function f(all) {\n  for (var name in all)\n    console.log(name);\n}\n",
    ),
    (
        "while and do-while",
        "function f(n) {\n  while (n) n--;\n  do {\n    n++;\n  } while (n < 2);\n}\n",
    ),
    (
        "switch with cases",
        "function f(x) {\n  switch (x) {\n    case 1:\n      return 1;\n    default:\n      return 2;\n  }\n}\n",
    ),
    (
        "try / catch / finally and throw",
        "function f(x) {\n  try {\n    return g(x);\n  } catch (e) {\n    throw e;\n  } finally {\n    h();\n  }\n}\n",
    ),
    (
        "destructuring declarations and parameters",
        "const { a, b: c } = obj;\nconst [x, ...rest] = list;\nfunction f({ a, b }, [c]) {\n  return a + b + c;\n}\n",
    ),
    (
        "arrow functions: expression and block bodies",
        "const double = (x) => x * 2;\nconst run = async (x) => {\n  return double(x);\n};\nconst obj = (x) => ({ x });\n",
    ),
    (
        "template literals and tagged templates",
        "function f(name) {\n  const plain = `hi ${name}`;\n  const tagged = tag`a ${name} b`;\n  return plain + tagged;\n}\n",
    ),
    (
        "classes: methods, getters, fields, static and private",
        "class A extends B {\n  static count = 0;\n  #secret = 1;\n  constructor(x) {\n    super(x);\n  }\n  get value() {\n    return this.#secret;\n  }\n  method(y) {\n    return this.value + y;\n  }\n}\n",
    ),
    (
        "expressions: ternary, comma, comma assignment and logical assignment",
        "function f(a, b) {\n  const x = a ? b : 0;\n  a ||= b;\n  a ??= 1;\n  a &&= 2;\n  return a, x;\n}\n",
    ),
    (
        "calls: member, optional, constructor and chain",
        "function f(a) {\n  const b = new Map();\n  a.b().c(a?.d);\n  return b.size;\n}\n",
    ),
    (
        "exports and imports",
        "import def, { a as b } from './m';\nimport * as ns from './n';\nexport { b };\nexport * from './o';\nexport default function f() {\n  return ns.x + def;\n}\n",
    ),
    (
        "re-exports and CommonJS require shapes",
        "export { a as b } from './m';\nexport * as api from './n';\nconst is = require('./is');\nconst { x, y: z } = require('./xy');\nconst get = require('./get');\n",
    ),
    (
        "generators and yield",
        "function* gen(xs) {\n  for (const x of xs) {\n    yield x;\n  }\n}\n",
    ),
    (
        "regex and numeric literals",
        "const re = /a+b/g;\nconst n = 0x10 + 1_000 + 1e3;\nconst ok = re.test('aab');\n",
    ),
    (
        "optional call",
        "function f(a) {\n  const c = a?.();\n  return c;\n}\n",
    ),
    (
        "object literal families: keyword keys, computed keys, shorthand, methods and spread",
        "const a = { delete: 1, default: 2, class: 3, new: 4, in: 5, of: 6, function: 7 };\nconst b = { [key]: 1, [a.b]: 2, ['str']: 3 };\nconst c = { x, y, z };\nconst d = { m() {}, get g() { return 1; }, set s(v) {}, async am() {}, *gen() {} };\nconst e = { ...rest, q: 1 };\nconst f = { type, value, size };\n",
    ),
    (
        "computed object key",
        "const key = 'k';\nconst o = { [key]: 1 };\n",
    ),
    (
        "object literals: shorthand, keyed, method and spread",
        "function f(a) {\n  const o = { a, b: 1, c(x) { return x; }, ...a };\n  return o.b;\n}\n",
    ),
    (
        "multi-line ternary",
        "function f(a, b) {\n  const x = a\n    ? b\n    : 0;\n  return x;\n}\n",
    ),
    (
        "null and undefined literals",
        "function f(x) {\n  if (x) return null;\n  return undefined;\n}\n",
    ),
    (
        "for-of statement with an inline body",
        "function f(xs) {\n  for (const x of xs) console.log(x);\n}\n",
    ),
    (
        "c-style for with an inline body",
        "function f(xs) {\n  for (var i = 0; i < 2; i++) console.log(i);\n}\n",
    ),
];

/// Constructs the reader does not cover yet, with the exact disagreement.
///
/// This list is asserted, not skipped: a test that fails here means a gap was
/// closed and the row moved up to `COVERAGE`, which is how a known gap stops
/// being a comment and starts being a checklist.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[
    (
        "labelled statement",
        "function f(xs) {\n  outer: for (const x of xs) {\n    if (x) continue outer;\n  }\n}\n",
        "the CST drops the label name; the lexer writes it beside the loop binding",
    ),
];


#[test]
fn every_claimed_javascript_construct_reads_the_same_on_both_paths() {
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
    assert!(failed.is_empty(), "the JS reader falls back on {} construct(s):\n{}", failed.len(), failed.join("\n"));
}

#[test]
fn the_known_gaps_are_still_gaps() {
    let mut closed: Vec<&str> = Vec::new();
    for (name, source, _) in KNOWN_GAPS {
        let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
        if !verdict.lost() {
            closed.push(name);
        }
    }
    assert!(closed.is_empty(), "these rows now read cleanly, move them to COVERAGE: {closed:?}");
}
