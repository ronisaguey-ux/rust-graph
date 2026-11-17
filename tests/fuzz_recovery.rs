//! Recovery never panics.
//!
//! A parser that is correct on valid input and crashes on the file someone is
//! editing at 2 a.m. is worse than one that is wrong, because the whole process
//! goes with it. This mutates real sources — byte flips, truncations,
//! insertions, deletions, random bytes — and requires every input to come back
//! as a tree or as a clean refusal. A panic anywhere fails the test by itself,
//! which is the point: nothing here catches one.

#![cfg(all(feature = "deep", feature = "parser"))]

#[path = "../src/deep/mod.rs"]
mod deep;

#[path = "../src/deep_integration.rs"]
mod deep_integration;

use deep_integration::Lang;

const SOURCES: [(&str, Lang); 14] = [
    ("fn f(x: u32) -> u32 {\n    let y = x + 1;\n    y\n}\n", Lang::Rust),
    ("def f(x):\n    y = x + 1\n    return y\n", Lang::Python),
    ("export function f(x) {\n  const y = x + 1;\n  return y;\n}\n", Lang::JavaScript),
    ("interface A {\n    f(x: string): number;\n}\n", Lang::TypeScript),
    ("{\"a\": [1, 2, {\"b\": null}], \"c\": true}\n", Lang::Json),
    // The C family, because a half-written header is the input that actually
    // arrives: a save in flight, a merge marker, a file cut at a buffer edge.
    (
        "#include <stddef.h>\n#define MAX(a, b) ((a) > (b) ? (a) : (b))\ntypedef struct point { int x, y; } point;\nstatic inline int sum(const point *p) { return p->x + p->y; }\n",
        Lang::C,
    ),
    (
        "#include <vector>\nclass Holder { public: std::vector<int> v; int first() const { return v.empty() ? 0 : v[0]; } };\n",
        Lang::Cpp,
    ),
    (
        "#import <Foundation/Foundation.h>\n@interface Counter : NSObject\n@property (nonatomic) int value;\n- (int)add:(int)x to:(int)y;\n@end\n",
        Lang::ObjC,
    ),
    (
        "@SuppressWarnings(\"unchecked\")\nclass Box<T> { private T v; T get() { return v; } }\n",
        Lang::Java,
    ),
    (
        "namespace N { class C { public string S => @\"a\" + \"b\"; public int M(int x) { return x switch { 0 => 1, _ => 2 }; } } }\n",
        Lang::CSharp,
    ),
    (
        "package main\nfunc f(xs []int) int {\n\ts := 0\n\tfor _, v := range xs {\n\t\ts += v\n\t}\n\treturn s\n}\n",
        Lang::Go,
    ),
    (
        "data class P(val x: Int) {\n    fun sum(other: P): Int = when (x) { 0 -> other.x; else -> x + other.x }\n}\n",
        Lang::Kotlin,
    ),
    (
        "object M { def f(xs: List[Int]): Int = xs match { case Nil => 0; case h :: t => h + f(t) } }\n",
        Lang::Scala,
    ),
    (
        "struct P { var x: Int\n    func sum(_ o: P) -> Int { return x + o.x } }\n",
        Lang::Swift,
    ),
];

/// A deterministic xorshift: the same mutations on every run and every machine,
/// so a failure can be reproduced from the seed alone.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }
}

fn mutate(source: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = source.to_vec();
    if out.is_empty() {
        return out;
    }
    match rng.below(5) {
        0 => {
            // Flip one byte into an arbitrary one.
            let at = rng.below(out.len());
            out[at] = (rng.next() & 0xff) as u8;
        }
        1 => {
            // Truncate anywhere.
            let at = rng.below(out.len());
            out.truncate(at);
        }
        2 => {
            // Insert a snippet that is likely to unbalance the grammar.
            let at = rng.below(out.len() + 1);
            let snippets: [&[u8]; 12] = [b"(", b")", b"{", b"}", b"[", b"]", b"\"", b"'", b"\n", b"=>", b"/*", b"\\"];
            let insert = snippets[rng.below(snippets.len())];
            out.splice(at..at, insert.iter().copied());
        }
        3 => {
            // Delete a run.
            let at = rng.below(out.len());
            let len = 1 + rng.below(8.min(out.len() - at));
            out.drain(at..at + len);
        }
        _ => {
            // Repeat a slice: the depth budget and the pair stack both get a
            // longer run to walk than the original file ever had.
            let at = rng.below(out.len());
            let len = 1 + rng.below(16.min(out.len() - at));
            let slice = out[at..at + len].to_vec();
            let times = 1 + rng.below(4);
            for _ in 0..times {
                out.splice(at..at, slice.iter().copied());
            }
        }
    }
    out
}

#[test]
fn mutated_input_never_panics() {
    let mut rng = Rng(0x5eed_1234_9abc_def0);
    let mut parses = 0usize;
    for (source, lang) in SOURCES {
        let bytes = source.as_bytes();
        for _ in 0..200 {
            let mutated = mutate(bytes, &mut rng);
            // The contract: a tree or nothing. `parse_file` returning `None` is
            // the clean refusal; a panic would take the test down here.
            if let Some(cst) = deep_integration::parse_file(&mutated, lang) {
                assert!(cst.nodes.len() >= 1, "a tree without a root: {mutated:?}");
                assert!(cst.root < cst.nodes.len() as u32, "root outside the arena: {mutated:?}");
            }
            parses += 1;
        }
    }
    assert_eq!(parses, 2800, "the sweep runs 200 mutations per language");
}

/// A header cut at every byte is still a file the tool has to answer for.
///
/// Editors write partial files and corpora carry them; the C family is where it
/// bites, because a `.h` truncated mid-declaration is a normal input to a scan
/// that walks a checkout being edited. The contract is the same as for mutation:
/// a tree with a valid root, or a refusal only when there is not enough source
/// to be a file at all.
#[test]
fn a_truncated_c_family_file_still_yields_a_tree() {
    const TARGETS: [(&str, Lang); 3] = [
        (
            "#ifndef POINT_H\n#define POINT_H\n#include <stddef.h>\n\
             typedef struct point { int x, y; } point;\n\
             static inline int sum(const point *p) { return p->x + p->y; }\n#endif\n",
            Lang::C,
        ),
        (
            "#import <Foundation/Foundation.h>\n\
             @interface Counter : NSObject\n@property (nonatomic) int value;\n\
             - (int)add:(int)x to:(int)y;\n@end\n",
            Lang::ObjC,
        ),
        (
            "namespace N { class C { public: int m(int x) { return x > 0 ? x : -x; } }; }\n",
            Lang::Cpp,
        ),
    ];
    for (source, lang) in TARGETS {
        let bytes = source.as_bytes();
        for cut in 0..=bytes.len() {
            let prefix = &bytes[..cut];
            match deep_integration::parse_file(prefix, lang) {
                Some(cst) => {
                    assert!(!cst.nodes.is_empty(), "a tree without a root at cut {cut} of {source:?}");
                    assert!(
                        cst.root < cst.nodes.len() as u32,
                        "root outside the arena at cut {cut} of {source:?}"
                    );
                }
                None => assert!(
                    cut < 8,
                    "a {cut}-byte prefix of {source:?} was refused instead of recovered"
                ),
            }
        }
    }
}
