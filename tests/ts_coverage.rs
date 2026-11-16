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
        "import-equals: a require declaration is an import, not a call",
        "import fs = require('fs');\nimport _ = require(\"../index\");\n",
    ),
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
    (
        "conditional type on a declaration, object type opened on the next line",
        "declare var M: typeof globalThis extends\n    { onmessage: any; M: infer T } ? T : typeof undici.M;\n",
    ),
    (
        "conditional type on a declaration with comma-separated object type and object fallback",
        "var Event: typeof globalThis extends { onmessage: any, Event: infer T }\n    ? T\n    : { prototype: __Event };\n",
    ),
    (
        "a heritage clause that wraps: the interface header spans the line break",
        "export interface declareConstructor<T, E> extends\n  Bottom<\n    T,\n    E\n  >\n{\n  readonly id: number\n}\n",
    ),
    (
        "type operators in front of a group: a readonly array constraint is not a call",
        "declare const M: <const Keys extends readonly (\"json\" | \"insert\")[]>(keys: Keys) => void;\n",
    ),
    (
        "type annotations: a generic annotation is cut at its colon, not its commas",
        "interface Matcher<A, B> { id: A; other: B }\nconst proto: Omit<\n  Matcher<any, any, any, any, any>,\n  \"provided\" | \"value\"\n> = {\n  id: 1,\n};\n",
    ),
    (
        "type arguments: `new` with type arguments is one call, not a merge",
        "type Events = \"open\" | \"close\";\ndeclare class Handler<T> {}\nconst handlers = new Map<keyof Events | \"*\", Array<Handler<any>>>();\nconst empty = new Map<string, number>();\n",
    ),
];

/// Constructs the reader does not cover yet, with the exact disagreement.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[];

/// Rows that need the source in hand: the guard excuses them by reading the
/// line the fact sits on, so they can only be compared through `loses_in`.
const LINE_ROWS: &[(&str, &str)] = &[
    (
        "a member signature whose line is opened by another group: has(o: any) in a union type",
        "export declare function f(\n    state: (new (...args: any[]) => unknown) | { has(o: any): boolean },\n    receiver: unknown,\n): boolean;\n",
    ),
    (
        "a generic method signature whose type argument calls import(): setState<K extends keyof import(…).S>",
        "declare class Chart {\n    setState<K extends keyof import(\"./types\").State>(state: import(\"./types\").State, callback?: () => void): void;\n}\n",
    ),
    (
        "an annotated arrow whose annotation is a function type: the name is before the colon",
        "export const getCodePoint: (input: string, index: number) => number = (\n    input: string,\n    index: number,\n): number => input.codePointAt(index)!;\n",
    ),
];

#[test]
fn a_line_that_declares_a_member_is_not_a_call() {
    let mut failed: Vec<String> = Vec::new();
    for (name, source) in LINE_ROWS {
        let verdict = pipeline::loses_in(&lexer_facts(source), &deep_facts(source), source);
        if verdict.lost() {
            failed.push(format!("{name}: {verdict:?}"));
        }
    }
    assert!(failed.is_empty(), "the TS reader falls back on {} row(s):\n{}", failed.len(), failed.join("\n"));
}


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
