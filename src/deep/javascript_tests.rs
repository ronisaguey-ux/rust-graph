//! The JavaScript grammar's own tests. They live here, not in the grammar
//! file, because `javascript` is that file included by path: the root crate
//! and `deep-grammars` compile one text, and these tests run against it.

use super::budget::Budget;
use super::javascript::*;
use super::parser::Workspace;
use super::syntax::Language;
use super::tests::{check, mutate, on_big_stack, Rng};
use super::tree::HAS_ERROR;

fn parse(src: &str, language: &Language) -> Workspace {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
    assert!(outcome.complete(), "{src:?}: {}", outcome.describe());
    check(&workspace, src.as_bytes(), language);
    workspace
}

fn named(src: &str, language: &Language) -> String {
    parse(src, language).tree.sexp(language, true)
}

fn clean(src: &str, language: &Language) -> bool {
    parse(src, language).tree.root().flags & HAS_ERROR == 0
}

/// The expression in the statement `SRC;`.
fn expression(src: &str, language: &Language) -> String {
    let tree = named(&format!("{src};"), language);
    tree.strip_prefix("(program (expression_statement ").and_then(|rest| rest.strip_suffix("))")).unwrap_or_else(|| panic!("{src}: {tree}")).to_string()
}

/// The significant tokens' kinds.
fn kinds(src: &str, language: &Language) -> Vec<&'static str> {
    let workspace = parse(src, language);
    (0..workspace.tokens.len())
        .filter(|&at| !language.is_trivia(workspace.tokens.kind[at]))
        .map(|at| language.name(workspace.tokens.kind[at]))
        .collect()
}

#[test]
fn an_error_stays_inside_its_construct() {
    // A bad declaration is an error inside the function's block, and the
    // class after it survives whole.
    assert_eq!(
        named("function f() { let = 1; }\nclass C {}\n", &JAVASCRIPT),
        "(program (function_declaration function identifier (formal_parameters) (statement_block (ERROR let) (ERROR) (expression_statement number))) (class_declaration class identifier (class_body)))"
    );
    // An unclosed parenthesis costs its own `if`, not the next method or statement.
    let tree = named("class A { m() { if (x { y(); } } n() {} }\nf();\n", &JAVASCRIPT);
    assert!(tree.contains("(parenthesized_expression UNCLOSED identifier (ERROR"), "{tree}");
    assert!(tree.ends_with("(method_definition identifier (formal_parameters) (statement_block)))) (expression_statement (call_expression identifier (arguments))))"), "{tree}");
    // A bad token inside an argument list stays in that list.
    assert_eq!(
        named("f(1, #, 2); g();\n", &JAVASCRIPT),
        "(program (expression_statement (call_expression identifier (arguments number (ERROR UNKNOWN) number))) (expression_statement (call_expression identifier (arguments))))"
    );
    // A missing value is one pair's error; the other pairs and the next statement stand.
    assert_eq!(
        named("let o = { a: 1, b: , c: 3 };\nh();\n", &JAVASCRIPT),
        "(program (lexical_declaration let (variable_declarator identifier (object (pair identifier number) (pair identifier (ERROR MISSING)) (pair identifier number)))) (expression_statement (call_expression identifier (arguments))))"
    );
    assert_eq!(
        named("interface I { a: ; b: string }\ntype T = number;\n", &TYPESCRIPT),
        "(program (interface_declaration interface identifier (object_type (property_signature identifier (type_annotation (ERROR MISSING))) (property_signature identifier (type_annotation identifier)))) (type_alias_declaration type identifier identifier))"
    );
}

