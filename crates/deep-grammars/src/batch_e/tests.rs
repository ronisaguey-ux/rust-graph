//! Fixtures for `fixtures/longtail2/`: a valid file parses completely with no
//! error node and exactly the listed top-level kinds; a malformed one keeps its
//! root and carries at least one error inside it.
//!
//! The tables are explicit so a fixture added without an expectation fails
//! instead of being skipped, which is the same contract as `tests/longtail.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::deep::syntax::{Language, ERROR};
use crate::deep::tree::{HAS_ERROR, TOKEN};
use crate::deep::{Budget, Workspace};

use super::crystal::CRYSTAL;
use super::d::D;
use super::haskell::HASKELL;
use super::nim::NIM;
use super::ocaml::OCAML;

/// `(language name, static, valid fixtures)`.
fn languages() -> Vec<(&'static str, &'static Language, &'static [(&'static str, &'static str)])> {
    vec![
        ("ocaml", &OCAML, OCAML_VALID),
        ("crystal", &CRYSTAL, CRYSTAL_VALID),
        ("d", &D, D_VALID),
        ("nim", &NIM, NIM_VALID),
        ("haskell", &HASKELL, HASKELL_VALID),

    ]
}

const OCAML_VALID: &[(&str, &str)] = &[
    ("config.ml", "type_definition type_definition let_definition let_definition let_definition let_definition"),
    ("scan.ml", "module_type_definition module_definition class_definition"),
    ("server.ml", "open_statement module_definition external_definition let_definition let_definition"),
];

const D_VALID: &[(&str, &str)] = &[
    ("build.d", "module_declaration import_declaration import_declaration import_declaration enum_definition aggregate_definition function_definition function_definition"),
    ("scanner.d", "module_declaration import_declaration import_declaration alias_declaration enum_definition template_definition aggregate_definition function_definition function_definition"),
    ("server.d", "module_declaration import_declaration aggregate_definition function_definition"),
];

const CRYSTAL_VALID: &[(&str, &str)] = &[
    ("scanner.cr", "require_statement module_definition"),
    ("server.cr", "require_statement module_definition expression_statement expression_statement if_statement expression_statement"),
    ("store.cr", "require_statement module_definition lib_definition expression_statement expression_statement expression_statement while_statement"),
];

const NIM_VALID: &[(&str, &str)] = &[
    ("scanner.nim", "import_statement import_statement type_section const_section routine_definition routine_definition routine_definition routine_definition"),
    ("server.nim", "import_statement import_statement from_statement include_statement type_section const_section var_section routine_definition routine_definition routine_definition routine_definition when_statement"),
    ("store.nim", "import_statement import_statement type_section routine_definition routine_definition routine_definition routine_definition routine_definition block_statement try_statement"),
];

const HASKELL_VALID: &[(&str, &str)] = &[
    ("Main.hs", "module_header declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration"),
    ("Scan.hs", "module_header declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration"),
    ("Server.hs", "module_header declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration declaration"),
];

/// `(file, language name, top-level kinds)` for every malformed fixture.
const MALFORMED: &[(&str, &str, &str)] = &[
    ("crystal_broken.cr", "crystal", "require_statement class_definition class_definition"),
    ("d_broken.d", "d", "module_declaration aggregate_definition function_definition"),
    ("haskell_broken.hs", "haskell", "module_header declaration declaration declaration declaration declaration declaration"),
    ("nim_broken.nim", "nim", "import_statement type_section routine_definition routine_definition"),
    ("ocaml_broken.ml", "ocaml", "type_definition let_definition let_definition"),

];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/longtail2")
}

fn files_in(folder: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures().join(folder))
        .unwrap_or_else(|error| panic!("fixtures/longtail2/{folder}: {error}"))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

struct Parsed {
    complete: bool,
    root: &'static str,
    errors: usize,
    top: Vec<&'static str>,
    nested_errors: usize,
}

