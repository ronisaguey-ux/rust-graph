//! The TypeScript reader's coverage table.
//!
//! The mirror of `tests/js_coverage.rs`: every construct the plan claims the TS
//! reader covers is one row, and one row is one assertion — both readers read
//! the smallest source that exercises the construct and the guard finds nothing
//! to fall back for. The plain JavaScript rows are repeated here so a TS-only
//! change cannot silently break the node set the two grammars share.
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
    let language = langs::by_content("ts", source.as_bytes()).expect("a TypeScript row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), Lang::TypeScript).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, Lang::TypeScript)
}

/// One row per construct that reads cleanly today.
const COVERAGE: &[(&str, &str)] = &[
    (
        "type annotations: variables, parameters, returns and fields",
        "function f(a: number, b: string): boolean {\n  const c: number = a + 1;\n  return b.length > c;\n}\n",
    ),
    (
        "interface: properties, optional members, methods and extends",
        "interface Base {\n  id: number;\n}\ninterface User extends Base {\n  name: string;\n  nick?: string;\n  greet(who: string): string;\n}\n",
    ),
    (
        "generics on functions, classes and interfaces with a constraint",
        "interface Box<T> {\n  value: T;\n}\nfunction id<T extends object>(x: T): T {\n  return x;\n}\nclass Holder<T> {\n  constructor(private item: T) {}\n}\n",
    ),
    (
        "enums: numeric, string and computed",
        "enum Direction {\n  Up,\n  Down = 4,\n}\nenum Named {\n  A = 'a',\n  B = 'b',\n}\n",
    ),
    (
        "namespaces with exported members",
        "namespace NS {\n  export const x = 1;\n  export function f() {\n    return x;\n  }\n}\n",
    ),
    (
        "import type and export type",
        "import type { Foo } from './foo';\nexport type Bar = Foo | null;\nexport function f(x: Bar) {\n  return x;\n}\n",
    ),
    (
        "abstract classes with implements and access modifiers",
        "abstract class Shape {\n  protected name: string;\n  private size = 0;\n  abstract area(): number;\n  describe(): string {\n    return this.name;\n  }\n}\nclass Square extends Shape implements Shape {\n  area(): number {\n    return 1;\n  }\n}\n",
    ),
    (
        "readonly, definite assignment, getters and setters",
        "class A {\n  readonly id: number = 1;\n  name!: string;\n  get label(): string {\n    return this.name;\n  }\n  set label(v: string) {\n    this.name = v;\n  }\n}\n",
    ),
    (
        "satisfies and as clauses",
        "const cfg = { a: 1 } satisfies Record<string, number>;\nconst n = (cfg as unknown as { a: number }).a;\n",
    ),
    (
        "type aliases: unions, intersections, function and tuple types",
        "type Id = string | number;\ntype Pair = { a: number } & { b: number };\ntype Fn = (x: number) => void;\ntype Tup = [number, string];\n",
    ),
    (
        "type operators: keyof, typeof, indexed access and non-null",
        "type K = keyof { a: number };\ntype T = typeof globalThis;\ntype V = { a: number }['a'];\nfunction f(x?: string) {\n  return x!.length;\n}\n",
    ),
    (
        "JS node set: statements, arrows, destructuring and templates",
        "function f({ a, b }: { a: number; b: number }, [c]: number[]) {\n  const double = (x: number) => x * 2;\n  const s = tag`v ${a} ${b} ${c}`;\n  return double(a) + s.length;\n}\n",
    ),
    (
        "JS node set: classes, optional chaining and exports",
        "export default class A extends B {\n  #secret = 1;\n  static count = 0;\n  get value(): number {\n    return this.#secret;\n  }\n  call(x?: { d(): number }) {\n    return x?.d() ?? 0;\n  }\n}\n",
    ),
    (
        "decorators on a class, a method and a property",
        "function dec(...args: unknown[]) {\n  return args;\n}\n@dec\nclass A {\n  @dec\n  field: number = 1;\n  @dec\n  method() {\n    return this.field;\n  }\n}\n",
    ),
];

/// Constructs the reader does not cover yet, with the exact disagreement.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];


#[test]
fn every_claimed_typescript_construct_reads_the_same_on_both_paths() {
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
        "the TS reader falls back on {} construct(s):\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn the_known_typescript_gaps_are_still_gaps() {
    let mut closed: Vec<&str> = Vec::new();
    for (name, source, _) in KNOWN_GAPS {
        let verdict = pipeline::loses(&lexer_facts(source), &deep_facts(source));
        if !verdict.lost() {
            closed.push(name);
        }
    }
    assert!(closed.is_empty(), "these rows now read cleanly, move them to COVERAGE: {closed:?}");
}