#[test]
fn operators_nest_by_precedence() {
    let cases = [
        ("a + b * c", "(binary_expression identifier (binary_expression identifier identifier))"),
        ("a * b + c", "(binary_expression (binary_expression identifier identifier) identifier)"),
        ("a - b - c", "(binary_expression (binary_expression identifier identifier) identifier)"),
        ("a ** b ** c", "(binary_expression identifier (binary_expression identifier identifier))"),
        ("a ?? b || c", "(binary_expression identifier (binary_expression identifier identifier))"),
        ("a || b && c", "(binary_expression identifier (binary_expression identifier identifier))"),
        ("a >> b > c", "(binary_expression (binary_expression identifier identifier) identifier)"),
        ("x = y = z", "(assignment_expression identifier (assignment_expression identifier identifier))"),
        ("a ? b : c ? d : e", "(ternary_expression identifier identifier (ternary_expression identifier identifier identifier))"),
        ("(a, b) => a + b", "(arrow_function (formal_parameters (required_parameter identifier) (required_parameter identifier)) (binary_expression identifier identifier))"),
        ("async x => await x", "(arrow_function async identifier (await_expression await identifier))"),
        ("f?.(x)?.[y]?.z", "(member_expression (subscript_expression (call_expression identifier (arguments identifier)) (subscript identifier)) identifier)"),
        ("new Foo(1).bar", "(member_expression (new_expression new identifier (arguments number)) identifier)"),
        ("x = /re/g.test(s) / 2", "(assignment_expression identifier (binary_expression (call_expression (member_expression regex identifier) (arguments identifier)) number))"),
        ("!a instanceof B", "(binary_expression (unary_expression identifier) instanceof identifier)"),
        ("a, b = c", "(sequence_expression identifier (assignment_expression identifier identifier))"),
    ];
    for (src, want) in cases {
        assert_eq!(expression(src, &JAVASCRIPT), want, "{src:?}");
    }
}

#[test]
fn typescript_tells_generics_from_comparisons() {
    let cases = [
        ("f<T>(x)", "(call_expression identifier (type_arguments identifier) (arguments identifier))"),
        ("a < b && c > d", "(binary_expression (binary_expression identifier identifier) (binary_expression identifier identifier))"),
        ("<T,>(x: T) => x", "(arrow_function (type_parameters (type_parameter identifier)) (formal_parameters (required_parameter identifier (type_annotation identifier))) identifier)"),
        ("x as unknown as T", "(as_expression (as_expression identifier as identifier) as identifier)"),
        ("x!.y", "(member_expression (non_null_expression identifier) identifier)"),
    ];
    for (src, want) in cases {
        assert_eq!(expression(src, &TYPESCRIPT), want, "{src:?}");
    }
    assert!(clean("let m: Map<string, Set<number>> = new Map();\ntype F = <T>(x: T) => T extends string ? 1 : 2;\n", &TYPESCRIPT));
    assert!(clean("type K = keyof typeof o;\ntype L = T[number]['a'];\ntype N = import('x').Y<Z>;\nlet v = <T>x;\n", &TYPESCRIPT));
}

#[test]
fn declarations_classes_and_modules() {
    let ts = &TYPESCRIPT;
    assert!(clean("export default class A<T> extends B<T> implements I, J {\n  private readonly x: number = 1;\n  static #y?: string;\n  constructor(public a: T, protected b = 2) { super(); }\n  get z() { return this.x }\n  set z(v) {}\n  async *gen() { yield* g(); }\n  static { init(); }\n  [key: string]: unknown;\n  abstract m(): void;\n  declare d: D;\n}\n", ts));
    assert!(clean("import x, { a as b, type C } from './m';\nimport * as ns from 'n';\nimport type { T } from 't';\nimport fs = require('fs');\nexport { a, b as c } from 'd';\nexport * from 'e';\nexport = x;\nexport type { T };\n", ts));
    assert!(clean("declare module 'm' { export function f(): void; }\nnamespace N.M { export const x = 1; }\ndeclare global { interface Window { a: 1 } }\nconst enum E { A = 1, B }\nenum F { C }\nabstract class G {}\n", ts));
    assert!(clean("function f(this: Window, ...rest: number[]): asserts x is string {}\nfunction g<T extends object = {}>(x?: T): x is T;\nlet { a, b: [c, ...d] = [], ...e } = o;\n", ts));
    assert!(clean("label: for (const [k, v] of m) { if (k) continue label; else break }\nfor (let i = 0, j; i < n; i++) {}\nfor (x in o);\nfor await (const x of it) {}\ndo x++; while (x < 3)\nswitch (a) { case 1: case 2: f(); break; default: }\ntry { t() } catch { } finally { }\ntry {} catch ({ message }) {}\n", ts));
    // A contextual keyword is still a name where a declaration cannot start.
    assert!(clean("let type = 1, async = 2, get = 3, of = [];\ntype(of);\nasync(get);\nlet o = { get, set: 1, async() {}, get a() { return 1 }, static: 2 };\n", ts));
}

