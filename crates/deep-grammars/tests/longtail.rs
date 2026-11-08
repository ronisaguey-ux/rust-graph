//! Fixtures for the long-tail grammars under `fixtures/longtail/`.
//!
//! Same contract as `tests/fixtures.rs`: a valid file parses completely with no
//! error node and the listed top-level kinds; a malformed one keeps its root
//! and carries the error inside it. The tables are explicit so a fixture added
//! without an expectation fails instead of being skipped.

use std::fs;
use std::path::{Path, PathBuf};

use deep_grammars::deep::syntax::{Language, ERROR};
use deep_grammars::deep::tree::{HAS_ERROR, TOKEN};
use deep_grammars::deep::{Budget, Workspace};

use deep_grammars::batch_d::json::JSON;
use deep_grammars::batch_d::shell::SHELL;
use deep_grammars::batch_d::sql::SQL;
use deep_grammars::batch_d::yaml::YAML;
use deep_grammars::batch_d::css::CSS;
use deep_grammars::batch_d::html::HTML;
use deep_grammars::batch_d::lua::LUA;

/// `(language name, static, valid fixtures)`.
fn languages() -> Vec<(&'static str, &'static Language, &'static [(&'static str, &'static str)])> {
    vec![
        ("json", &JSON, JSON_VALID),
        ("shell", &SHELL, SHELL_VALID),
        ("sql", &SQL, SQL_VALID),
        ("yaml", &YAML, YAML_VALID),
        ("html", &HTML, HTML_VALID),
        ("css", &CSS, CSS_VALID),
        ("lua", &LUA, LUA_VALID),
    ]
}

const JSON_VALID: &[(&str, &str)] = &[
    ("graph.json", "object"),
    ("package.json", "object"),
    ("records.json", "array"),
];

const SHELL_VALID: &[(&str, &str)] = &[
    (
        "build.sh",
        "simple_command subshell function_definition function_definition for_clause subshell simple_command simple_command simple_command simple_command",
    ),
    (
        "deploy.sh",
        "simple_command simple_command simple_command simple_command paren_function paren_function if_clause simple_command paren_function paren_function for_clause if_clause simple_command simple_command",
    ),
    (
        "rotate.sh",
        "simple_command simple_command simple_command if_clause simple_command while_clause simple_command until_clause simple_command simple_command",
    ),
];

const SQL_VALID: &[(&str, &str)] = &[
    ("analytics.sql", "with_statement"),
    ("mutations.sql", "insert_statement insert_statement update_statement delete_statement select_statement"),
    (
        "schema.sql",
        "create_table_statement create_table_statement create_index_statement alter_statement create_view_statement",
    ),
];

const YAML_VALID: &[(&str, &str)] = &[
    ("compose.yaml", "key_entry key_entry key_entry"),
    ("config.yaml", "key_entry key_entry"),
    ("deploy.yaml", "key_entry key_entry key_entry key_entry"),
];

const HTML_VALID: &[(&str, &str)] = &[
    ("dashboard.html", "doctype_node element"),
    ("form.html", "element"),
    ("snippet.html", "element"),
];

const CSS_VALID: &[(&str, &str)] = &[
    (
        "app.css",
        "at_rule rule_set rule_set rule_set rule_set rule_set at_rule at_rule rule_set",
    ),
    ("print.css", "at_rule at_rule"),
    ("tokens.css", "rule_set rule_set rule_set rule_set rule_set"),
];

const LUA_VALID: &[(&str, &str)] = &[
    (
        "config.lua",
        "local_declaration local_declaration local_function local_function return_statement",
    ),
    (
        "init.lua",
        "local_declaration local_declaration local_declaration local_declaration function_declaration function_declaration function_declaration return_statement",
    ),
    (
        "parser.lua",
        "local_declaration local_declaration expression_statement local_function function_declaration function_declaration function_declaration function_declaration return_statement",
    ),
];

/// `(file, language name, top-level kinds)` for every malformed fixture.
const MALFORMED: &[(&str, &str, &str)] = &[
    ("css_broken.css", "css", "rule_set rule_set"),
    ("lua_broken.lua", "lua", "local_function"),
    ("html_broken.html", "html", "element"),
    ("json_broken_array.json", "json", "object"),
    ("shell_broken_if.sh", "shell", "simple_command if_clause ERROR"),
    ("sql_broken_insert.sql", "sql", "select_statement insert_statement"),
    ("yaml_broken.yaml", "yaml", "key_entry"),
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/longtail")
}

fn files_in(folder: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures().join(folder))
        .unwrap_or_else(|error| panic!("fixtures/longtail/{folder}: {error}"))
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

fn check_valid(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
    let on_disk = files_in(folder);
    let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
    assert_eq!(on_disk, listed, "fixtures/longtail/{folder}: the table and the directory disagree");
    assert!(table.len() >= 3, "fixtures/longtail/{folder}: at least three valid files");
    for (name, top) in table {
        let path = fixtures().join(folder).join(name);
        let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        assert!(!src.is_empty(), "{name}: an empty fixture proves nothing");
        let result = parse(language, &src);
        assert!(result.complete, "{name}: the parse did not finish");
        assert_eq!(result.root, language.name(language.root), "{name}: the root's kind changed");
        assert_eq!(result.errors, 0, "{name}: {} error nodes in a valid file:\n{}", result.errors, se(&path, language));
        assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
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
fn longtail_valid_fixtures_parse_cleanly() {
    for (folder, language, table) in languages() {
        check_valid(folder, language, table);
    }
}

#[test]
fn longtail_malformed_fixtures_keep_the_root() {
    let on_disk = files_in("malformed");
    let listed: Vec<String> = MALFORMED.iter().map(|(name, _, _)| (*name).to_string()).collect();
    assert_eq!(on_disk, listed, "fixtures/longtail/malformed: the table and the directory disagree");
    for (name, language, top) in MALFORMED {
        let language = languages().into_iter().map(|(_, language, _)| language).find(|candidate| candidate.name == *language).unwrap_or_else(|| panic!("{name}: unknown language {language}"));
        let path = fixtures().join("malformed").join(name);
        let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        let result = parse(language, &src);
        assert!(result.complete, "{name}: a malformed file must still produce a tree");
        assert_eq!(result.root, language.name(language.root), "{name}: the root did not survive");
        assert!(result.nested_errors > 0, "{name}: no error inside the root:\n{}", se(&path, language));
        assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
    }
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
