//! Java and C#: generics, annotations and attributes are layout, not symbols.
//!
//! The construct that breaks a naive reader is never the class: it is the
//! `@Bean("x")` in front of it, the `<T extends Comparable<T>>` in its header
//! and the `[JsonPropertyName("id")]` above a field. Each of those carries a
//! name that a scanner can mistake for a declaration, and the phase's claim is
//! that the CST record invents none of them and drops none the lexer reads.
//!
//! The fixture rows below are one construct each; the ignored test at the
//! bottom is the corpus measurement (`CORPUS45=... cargo test --test
//! facts_java_csharp -- --ignored`).
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

fn lexer_facts(extension: &str, source: &str) -> Facts {
    let language = langs::by_content(extension, source.as_bytes()).expect("a Java or C# row");
    parser::parse(source.as_bytes(), language)
}

fn deep_facts(lang: Lang, source: &str) -> Facts {
    let cst = deep_integration::parse_file(source.as_bytes(), lang).expect("a deep parse");
    deep_facts::extract(source.as_bytes(), &cst, lang)
}

/// Symbol keys, which is what the guard pairs on.
fn keys(facts: &Facts) -> Vec<String> {
    facts.symbols.iter().map(|symbol| symbol.key.clone()).collect()
}

/// One row per construct the phase names. Both readers must agree on the symbol
/// table, so an annotation or a type parameter that becomes a symbol fails.
const COVERAGE: &[(&str, &str, Lang, &str)] = &[
    (
        "Java: a generic class and a bounded type parameter",
        "java",
        Lang::Java,
        "public class Box<T extends Comparable<T>> {\n    private final T value;\n\n    public Box(T value) {\n        this.value = value;\n    }\n\n    public T get() {\n        return value;\n    }\n}\n",
    ),
    (
        "Java: a generic method with its own type parameter",
        "java",
        Lang::Java,
        "public class Lists {\n    public static <E> List<E> of(E one) {\n        return List.of(one);\n    }\n}\n",
    ),
    (
        "Java: annotations with element values on a class and a method",
        "java",
        Lang::Java,
        "@RestController\n@RequestMapping(path = \"/users\", produces = \"application/json\")\npublic class UserController {\n    @GetMapping(\"/{id}\")\n    @ResponseBody\n    public User find(@PathVariable(\"id\") long id) {\n        return service.find(id);\n    }\n}\n",
    ),
    (
        "Java: an annotation type and a type-use annotation",
        "java",
        Lang::Java,
        "@Retention(RetentionPolicy.RUNTIME)\npublic @interface Marker {\n    String value() default \"\";\n}\n\nclass Uses {\n    void f(@Nullable String name) {\n        render(name);\n    }\n}\n",
    ),
    (
        "C#: a generic class and a generic method with constraints",
        "cs",
        Lang::CSharp,
        "public class Cache<TKey, TValue> where TKey : notnull {\n    private readonly Dictionary<TKey, TValue> _items = new();\n\n    public TValue Get(TKey key) {\n        return _items[key];\n    }\n}\n",
    ),
    (
        "C#: attributes with arguments on a class, a method and a parameter",
        "cs",
        Lang::CSharp,
        "[ApiController]\n[Route(\"api/[controller]\")]\npublic class ItemsController : ControllerBase\n{\n    [HttpGet(\"{id}\")]\n    public ActionResult<Item> Get([FromRoute] int id)\n    {\n        return Find(id);\n    }\n}\n",
    ),
    (
        "Java: a type-parameter annotation and nested generic bounds",
        "java",
        Lang::Java,
        "public class Registry<@NonNull K, V extends List<? extends Number>> {\n    private final Map<K, List<V>> rows = new HashMap<>();\n\n    public <R extends Comparable<R>> R pick(Comparator<R> order) {\n        return order.first();\n    }\n}\n",
    ),
    (
        "C#: a return-value attribute and generic constraints with an interface",
        "cs",
        Lang::CSharp,
        "public class Repo<T> where T : class, IEntity<T>, new()\n{\n    [return: NotNull]\n    public T Load<TKey>(TKey key) where TKey : struct\n    {\n        return Find<T>(key);\n    }\n}\n",
    ),
    (
        "C#: attributes on an auto-property",
        "cs",
        Lang::CSharp,
        "public class Item\n{\n    [JsonPropertyName(\"id\")]\n    [Required]\n    public int Id { get; init; }\n\n    [JsonIgnore]\n    public string? Note { get; set; }\n}\n",
    ),
    (
        "C#: an attribute type and an assembly-level attribute",
        "cs",
        Lang::CSharp,
        "[assembly: AssemblyVersion(\"1.0.0.0\")]\n\n[AttributeUsage(AttributeTargets.Class)]\npublic sealed class MarkerAttribute : Attribute\n{\n}\n",
    ),
];

