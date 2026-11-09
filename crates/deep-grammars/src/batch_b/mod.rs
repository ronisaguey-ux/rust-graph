//! Batch B of the deep scan's grammars: Ruby, PHP, Swift and Scala.
//!
//! Each language is one `grammar!` definition plus a hand-written step lexer in
//! its own module. The tests at the bottom of this file are the contract every
//! language shares: its fixtures parse completely with no error node, its
//! malformed fixtures keep the root and report the error inside it, and the
//! top-level kinds of each file are exactly what the table says.
//!
//! Fixtures live in `fixtures/<language>/` (valid) and
//! `fixtures/<language>/malformed/` (malformed), so a language's malformed
//! files sit with its valid ones and cannot collide with the three other
//! languages' files in the shared `fixtures/malformed/` folder.

pub mod php;
pub mod ruby;
pub mod scala;
pub mod swift;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::deep::syntax::{Language, ERROR};
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    use crate::batch_b::php::PHP;
    use crate::batch_b::ruby::RUBY;
    use crate::batch_b::scala::SCALA;
    use crate::batch_b::swift::SWIFT;

    /// `(file, top-level node kinds)` for every valid fixture, in order.
    const RUBY_VALID: &[(&str, &str)] = &[
        (
            "collections.rb",
            "expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement",
        ),
        (
            "control_flow.rb",
            "method_definition method_definition expression_statement while_statement until_statement for_statement case_statement expression_statement expression_statement",
        ),
        ("exceptions.rb", "method_definition method_definition expression_statement expression_statement"),
        ("modules.rb", "require_statement require_statement require_statement module_definition module_definition class_definition"),
        (
            "numeric.rb",
            "expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement",
        ),
        ("service.rb", "require_statement class_definition expression_statement expression_statement expression_statement expression_statement"),
        ("shapes.rb", "module_definition class_definition class_definition"),
        (
            "strings.rb",
            "expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement",
        ),
    ];

    /// `(file, top-level node kinds)` for every malformed fixture.
    const RUBY_MALFORMED: &[(&str, &str)] = &[
        ("broken_string.rb", "class_definition ERROR"),
        ("stray_keyword.rb", "method_definition ERROR method_definition"),
        ("unclosed_method.rb", "class_definition"),
    ];

        /// `(file, top-level node kinds)` for every valid PHP fixture.
    const PHP_VALID: &[(&str, &str)] = &[
        (
            "arrays.php",
            "expression_statement expression_statement expression_statement expression_statement foreach_statement expression_statement expression_statement expression_statement unset_statement expression_statement expression_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement",
        ),
        ("classes.php", "namespace_declaration trait_declaration enum_declaration class_declaration"),
        (
            "control_flow.php",
            "expression_statement if_statement expression_statement while_statement do_statement for_statement switch_statement while_statement",
        ),
        ("functions.php", "expression_statement expression_statement function_declaration function_declaration expression_statement echo_statement echo_statement"),
        (
            "operators.php",
            "expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement echo_statement",
        ),
        ("service.php", "namespace_declaration use_declaration use_declaration class_declaration"),
        ("shapes.php", "namespace_declaration use_declaration interface_declaration class_declaration class_declaration"),
        (
            "strings.php",
            "expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement echo_statement echo_statement echo_statement echo_statement echo_statement",
        ),
    ];

    /// `(file, top-level node kinds)` for every malformed PHP fixture.
    const PHP_MALFORMED: &[(&str, &str)] = &[
        ("broken_string.php", "class_declaration"),
        ("missing_semicolon.php", "function_declaration"),
        ("unclosed_class.php", "class_declaration"),
    ];

    /// `(file, top-level node kinds)` for every valid Swift fixture.
    const SWIFT_VALID: &[(&str, &str)] = &[
        (
            "collections.swift",
            "property_declaration expression_statement property_declaration property_declaration property_declaration property_declaration property_declaration property_declaration property_declaration property_declaration expression_statement expression_statement expression_statement expression_statement expression_statement expression_statement",
        ),
        (
            "control_flow.swift",
            "function_declaration function_declaration property_declaration while_statement repeat_statement for_statement switch_statement defer_statement",
        ),
        ("enums.swift", "enum_declaration function_declaration"),
        (
            "functions.swift",
            "import_declaration function_declaration function_declaration function_declaration property_declaration property_declaration property_declaration property_declaration expression_statement expression_statement expression_statement expression_statement expression_statement",
        ),
        ("generics.swift", "struct_declaration extension_declaration function_declaration"),
        ("models.swift", "import_declaration protocol_declaration struct_declaration class_declaration enum_declaration"),
        ("service.swift", "import_declaration protocol_declaration class_declaration function_declaration"),
        (
            "strings.swift",
            "property_declaration property_declaration property_declaration property_declaration property_declaration property_declaration property_declaration expression_statement expression_statement expression_statement expression_statement expression_statement",
        ),
    ];

    /// `(file, top-level node kinds)` for every malformed Swift fixture.
    const SWIFT_MALFORMED: &[(&str, &str)] = &[
        ("broken_string.swift", "property_declaration"),
        ("stray_brace.swift", "function_declaration ERROR"),
        ("unclosed_function.swift", "struct_declaration"),
    ];

    /// `(file, top-level node kinds)` for every valid Scala fixture.
    const SCALA_VALID: &[(&str, &str)] = &[
        ("collections.scala", "object_definition"),
        ("control_flow.scala", "object_definition"),
        ("functions.scala", "object_definition"),
        ("models.scala", "package_clause import_declaration import_declaration trait_definition class_definition class_definition object_definition"),
        ("pattern_match.scala", "object_definition"),
        ("service.scala", "package_clause import_declaration import_declaration class_definition class_definition object_definition"),
        ("strings.scala", "object_definition"),
        ("traits.scala", "trait_definition class_definition class_definition object_definition"),
    ];

    /// `(file, top-level node kinds)` for every malformed Scala fixture.
    const SCALA_MALFORMED: &[(&str, &str)] = &[
        ("broken_string.scala", "object_definition"),
        ("stray_brace.scala", "object_definition ERROR"),
        ("unclosed_class.scala", "class_definition"),
    ];

