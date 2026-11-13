//! Every fixture under `fixtures/`, parsed and checked against what it must be:
//!
//! - a valid file parses completely, its root is the language's root, and no
//!   node is an `ERROR` and no token is a bad token;
//! - a malformed file still produces a tree: the root survives, spans the whole
//!   file, keeps its kind, and carries the error *inside* it. That is the
//!   cascade rule: one broken construct costs its own subtree, never the file;
//! - the top-level node kinds are exactly the expected ones, so a grammar change
//!   that moves a construct out of the root is a test failure, not a surprise.
//!
//! The expectation tables list fixtures by name, so a fixture added without an
//! expectation fails the test instead of being silently skipped.

use std::fs;
use std::path::{Path, PathBuf};

use deep_grammars::deep::syntax::{Language, ERROR};
use deep_grammars::deep::tree::{HAS_ERROR, TOKEN};
use deep_grammars::deep::{Budget, Workspace};
use deep_grammars::javascript::{JAVASCRIPT, TSX, TYPESCRIPT};
use deep_grammars::python::PYTHON;

/// `(file, top-level node kinds)` for every valid fixture, in order.
const PYTHON_VALID: &[(&str, &str)] = &[
    ("assigned_lambda.py", "expression_statement expression_statement function_definition"),
    (
        "async_worker.py",
        "import_statement import_from_statement decorated_definition function_definition function_definition function_definition if_statement",
    ),
    (
        "cli_tool.py",
        "expression_statement import_statement import_statement import_statement import_from_statement function_definition function_definition function_definition function_definition if_statement",
    ),
    (
        "config_loader.py",
        "expression_statement import_statement import_statement import_from_statement expression_statement expression_statement function_definition function_definition function_definition function_definition",
    ),
    (
        "data_pipeline.py",
        "expression_statement import_from_statement import_statement import_from_statement import_from_statement import_from_statement decorated_definition function_definition function_definition if_statement",
    ),
    (
        "flask_api.py",
        "import_statement import_statement import_from_statement expression_statement expression_statement expression_statement expression_statement function_definition decorated_definition decorated_definition decorated_definition",
    ),
    ("fstring_fields.py", "expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement"),
    (
        "fstring_nested.py",
        "expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement",
    ),
    ("matrix_math.py", "import_from_statement class_definition function_definition"),
    ("models.py", "import_from_statement import_from_statement import_from_statement import_from_statement class_definition class_definition class_definition"),
    (
        "scraper.py",
        "expression_statement import_statement import_from_statement import_statement import_from_statement expression_statement function_definition function_definition function_definition if_statement",
    ),
    ("star_import.py", "import_from_statement import_from_statement function_definition"),
    ("test_units.py", "import_statement import_from_statement import_from_statement class_definition if_statement"),
    ("tuple_assignment.py", "expression_statement expression_statement expression_statement expression_statement function_definition"),
    (
        "type_utils.py",
        "import_from_statement expression_statement expression_statement expression_statement expression_statement class_definition function_definition function_definition function_definition function_definition type_alias_statement type_alias_statement",
    ),
];

const JAVASCRIPT_VALID: &[(&str, &str)] = &[
    ("cache.js", "export_statement export_statement"),
    ("dom_helpers.js", "export_statement export_statement export_statement export_statement export_statement export_statement"),
    ("esm_module.js", "import_statement import_statement lexical_declaration export_statement export_statement export_statement export_statement"),
    ("events.js", "lexical_declaration class_declaration function_declaration expression_statement"),
    (
        "express_server.js",
        "expression_statement lexical_declaration lexical_declaration lexical_declaration lexical_declaration expression_statement lexical_declaration lexical_declaration function_declaration expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement",
    ),
    // JSX: the tag parts are nodes, and each `{...}` is code again.
    ("jsx_attributes.jsx", "function_declaration function_declaration"),
    ("jsx_elements.jsx", "lexical_declaration lexical_declaration lexical_declaration lexical_declaration lexical_declaration lexical_declaration lexical_declaration"),
    ("jsx_nested.jsx", "lexical_declaration lexical_declaration"),
    ("jsx_types.tsx", "type_alias_declaration function_declaration lexical_declaration"),
    ("keyword_keys.js", "lexical_declaration lexical_declaration export_statement"),
    ("math_stats.js", "function_declaration function_declaration function_declaration lexical_declaration lexical_declaration lexical_declaration expression_statement"),
    ("node_stream.js", "lexical_declaration lexical_declaration lexical_declaration class_declaration function_declaration if_statement expression_statement"),
    ("reducers.js", "lexical_declaration export_statement export_statement export_statement"),
    ("regex_parse.js", "lexical_declaration lexical_declaration lexical_declaration function_declaration function_declaration function_declaration expression_statement"),
    // A template's substitution is code now: the fixture is the statements a
    // reader expects, not one literal swallowing the rest of a line.
    ("template_substitutions.js", "lexical_declaration lexical_declaration lexical_declaration lexical_declaration lexical_declaration function_declaration"),
    ("util_async.js", "lexical_declaration function_declaration function_declaration function_declaration function_declaration expression_statement"),
];