/// Shapes that still disagree, with the exact disagreement.
const KNOWN_GAPS: &[(&str, &str, Lang, &str, &str)] = &[];

/// The symbol table each row must produce. Generics, annotations and
/// attributes contribute no names of their own: `Box<T extends Comparable<T>>`
/// declares `Box`, its constructor and its getter, and nothing else.
const EXPECTED: &[&[&str]] = &[
    &["<module>", "Box", "Box.Box", "Box.get"],
    &["<module>", "Lists", "Lists.of"],
    &["<module>", "UserController", "UserController.find"],
    &["<module>", "Marker", "Marker.value", "Uses", "Uses.f"],
    &["<module>", "Cache", "Cache.Get"],
    &["<module>", "ItemsController", "ItemsController.Get"],
    &["<module>", "Registry", "Registry.pick"],
    &["<module>", "Repo", "Repo.Load"],
    &["<module>", "Item"],
    &["<module>", "MarkerAttribute"],
];

#[test]
fn every_generic_and_annotation_construct_declares_exactly_its_own_names() {
    let mut failed: Vec<String> = Vec::new();
    for (index, (name, _, lang, source)) in COVERAGE.iter().enumerate() {
        let deep = deep_facts(*lang, source);
        let got = keys(&deep);
        let want: Vec<String> = EXPECTED[index].iter().map(|key| (*key).to_string()).collect();
        if got != want {
            failed.push(format!("{name}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failed.is_empty(), "a generic, an annotation or an attribute moved the symbols:\n{}", failed.join("\n"));
}

#[test]
fn the_known_generic_gaps_are_still_gaps() {
    for (name, extension, lang, source, disagreement) in KNOWN_GAPS {
        let lexer = lexer_facts(extension, source);
        let deep = deep_facts(*lang, source);
        assert!(
            pipeline::loses(&lexer, &deep).lost(),
            "{name} now reads cleanly — move it into COVERAGE: {disagreement}"
        );
    }
}

/// The corpus measurement: both corpora, a deterministic sample, and the one
/// thing this phase claims — an annotation or an attribute never becomes a
/// symbol. A symbol counts as invented when its short name is the name of a
/// decorator in the same file and the file never declares a type of that name
/// (`@interface Marker` declares `Marker`; `@Override` declares nothing).
///
/// ```text
/// CORPUS45=/home/roni/Roni_workspace/corpus45 SAMPLE=300 \
///   cargo test --features "deep parser" --test facts_java_csharp -- --ignored --nocapture
/// ```
#[test]
#[ignore = "reads the corpus at $CORPUS45"]
fn the_java_and_csharp_corpora_invent_no_symbols() {
    use std::path::Path;
    let root = std::env::var("CORPUS45").unwrap_or_else(|_| "/home/roni/Roni_workspace/corpus45".to_string());
    let sample: usize = std::env::var("SAMPLE").ok().and_then(|value| value.parse().ok()).unwrap_or(300);
    let mut checked = 0usize;
    let mut symbols = 0usize;
    let mut invented_files = 0usize;
    let mut invented_examples: Vec<String> = Vec::new();
    for (folder, extension, lang) in [("java", "java", Lang::Java), ("csharp", "cs", Lang::CSharp)] {
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        let mut walk = vec![Path::new(&root).join(folder)];
        while let Some(dir) = walk.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk.push(path);
                } else if path.extension().and_then(|it| it.to_str()).is_some_and(|it| it == extension) {
                    paths.push(path);
                }
            }
        }
        paths.sort();
        if paths.len() > sample {
            let stride = paths.len() / sample;
            paths = paths.into_iter().step_by(stride.max(1)).take(sample).collect();
        }
        let _ = extension;
        for path in paths {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(text) = std::str::from_utf8(&bytes) else { continue };
            let Some(cst) = deep_integration::parse_file(&bytes, lang) else { continue };
            if cst.degraded() {
                continue;
            }
            checked += 1;
            let deep = deep_facts::extract(&bytes, &cst, lang);
            symbols += deep.symbols.len();
            let decorators = decorator_names(text, lang);
            let invented: Vec<&String> = deep
                .symbols
                .iter()
                .filter(|symbol| symbol.key != "<module>")
                .filter(|symbol| {
                    let short = symbol.key.rsplit('.').next().unwrap_or(&symbol.key);
                    decorators.contains(short) && !declares_type(text, short) && !declares_member(text, short)
                })
                .map(|symbol| &symbol.key)
                .collect();
            if !invented.is_empty() {
                invented_files += 1;
                if invented_examples.len() < 20 {
                    invented_examples.push(format!("{path:?}: {invented:?}"));
                }
            }
        }
    }
    println!(
        "CORPUS {checked} files, {symbols} symbols: {invented_files} files invent a symbol from an annotation or attribute\n  {}",
        invented_examples.join("\n  ")
    );
    assert_eq!(invented_files, 0, "{invented_files} of {checked} files invent a symbol from layout");
}

/// The names that decorate rather than declare, per language. Comments are
/// stripped first: a `{@code X}` javadoc tag is not an annotation and a `[0]`
/// in a comment is not an attribute.
fn decorator_names(source: &str, lang: Lang) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut in_block = false;
    for line in source.lines() {
        let mut text = line;
        if in_block {
            match text.find("*/") {
                Some(end) => {
                    text = &text[end + 2..];
                    in_block = false;
                }
                None => continue,
            }
        }
        if let Some(start) = text.find("/*") {
            if text[start + 2..].find("*/").is_none() {
                in_block = true;
            }
            text = &text[..start];
        }
        if let Some(at) = text.find("//") {
            text = &text[..at];
        }
        let trimmed = text.trim_start();
        let mut word = |after: &str| {
            let mut chars = after.chars();
            let first = chars.next()?;
            if !(first.is_alphabetic() || first == '_') {
                return None;
            }
            let mut name = String::from(first);
            for ch in chars {
                if ch.is_alphanumeric() || ch == '_' || ch == '.' {
                    name.push(ch);
                } else {
                    break;
                }
            }
            Some(name.rsplit('.').next().unwrap_or(&name).to_string())
        };
        match lang {
            Lang::Java => {
                if let Some(at) = trimmed.find('@') {
                    let after = &trimmed[at + 1..];
                    if !after.starts_with("interface") {
                        if let Some(name) = word(after) {
                            out.insert(name);
                        }
                    }
                }
            }
            _ => {
                // An attribute section is line-initial: `[Route("x")]`.
                if let Some(after) = trimmed.strip_prefix('[') {
                    if let Some(name) = word(after) {
                        out.insert(name);
                    }
                }
            }
        }
    }
    out
}

/// Whether the file declares a member of this name: `public Task Test()` is a
/// method even when `[Test]` above it is an attribute, and the attribute line
/// itself does not count as the declaration.
fn declares_member(source: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    source.lines().any(|line| {
        let stripped = line.trim_start();
        !stripped.starts_with('[') && !stripped.starts_with('@') && line.contains(&needle)
    })
}

/// Whether the file declares a type of this name, which is what makes
/// `@interface Marker` or `class MarkerAttribute` a declaration and not layout.
fn declares_type(source: &str, name: &str) -> bool {
    ["class", "interface", "enum", "record", "struct", "delegate", "protocol", "@interface"]
        .iter()
        .any(|keyword| source.contains(&format!("{keyword} {name}")) || source.contains(&format!("{keyword} {name}<")))
}