/// What one parse produced, as the tests compare it.
    struct Parsed {
        complete: bool,
        root: &'static str,
        /// `ERROR` nodes and bad-token leaves, anywhere in the tree.
        errors: usize,
        /// The same, below the root.
        nested_errors: usize,
        /// The child nodes of the root, in order, punctuation leaves excluded.
        top: Vec<&'static str>,
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
            nested_errors: (1..nodes.len()).filter(|&index| is_error(index)).count(),
            top,
        }
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    fn files_in(folder: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(fixtures().join(folder))
            .unwrap_or_else(|error| panic!("fixtures/{folder}: {error}"))
            .filter(|entry| entry.as_ref().is_ok_and(|entry| entry.path().is_file()))
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A valid file: a complete parse, the language's root, no error anywhere,
    /// and exactly the listed top-level nodes.
    fn check_valid(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
        let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
        assert_eq!(files_in(folder), listed, "fixtures/{folder}: the expectation table and the directory disagree");
        for (name, top) in table {
            let path = fixtures().join(folder).join(name);
            let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            assert!(!src.is_empty(), "{name}: an empty fixture proves nothing");
            let result = parse(language, &src);
            assert!(result.complete, "{name}: the parse did not finish");
            assert_eq!(result.root, language.name(language.root), "{name}: the root's kind changed");
            assert_eq!(result.errors, 0, "{name}: {} error nodes in a valid file:\n{}", result.errors, tree(&path, language));
            assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
        }
        assert!(table.len() >= 8, "fixtures/{folder}: only {} valid files", table.len());
    }

    /// A malformed file: the root survives, spans the file, keeps its kind, and
    /// carries the error inside it.
    fn check_malformed(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
        let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
        assert_eq!(files_in(&format!("{folder}/malformed")), listed, "fixtures/{folder}/malformed: the expectation table and the directory disagree");
        for (name, top) in table {
            let path = fixtures().join(folder).join("malformed").join(name);
            let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            let result = parse(language, &src);
            assert!(result.complete, "{name}: a malformed file must still produce a tree");
            assert_eq!(result.root, language.name(language.root), "{name}: the root did not survive");
            assert!(result.nested_errors > 0, "{name}: no error inside the root:\n{}", tree(&path, language));
            assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
        }
        assert!(table.len() >= 3, "fixtures/{folder}/malformed: only {} malformed files", table.len());
    }

    /// The tree as an S-expression, for the failure messages.
    fn tree(path: &Path, language: &'static Language) -> String {
        let src = fs::read(path).unwrap();
        let mut workspace = Workspace::default();
        workspace.parse(&src, language, &Budget::default()).unwrap();
        workspace.tree.sexp(language, true)
    }

    /// A debugging aid: the tree and the top-level kinds of every fixture, for
    /// retuning a grammar. `cargo test -p deep-grammars -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dump_trees() {
        dump("ruby", &RUBY);
        dump("php", &PHP);
        dump("swift", &SWIFT);
        dump("scala", &SCALA);
    }

    fn dump(folder: &str, language: &'static Language) {
        let mut paths: Vec<PathBuf> = Vec::new();
        for inner in [folder.to_string(), format!("{folder}/malformed")] {
            for entry in fs::read_dir(fixtures().join(inner)).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    paths.push(path);
                }
            }
        }
        paths.sort();
        for path in paths {
            let src = fs::read(&path).unwrap();
            let mut workspace = Workspace::default();
            let outcome = workspace.parse(&src, language, &Budget::default());
            let nodes = &workspace.tree.nodes;
            let errors = (0..nodes.len()).filter(|&i| nodes[i].kind == ERROR || nodes[i].flags & HAS_ERROR != 0 && nodes[i].flags & TOKEN != 0).count();
            let top: Vec<&str> = workspace.tree.children(0).map(|c| &nodes[c as usize]).filter(|n| n.flags & TOKEN == 0).map(|n| language.name(n.kind)).collect();
            println!("=== {:?} complete={:?} errors={errors} top={top:?}", path.file_name().unwrap(), outcome.map(|o| o.complete()));
            if errors > 0 {
                println!("{}", workspace.tree.sexp(language, true));
            }
        }
    }

    #[test]
    fn ruby_fixtures_parse_cleanly() {
        check_valid("ruby", &RUBY, RUBY_VALID);
    }

    #[test]
    fn ruby_malformed_fixtures_keep_the_root() {
        check_malformed("ruby", &RUBY, RUBY_MALFORMED);
    }

    #[test]
    fn ruby_names_its_root() {
        assert_eq!((RUBY.name, RUBY.name(RUBY.root)), ("ruby", "program"));
        assert!(RUBY.layout.is_none());
        assert!(RUBY.kinds.len() > 60, "a grammar this size has dozens of kinds, not {}", RUBY.kinds.len());
    }

    #[test]
    fn php_fixtures_parse_cleanly() {
        check_valid("php", &PHP, PHP_VALID);
    }

    #[test]
    fn php_malformed_fixtures_keep_the_root() {
        check_malformed("php", &PHP, PHP_MALFORMED);
    }

    #[test]
    fn php_names_its_root() {
        assert_eq!((PHP.name, PHP.name(PHP.root)), ("php", "program"));
        assert!(PHP.layout.is_none());
        assert!(PHP.kinds.len() > 100, "a grammar this size has hundreds of kinds, not {}", PHP.kinds.len());
        assert!(PHP.kinds.iter().any(|kind| kind.name == "<?php"), "the opening tag is a token, trivia though it is");
    }

    #[test]
    fn swift_fixtures_parse_cleanly() {
        check_valid("swift", &SWIFT, SWIFT_VALID);
    }

    #[test]
    fn swift_malformed_fixtures_keep_the_root() {
        check_malformed("swift", &SWIFT, SWIFT_MALFORMED);
    }

    #[test]
    fn swift_names_its_root() {
        assert_eq!((SWIFT.name, SWIFT.name(SWIFT.root)), ("swift", "program"));
        assert!(SWIFT.layout.is_none());
        assert!(SWIFT.kinds.len() > 100, "a grammar this size has hundreds of kinds, not {}", SWIFT.kinds.len());
    }

    #[test]
    fn scala_fixtures_parse_cleanly() {
        check_valid("scala", &SCALA, SCALA_VALID);
    }

    #[test]
    fn scala_malformed_fixtures_keep_the_root() {
        check_malformed("scala", &SCALA, SCALA_MALFORMED);
    }

    #[test]
    fn scala_names_its_root() {
        assert_eq!((SCALA.name, SCALA.name(SCALA.root)), ("scala", "program"));
        assert!(SCALA.layout.is_none());
        assert!(SCALA.kinds.len() > 100, "a grammar this size has hundreds of kinds, not {}", SCALA.kinds.len());
    }
}