const TYPESCRIPT_VALID: &[(&str, &str)] = &[
    (
        "api_types.ts",
        "export_statement export_statement export_statement export_statement export_statement export_statement export_statement export_statement export_statement",
    ),
    ("collection.ts", "export_statement"),
    ("config.ts", "export_statement export_statement export_statement export_statement type_alias_declaration lexical_declaration function_declaration export_statement"),
    (
        "guards.ts",
        "export_statement export_statement export_statement export_statement export_statement export_statement export_statement export_statement export_statement export_statement",
    ),
    ("helpers.ts", "export_statement export_statement export_statement export_statement export_statement export_statement export_statement"),
    ("http_client.ts", "export_statement export_statement lexical_declaration export_statement"),
    ("parser.ts", "export_statement export_statement export_statement export_statement export_statement"),
    ("router.ts", "export_statement export_statement export_statement lexical_declaration function_declaration export_statement"),
    ("service.ts", "import_statement export_statement export_statement export_statement"),
    ("store.ts", "export_statement export_statement export_statement export_statement"),
    ("telemetry.ts", "export_statement export_statement export_statement export_statement class_declaration export_statement"),
    ("template_substitutions.ts", "export_statement export_statement"),
];

/// `(file, language, top-level node kinds)` for every malformed fixture.
const MALFORMED: &[(&str, &str, &str)] = &[
    ("broken_signature.js", "javascript", "function_declaration function_declaration"),
    ("broken_signature.py", "python", "expression_statement import_statement function_definition function_definition"),
    ("broken_signature.ts", "typescript", "export_statement export_statement"),
    ("jsx_broken_attr.js", "javascript", "lexical_declaration lexical_declaration"),
    ("jsx_unclosed_eof.js", "javascript", "lexical_declaration"),
    ("jsx_unclosed_pair.js", "javascript", "lexical_declaration"),
    ("jsx_unclosed_tag.js", "javascript", "lexical_declaration"),
    ("stray_keyword.js", "javascript", "function_declaration ERROR statement_block"),
    ("stray_keyword.py", "python", "expression_statement function_definition ERROR"),
    ("stray_keyword.ts", "typescript", "function_declaration ERROR statement_block"),
    ("unclosed_brace.js", "javascript", "lexical_declaration"),
    ("unclosed_brace.py", "python", "expression_statement expression_statement"),
    ("unclosed_brace.ts", "typescript", "lexical_declaration"),
    // A replacement field that cannot close: `f"{"` at a line end, an empty
    // field, a field whose quote is the string's own, and an escaped brace.
    ("unclosed_field.py", "python", "expression_statement expression_statement"),
    // A substitution with no expression and a template that runs out of file:
    // both are errors inside their statement, and the file still parses.
    ("unclosed_template.js", "javascript", "lexical_declaration lexical_declaration"),
    ("unterminated_string.js", "javascript", "lexical_declaration expression_statement"),
    ("unterminated_string.py", "python", "expression_statement expression_statement expression_statement"),
    ("unterminated_string.ts", "typescript", "lexical_declaration expression_statement"),
];

/// What one parse produced, as the tests compare it.
struct Parsed {
    complete: bool,
    root: &'static str,
    /// Nodes of a kind that is `ERROR`, or bad-token leaves: both are errors.
    errors: usize,
    /// The child nodes of the root, in order, punctuation leaves excluded.
    top: Vec<&'static str>,
    /// The error nodes anywhere below the root.
    nested_errors: usize,
}