#[test]
fn line_ends_end_statements_where_asi_does() {
    assert_eq!(
        named("a\n++b\nlet x = 1\n[1, 2].map(f)\nreturn\n", &JAVASCRIPT),
        "(program (expression_statement identifier line_end) (expression_statement (update_expression identifier)) (lexical_declaration let (variable_declarator identifier number) line_end) (expression_statement (call_expression (member_expression (array number number) identifier) (arguments identifier))) (return_statement return line_end))"
    );
    // A line break inside an expression goes on with it.
    assert_eq!(
        named("x = a\n  + b\n  .c()\n", &JAVASCRIPT),
        "(program (expression_statement (assignment_expression identifier (binary_expression identifier (call_expression (member_expression identifier identifier) (arguments))))))"
    );
    // `break` then a name on the next line is not a label.
    assert!(named("while (1) { break\nfoo() }\n", &JAVASCRIPT).contains("(break_statement break line_end) (expression_statement (call_expression identifier (arguments)))"));
}

#[test]
fn lexing_regexes_templates_and_jsx() {
    assert_eq!(kinds("a / b / c", &JAVASCRIPT), ["identifier", "/", "identifier", "/", "identifier"]);
    assert_eq!(kinds("x = /a[/]b/g; y = !/c/.test(s)", &JAVASCRIPT), ["identifier", "=", "regex", ";", "identifier", "=", "!", "regex", ".", "identifier", "(", "identifier", ")"]);
    // A template's substitution is code: the object's own brace is a token
    // between the substitution's `${` and its `}`, and the nested template
    // opens and closes its own frames.
    assert_eq!(
        kinds("`a ${ {b: `c${d}`}.b } e` + 1", &JAVASCRIPT),
        [
            "template_start",
            "string_fragment",
            "template_substitution_open",
            "{",
            "identifier",
            ":",
            "template_start",
            "string_fragment",
            "template_substitution_open",
            "identifier",
            "template_substitution_close",
            "template_end",
            "}",
            ".",
            "identifier",
            "template_substitution_close",
            "string_fragment",
            "template_end",
            "+",
            "number",
        ]
    );
    assert_eq!(kinds("<div a=\"}\" b={c > d}>{'}'}<br/>{x && <i>{`${y}`}</i>}</div>;", &JAVASCRIPT), ["jsx_element", ";"]);
    assert_eq!(kinds("<T>x", &TYPESCRIPT), ["<", "identifier", ">", "identifier"]);
    assert_eq!(kinds("<Select<T | false> v={1} />", &TSX), ["jsx_element"]);
    assert_eq!(kinds("1_000n 0x1F .5e-3 1.", &JAVASCRIPT), ["number", "number", "number", "number"]);
    assert_eq!(kinds("#p 'it\\'s' \"q\"", &JAVASCRIPT), ["private_property_identifier", "string", "string"]);
}

/// Malformed inputs: each must report an error, and keep its root.
#[test]
fn malformed_inputs_report_errors() {
    let bad: [(&str, &Language); 18] = [
        ("function f( {}\n", &JAVASCRIPT),
        ("let x = ;\n", &JAVASCRIPT),
        ("if (a) else b;\n", &JAVASCRIPT),
        ("f(a b);\n", &JAVASCRIPT),
        ("let o = { a: 1 b: 2 };\n", &JAVASCRIPT),
        ("class { }\n", &JAVASCRIPT),
        ("x.;\n", &JAVASCRIPT),
        ("a + ;\n", &JAVASCRIPT),
        ("for (let i = 0; i < n) {}\n", &JAVASCRIPT),
        ("let s = 'unterminated;\nf();\n", &JAVASCRIPT),
        ("let t = `open ${ x;\n", &JAVASCRIPT),
        ("import { a from 'b';\n", &JAVASCRIPT),
        ("switch (x) { foo: 1 }\n", &JAVASCRIPT),
        ("let e = <div>;\n", &JAVASCRIPT),
        ("let x: = 1;\n", &TYPESCRIPT),
        ("interface I extends { }\n", &TYPESCRIPT),
        ("type T = A | ;\n", &TYPESCRIPT),
        ("function f<T(x: T) {}\n", &TSX),
    ];
    for (src, language) in bad {
        let workspace = parse(src, language);
        assert!(workspace.tree.root().flags & HAS_ERROR != 0, "{src:?} parsed clean: {}", workspace.tree.sexp(language, true));
        assert_eq!(workspace.tree.root().kind, PROGRAM);
    }
}