fn parse(language: &'static Language, src: &[u8]) -> Parsed {
    let mut workspace = Workspace::default();
    let outcome = workspace
        .parse(src, language, &Budget::default())
        .unwrap_or_else(|_| panic!("{}: input too large", language.name));
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

fn check_valid(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
    let on_disk = files_in(folder);
    let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
    assert_eq!(on_disk, listed, "fixtures/longtail2/{folder}: the table and the directory disagree");
    assert!(table.len() >= 3, "fixtures/longtail2/{folder}: at least three valid files");
    for (name, top) in table {
        let path = fixtures().join(folder).join(name);
        let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        assert!(!src.is_empty(), "{name}: an empty fixture proves nothing");
        let result = parse(language, &src);
        assert!(result.complete, "{name}: the parse did not finish");
        assert_eq!(result.root, language.name(language.root), "{name}: the root's kind changed");
        assert_eq!(result.errors, 0, "{name}: {} error nodes in a valid file:\n{}", result.errors, se(&path, language));
        assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
        assert!(!result.top.is_empty(), "{name}: the tree has no top-level node");
    }
}

/// The tree as an S-expression, for the failure messages.
fn se(path: &Path, language: &'static Language) -> String {
    let src = fs::read(path).unwrap();
    let mut workspace = Workspace::default();
    workspace.parse(&src, language, &Budget::default()).unwrap();
    workspace.tree.sexp(language, true)
}

#[test]
fn longtail2_valid_fixtures_parse_cleanly() {
    for (folder, language, table) in languages() {
        check_valid(folder, language, table);
    }
}

#[test]
fn longtail2_malformed_fixtures_keep_the_root() {
    let on_disk = files_in("malformed");
    let listed: Vec<String> = MALFORMED.iter().map(|(name, _, _)| (*name).to_string()).collect();
    assert_eq!(on_disk, listed, "fixtures/longtail2/malformed: the table and the directory disagree");
    for (name, language, top) in MALFORMED {
        let language = languages()
            .into_iter()
            .map(|(_, language, _)| language)
            .find(|candidate| candidate.name == *language)
            .unwrap_or_else(|| panic!("{name}: unknown language {language}"));
        let path = fixtures().join("malformed").join(name);
        let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        let result = parse(language, &src);
        assert!(result.complete, "{name}: a malformed file must still produce a tree");
        assert_eq!(result.root, language.name(language.root), "{name}: the root did not survive");
        assert!(result.nested_errors > 0, "{name}: no error inside the root:\n{}", se(&path, language));
        assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
    }
}

/// The tokens the grammar sees: trivia and layout-only bytes dropped.
fn significant(language: &'static Language, src: &str) -> Vec<&'static str> {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap_or_else(|_| panic!("{}: too large", language.name));
    (0..workspace.tokens.len())
        .filter(|&at| !language.is_trivia(workspace.tokens.kind[at]))
        .map(|at| language.name(workspace.tokens.kind[at]))
        .collect()
}

/// Every token, trivia included, by kind name.
fn all_tokens(language: &'static Language, src: &str) -> Vec<&'static str> {
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap_or_else(|_| panic!("{}: too large", language.name));
    (0..workspace.tokens.len()).map(|at| language.name(workspace.tokens.kind[at])).collect()
}

#[test]
fn ocaml_lexes_comments_strings_characters_and_type_variables() {
    assert_eq!(
        significant(&OCAML, "let s = \"a\\n\" and c = 'x' and t : 'a list = []"),
        [
            "let", "identifier", "=", "string", "and", "identifier", "=", "char", "and", "identifier",
            ":", "type_variable", "identifier", "=", "[", "]"
        ]
    );
    assert_eq!(
        significant(&OCAML, "(* outer (* inner *) still *) 0x1F 1_000 3.5e-2"),
        ["number", "number", "number"]
    );
    assert_eq!(all_tokens(&OCAML, "x (* note *)"), ["identifier", "whitespace", "comment"]);
}

#[test]
fn d_lexes_every_string_form_and_both_block_comments() {
    assert_eq!(
        significant(&D, "auto a = \"x\"; auto b = r\"c:\\temp\"; auto c = `raw`;"),
        [
            "auto", "identifier", "=", "string", ";", "auto", "identifier", "=", "string", ";",
            "auto", "identifier", "=", "string", ";"
        ]
    );
    assert_eq!(significant(&D, "/+ outer /+ inner +/ still +/ 0x1Fu 1_000.5f"), ["number", "number"]);
    assert_eq!(all_tokens(&D, "x // note"), ["identifier", "whitespace", "comment"]);
}

#[test]
fn crystal_lexes_interpolation_symbols_and_regexes() {
    assert_eq!(
        significant(&CRYSTAL, "name = \"a#{b}c\"; sym = :ready; re = /ab+c/;"),
        [
            "identifier", "=", "string", ";", "identifier", "=", "symbol", ";", "identifier", "=",
            "regex", ";"
        ]
    );
    assert_eq!(significant(&CRYSTAL, "@count += 1; @@all = @count"), ["instance_variable", "+=", "number", ";", "class_variable", "=", "instance_variable"]);
    assert_eq!(significant(&CRYSTAL, "%w[one two] 'single'"), ["string", "string"]);
}

#[test]
fn nim_lexes_pragmas_triple_strings_and_backtick_names() {
    assert_eq!(
        significant(&NIM, "proc f() {.inline.} = discard"),
        ["proc", "identifier", "(", ")", "pragma", "=", "discard", "newline"]
    );
    assert_eq!(significant(&NIM, "let s = \"\"\"a\nb\"\"\""), ["let", "identifier", "=", "string", "newline"]);
    assert_eq!(significant(&NIM, "let r = r\"c:\\temp\"  # note"), ["let", "identifier", "=", "string", "newline"]);
    assert_eq!(
        significant(&NIM, "proc `+`(a, b: int)"),
        ["proc", "operator_name", "(", "identifier", ",", "identifier", ":", "identifier", ")", "newline"]
    );
}

#[test]
fn haskell_lexes_operator_runs_and_nested_comments() {
    assert_eq!(
        significant(&HASKELL, "f = a >>= b <*> c"),
        ["variable", "=", "variable", "operator", "variable", "operator", "variable", "newline"]
    );
    assert_eq!(significant(&HASKELL, "{- a {- b -} c -}\nx = 'q'"), ["variable", "=", "char", "newline"]);
    assert_eq!(significant(&HASKELL, "Data.List.sort xs"), ["qualified_name", "variable", "newline"]);
    assert_eq!(all_tokens(&HASKELL, "-- note"), ["comment"]);
}

/// Prints the top-level kinds of every fixture, so a new fixture's row can be
/// written from what the grammar actually produced rather than guessed.
#[test]
#[ignore]
fn dump_top_level_kinds() {
    for (folder, language, _) in languages() {
        for name in files_in(folder) {
            let path = fixtures().join(folder).join(&name);
            let src = fs::read(&path).unwrap();
            let result = parse(language, &src);
            println!("{folder}/{name}: complete={} errors={} top={}", result.complete, result.errors, result.top.join(" "));
        }
    }
    for (name, language, _) in MALFORMED {
        let language = languages().into_iter().map(|(_, language, _)| language).find(|candidate| candidate.name == *language).unwrap();
        let path = fixtures().join("malformed").join(name);
        let src = fs::read(&path).unwrap();
        let result = parse(language, &src);
        println!("malformed/{name}: complete={} nested_errors={} top={}", result.complete, result.nested_errors, result.top.join(" "));
    }
}

#[test]
#[ignore]
fn dump_sexp() {
    let name = std::env::var("FIXTURE").unwrap();
    let folder = std::env::var("FOLDER").unwrap();
    let language = languages().into_iter().find(|(f, _, _)| *f == folder).unwrap().1;
    println!("{}", se(&fixtures().join(folder).join(name), language));
}

#[test]
#[ignore]
fn probe() {
    let src = std::env::var("SRC").unwrap();
    let name = std::env::var("LANG").unwrap_or_else(|_| "ocaml".to_string());
    let language = languages().into_iter().find(|(f, _, _)| *f == name).unwrap().1;
    let mut workspace = Workspace::default();
    workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
    println!("{}", workspace.tree.sexp(language, true));
}