fn parse(language: &'static Language, src: &[u8]) -> Parsed {
    let mut workspace = Workspace::default();
    let outcome = workspace.parse(src, language, &Budget::default()).unwrap_or_else(|_| panic!("{}: input too large", language.name));
    let nodes = &workspace.tree.nodes;
    let is_error = |index: usize| nodes[index].kind == ERROR || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0;
    let top = workspace
        .tree
        .children(0)
        .map(|child| &nodes[child as usize])
        .filter(|node| node.flags & TOKEN == 0)
        .map(|node| language.name(node.kind))
        .collect();
    Parsed {
        complete: outcome.complete(),
        root: language.name(workspace.tree.root().kind),
        errors: (0..nodes.len()).filter(|&index| is_error(index)).count(),
        top,
        nested_errors: (1..nodes.len()).filter(|&index| is_error(index)).count(),
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn files_in(folder: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures().join(folder))
        .unwrap_or_else(|error| panic!("fixtures/{folder}: {error}"))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn check_valid(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
    let on_disk = files_in(folder);
    let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
    assert_eq!(on_disk, listed, "fixtures/{folder}: the expectation table and the directory disagree");
    let mut block = 0;
    for (name, top) in table {
        let path = fixtures().join(folder).join(name);
        let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        assert!(!src.is_empty(), "{name}: an empty fixture proves nothing");
        let result = parse(language, &src);
        assert!(result.complete, "{name}: the parse did not finish");
        assert_eq!(result.root, language.name(language.root), "{name}: the root's kind changed");
        assert_eq!(result.errors, 0, "{name}: {} error nodes in a valid file:\n{}", result.errors, tree(&path, language));
        assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
        block += 1;
    }
    assert!(block >= 10, "fixtures/{folder}: only {block} valid files");
}

/// A malformed file: the root survives, spans the file, and carries the error
/// inside it.
fn check_malformed() {
    let on_disk = files_in("malformed");
    let listed: Vec<String> = MALFORMED.iter().map(|(name, _, _)| (*name).to_string()).collect();
    assert_eq!(on_disk, listed, "fixtures/malformed: the expectation table and the directory disagree");
    for (name, language, top) in MALFORMED {
        let language: &'static Language = match *language {
            "python" => &PYTHON,
            "javascript" => &JAVASCRIPT,
            "typescript" => &TYPESCRIPT,
            other => panic!("{name}: unknown language {other}"),
        };
        let path = fixtures().join("malformed").join(name);
        let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        let result = parse(language, &src);
        assert!(result.complete, "{name}: a malformed file must still produce a tree");
        assert_eq!(result.root, language.name(language.root), "{name}: the root did not survive");
        assert!(result.nested_errors > 0, "{name}: no error inside the root:\n{}", tree(&path, language));
        assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
    }
}

/// The tree as an S-expression, for the failure messages.
fn tree(path: &Path, language: &'static Language) -> String {
    let src = fs::read(path).unwrap();
    let mut workspace = Workspace::default();
    workspace.parse(&src, language, &Budget::default()).unwrap();
    workspace.tree.sexp(language, true)
}

#[test]
fn python_fixtures_parse_cleanly() {
    check_valid("python", &PYTHON, PYTHON_VALID);
}

#[test]
fn javascript_fixtures_parse_cleanly() {
    check_valid("javascript", &JAVASCRIPT, JAVASCRIPT_VALID);
}

#[test]
fn typescript_fixtures_parse_cleanly() {
    check_valid("typescript", &TYPESCRIPT, TYPESCRIPT_VALID);
}

#[test]
fn malformed_fixtures_keep_the_root_and_report_inside_it() {
    check_malformed();
}

#[test]
fn every_valid_fixture_uses_the_layout_language_for_python_only() {
    // The layout tokens are Python's; the other two languages have none, so a
    // renamed `newline`/`indent`/`dedent` cannot leak into them.
    assert!(PYTHON.layout.is_some());
    assert!(JAVASCRIPT.layout.is_none());
    assert!(TYPESCRIPT.layout.is_none());
    assert!(TSX.layout.is_none());
}

#[test]
fn the_three_languages_name_their_roots() {
    assert_eq!((PYTHON.name, PYTHON.name(PYTHON.root)), ("python", "module"));
    assert_eq!((JAVASCRIPT.name, JAVASCRIPT.name(JAVASCRIPT.root)), ("javascript", "program"));
    assert_eq!((TYPESCRIPT.name, TYPESCRIPT.name(TYPESCRIPT.root)), ("typescript", "program"));
    assert!(JAVASCRIPT.kinds.len() > 200, "a grammar this size has hundreds of kinds, not {}", JAVASCRIPT.kinds.len());
    assert_eq!(JAVASCRIPT.kinds.len(), TYPESCRIPT.kinds.len(), "TypeScript shares JavaScript's kind table");
}