#[test]
fn deep_input_stays_within_the_budget() {
    on_big_stack(|| {
        let deep = [
            format!("x = {}1;", "(".repeat(100_000)),
            format!("x = {}y;", "-".repeat(100_000)),
            format!("x = {}y;", "!".repeat(100_000)),
            format!("x = {}b;", "a + ".repeat(100_000)),
            format!("x = {}y;", "a => ".repeat(50_000)),
            format!("x = {}0;", "a ? b : ".repeat(50_000)),
            format!("x = {}1{};", "[".repeat(50_000), "]".repeat(50_000)),
            format!("let x: {}T{};", "Array<".repeat(50_000), ">".repeat(50_000)),
            format!("{}", "if (a) ".repeat(50_000) + "b;"),
            format!("x = `{}`;", "${`".repeat(20_000)),
            "{".repeat(100_000),
        ];
        for src in deep {
            for language in [&JAVASCRIPT, &TYPESCRIPT] {
                let mut workspace = Workspace::default();
                let outcome = workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
                assert!(outcome.exhausted.is_none(), "{}: {}", &src[..20], outcome.describe());
                check(&workspace, src.as_bytes(), language);
                assert_eq!(workspace.tree.root().kind, PROGRAM);
            }
        }
    });
}

const SEEDS: &[&str] = &[
    "import { readFile } from 'fs/promises';\n\nexport class Cache {\n  #store = new Map();\n  constructor(limit = 100) { this.limit = limit }\n  async get(key) {\n    if (!this.#store.has(key)) {\n      const text = await readFile(key, 'utf8');\n      this.#store.set(key, JSON.parse(text));\n    }\n    return this.#store.get(key) ?? null\n  }\n}\n",
    "const sum = (xs) => xs.reduce((a, b) => a + b, 0)\nlet total = 0\nfor (const [i, x] of items.entries()) {\n  total += x.price * (i % 2 ? 1 : 0.5)\n  if (/^sale/i.test(x.name)) continue\n}\nconsole.log(`total: ${sum([total, 1])}`)\n",
    "export interface Node<T = unknown> {\n  kind: 'leaf' | 'branch';\n  children?: readonly Node<T>[];\n  value: T extends string ? number : T;\n}\n\nexport function walk<T>(node: Node<T>, visit: (n: Node<T>) => void): void {\n  visit(node);\n  for (const child of node.children ?? []) walk(child, visit);\n}\n",
    "export const App = ({ items }: Props) => {\n  const [open, setOpen] = useState<boolean>(false);\n  return (\n    <ul className=\"list\">\n      {items.map((x) => <li key={x.id} onClick={() => setOpen(!open)}>{x.label}</li>)}\n    </ul>\n  );\n};\n",
];

const JS_BYTES: &[u8] = b"()[]{}<>:;,.=+-*/%!&|^~?@#$'\"`\\_0123456789 \n\t xyfunctionletconstclassreturnifelseimportexporttypeinterfaceasyncawait=>${}\xc3\xa9";

#[test]
fn fuzzed_javascript_terminates_and_keeps_every_invariant() {
    on_big_stack(|| {
        let mut rng = Rng(0x5910_0004_006a_73ff);
        let mut workspace = Workspace::default();
        let budget = Budget::default();
        let mut erroneous = 0;
        const ROUNDS: usize = 3000;
        for round in 0..ROUNDS {
            let language: &Language = [&JAVASCRIPT, &TYPESCRIPT, &TSX][round % 3];
            let mut src = SEEDS[round % SEEDS.len()].as_bytes().to_vec();
            for _ in 0..1 + rng.below(6) {
                let length = src.len();
                mutate(&mut rng, &mut src, JS_BYTES, b"", 0, length);
            }
            let outcome = workspace.parse(&src, language, &budget).unwrap();
            assert!(outcome.exhausted.is_none(), "budget ran out on {:?}", String::from_utf8_lossy(&src));
            check(&workspace, &src, language);
            assert_eq!(workspace.tree.root().kind, PROGRAM);
            if workspace.tree.root().flags & HAS_ERROR != 0 {
                erroneous += 1;
            }
        }
        assert!(erroneous > ROUNDS / 3, "{erroneous} of {ROUNDS}");
    });
    assert!(clean(SEEDS[0], &JAVASCRIPT) && clean(SEEDS[1], &JAVASCRIPT));
    assert!(clean(SEEDS[2], &TYPESCRIPT));
    assert!(clean(SEEDS[3], &TSX));
}
