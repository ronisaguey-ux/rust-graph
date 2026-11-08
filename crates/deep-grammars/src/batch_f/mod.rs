//! The second long tail: dialects and legacy line-oriented languages.
//!
//! One module per language, each with a `grammar!`, its own step lexer and
//! fixtures under `fixtures/longtail3/<language>/` (plus one malformed file
//! each under `fixtures/longtail3/malformed/`).
//!
//! The module comment of each language names what it models and what it does
//! not; the tests beside it parse those fixtures and assert the top-level kinds,
//! zero error nodes on a valid file, and a surviving root with the error inside
//! on a malformed one.
//!
//! The languages here share a property the C-like tail does not: several are
//! defined by their *line* rather than by punctuation. Visual Basic, PowerShell,
//! COBOL and fixed-form Fortran each need the lexer to know where a line starts,
//! and COBOL and Fortran additionally need it to know which *column* a byte sits
//! in. That is done in the step functions with `Cursor::pos` and a line-start
//! lookup, not with lexer modes the DSL cannot drive.

pub mod cobol;
pub mod dart;
pub mod fortran;
pub mod fsharp;
pub mod groovy;
pub mod pascal;
pub mod powershell;
pub mod visual_basic;

#[cfg(test)]
pub(crate) mod harness {
    //! Shared fixture driver: parse a file, report the root, the error count and
    //! the top-level node kinds.

    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::deep::syntax::{Language, ERROR};
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    pub struct Parsed {
        pub complete: bool,
        pub root: &'static str,
        pub errors: usize,
        pub nested_errors: usize,
        pub top: Vec<String>,
    }

    pub fn parse(language: &'static Language, src: &[u8]) -> Parsed {
        let mut workspace = Workspace::default();
        let outcome = workspace
            .parse(src, language, &Budget::default())
            .unwrap_or_else(|_| panic!("{}: input too large", language.name));
        let nodes = &workspace.tree.nodes;
        let is_error = |index: usize| {
            nodes[index].kind == ERROR
                || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0
        };
        let top = workspace
            .tree
            .children(0)
            .map(|child| &nodes[child as usize])
            .filter(|node| node.flags & TOKEN == 0)
            .map(|node| language.name(node.kind).to_string())
            .collect();
        Parsed {
            complete: outcome.complete(),
            root: language.name(workspace.tree.root().kind),
            errors: (0..nodes.len()).filter(|&index| is_error(index)).count(),
            nested_errors: (1..nodes.len()).filter(|&index| is_error(index)).count(),
            top,
        }
    }

    pub fn sexp(language: &'static Language, src: &[u8]) -> String {
        let mut workspace = Workspace::default();
        workspace.parse(src, language, &Budget::default()).unwrap();
        workspace.tree.sexp(language, true)
    }

    pub fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/longtail3")
    }

    /// Prints every fixture's top-level kinds, so a table is written from what
    /// the grammar produced rather than guessed.
    pub fn dump(folder: &str, language: &'static Language) {
        let mut names: Vec<String> = fs::read_dir(fixtures().join(folder))
            .unwrap_or_else(|error| panic!("fixtures/longtail3/{folder}: {error}"))
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            let src = read(folder, &name);
            let result = parse(language, &src);
            println!(
                "{folder}/{name}: complete={} errors={} top={}",
                result.complete,
                result.errors,
                result.top.join(" ")
            );
            if result.errors > 0 {
                println!("{}", sexp(language, &src));
            }
        }
    }

    pub fn read(folder: &str, name: &str) -> Vec<u8> {
        let path = fixtures().join(folder).join(name);
        fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
    }

    /// Asserts the valid fixtures of one language: each parses completely, with
    /// no error node, the language's root, and exactly the listed top-level
    /// kinds.
    pub fn check_valid(
        folder: &str,
        language: &'static Language,
        table: &[(&str, &[&str])],
    ) {
        assert!(table.len() >= 3, "{folder}: at least three valid fixtures");
        for (name, top) in table {
            let src = read(folder, name);
            assert!(!src.is_empty(), "{name}: an empty fixture proves nothing");
            let result = parse(language, &src);
            assert!(result.complete, "{name}: the parse did not finish");
            assert_eq!(
                result.root,
                language.name(language.root),
                "{name}: the root's kind changed"
            );
            assert_eq!(
                result.errors,
                0,
                "{name}: {} error nodes in a valid file:\n{}",
                result.errors,
                sexp(language, &src)
            );
            assert_eq!(
                result.top,
                top.iter().map(|kind| kind.to_string()).collect::<Vec<_>>(),
                "{name}: top-level kinds:\n{}",
                sexp(language, &src)
            );
        }
    }

    /// Asserts one malformed fixture: the parse finishes, the root survives, at
    /// least one error sits inside it, and the top-level kinds are as listed.
    pub fn check_malformed(
        folder: &str,
        name: &str,
        language: &'static Language,
        top: &[&str],
    ) {
        let src = read(folder, name);
        let result = parse(language, &src);
        assert!(result.complete, "{name}: a malformed file must still produce a tree");
        assert_eq!(result.root, language.name(language.root), "{name}: the root did not survive");
        assert!(
            result.nested_errors > 0,
            "{name}: no error inside the root:\n{}",
            sexp(language, &src)
        );
        assert_eq!(
            result.top,
            top.iter().map(|kind| kind.to_string()).collect::<Vec<_>>(),
            "{name}: top-level kinds:\n{}",
            sexp(language, &src)
        );
    }
}
