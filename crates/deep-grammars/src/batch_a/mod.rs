//! Batch A: Go, Java, C# and Kotlin, each a `grammar!` definition plus the
//! hand-written step lexer it needs.
//!
//! The four modules follow the same shape as `python.rs` and
//! `javascript.rs`: a token list, one grammar, one lexer and one test module.
//! Every fixture (8 valid and 3 malformed per language) lives under
//! `fixtures/<language>/`, with the malformed ones in `malformed/`.
//!
//! The fixtures are checked from [`fixtures`], which is generic over the
//! language: a valid file must parse completely with no error node and with
//! exactly the listed top-level node kinds; a malformed file must keep its
//! root, span the input, and carry the error *inside* the root (the cascade
//! rule).

pub mod csharp;
pub mod go;
pub mod java;
pub mod kotlin;

/// Shared fixture checking, used by each language's test module.
///
/// The expectation tables are passed in, so a fixture added to a directory
/// without an entry fails (the directory and the table must agree).
#[cfg(test)]
pub(crate) mod fixtures {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::deep::syntax::{Language, ERROR};
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    struct Parsed {
        complete: bool,
        root: &'static str,
        /// `ERROR` nodes and bad-token leaves.
        errors: usize,
        /// The root's non-token children, in order.
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

    pub(crate) fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("{dir:?}: {error}"))
            .filter(|entry| entry.as_ref().map(|entry| entry.file_type().map(|kind| kind.is_file()).unwrap_or(false)).unwrap_or(false))
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn tree(path: &Path, language: &'static Language) -> String {
        let src = fs::read(path).unwrap();
        let mut workspace = Workspace::default();
        workspace.parse(&src, language, &Budget::default()).unwrap();
        workspace.tree.sexp(language, true)
    }

    /// Every valid fixture in `fixtures/<folder>/` against `table`:
    /// complete parse, the right root, zero errors, exact top-level kinds.
    pub(crate) fn valid(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
        let dir = fixtures().join(folder);
        let on_disk = files_in(&dir);
        let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
        assert_eq!(on_disk, listed, "fixtures/{folder}: the expectation table and the directory disagree");
        assert_eq!(table.len(), 8, "fixtures/{folder}: the batch promises eight valid files");
        for (name, top) in table {
            let path = dir.join(name);
            let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            assert!(!src.is_empty(), "{name}: an empty fixture proves nothing");
            let result = parse(language, &src);
            assert!(result.complete, "{name}: the parse did not finish");
            assert_eq!(result.root, language.name(language.root), "{name}: the root's kind changed");
            assert_eq!(result.errors, 0, "{name}: {} error nodes in a valid file:\n{}", result.errors, tree(&path, language));
            assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
        }
    }

    /// Every malformed fixture in `fixtures/<folder>/malformed/`: the root
    /// survives, spans the file, and the error is inside it.
    pub(crate) fn malformed(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
        let dir = fixtures().join(folder).join("malformed");
        let on_disk = files_in(&dir);
        let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
        assert_eq!(on_disk, listed, "fixtures/{folder}/malformed: the expectation table and the directory disagree");
        assert_eq!(table.len(), 3, "fixtures/{folder}: the batch promises three malformed files");
        for (name, top) in table {
            let path = dir.join(name);
            let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            let result = parse(language, &src);
            assert!(result.complete, "{name}: a malformed file must still produce a tree");
            assert_eq!(result.root, language.name(language.root), "{name}: the root did not survive");
            assert!(result.nested_errors > 0, "{name}: no error inside the root:\n{}", tree(&path, language));
            assert_eq!(result.top, top.split_whitespace().collect::<Vec<_>>(), "{name}: top-level kinds");
        }
    }

    /// The significant token kinds of a source, for a lexer test.
    pub(crate) fn tokens(language: &'static Language, src: &str) -> Vec<&'static str> {
        let mut workspace = Workspace::default();
        workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
        (0..workspace.tokens.len())
            .filter(|&at| !language.is_trivia(workspace.tokens.kind[at]))
            .map(|at| language.name(workspace.tokens.kind[at]))
            .collect()
    }

    /// Whether a source parses with no error node.
    pub(crate) fn clean(language: &'static Language, src: &str) -> bool {
        let mut workspace = Workspace::default();
        let outcome = workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
        assert!(outcome.complete(), "{}: {}", language.name, outcome.describe());
        workspace.tree.root().flags & HAS_ERROR == 0
    }
}
