//! What a measurement needs from a parser, and the extension table that decides
//! which parser to call.
//!
//! The language modules are written by other agents and may not exist in this
//! worktree. This file is the seam: the 42 languages of `src/langs.rs` are named
//! with their extensions, and each one carries an *optional* parse function.
//!
//! ```text
//! extension -> LanguageSpec { name, extensions, parser: Option<ParseFn> }
//! ```
//!
//! A file whose extension names a language with no parser is recorded as
//! `not_measured`. It is never folded into `clean`: absence of evidence is not
//! a pass. The integrator fills the registry by calling [`Registry::set_parser`]
//! (the `deep-grammars` feature does it for the three grammars in this
//! worktree; see `grammars.rs`).

/// Everything a scan needs from one parse, and nothing a parser has to agree
/// with the report format about.
///
/// A parser that produced a tree returns `fatal: None`, even when the tree has
/// error nodes: a recovered parse is a measured outcome, not a crash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseFacts {
    /// Nodes in the tree, root included.
    pub nodes: u64,
    /// Greatest nesting depth, the root at 0.
    pub max_depth: u32,
    /// Nodes whose kind is the grammar's ERROR kind.
    pub error_nodes: u64,
    /// Source bytes covered by top-level error nodes (nested errors counted once).
    pub error_bytes: u64,
    /// `Some` when no tree was produced at all.
    pub fatal: Option<&'static str>,
}

impl ParseFacts {
    /// A parse that produced a tree with no error node.
    pub fn clean(nodes: u64, max_depth: u32) -> Self {
        ParseFacts { nodes, max_depth, error_nodes: 0, error_bytes: 0, fatal: None }
    }

    /// A parse that recovered: a tree exists, but some source is in error.
    pub fn broken(nodes: u64, max_depth: u32, error_nodes: u64, error_bytes: u64) -> Self {
        ParseFacts { nodes, max_depth, error_nodes, error_bytes, fatal: None }
    }

    /// No tree. `reason` is printed in the report and is the whole story of why
    /// this file has no numbers.
    pub fn fatal(reason: &'static str) -> Self {
        ParseFacts { nodes: 0, max_depth: 0, error_nodes: 0, error_bytes: 0, fatal: Some(reason) }
    }
}

/// A parser under measurement: source bytes in, facts out. It must not panic;
/// a panic is caught by the scanner and recorded as a fatal outcome, because
/// one bad file may not end the run.
pub type ParseFn = fn(&[u8]) -> ParseFacts;

/// One language and, if one has landed, the parser that measures it.
#[derive(Clone, Copy, Debug)]
pub struct LanguageSpec {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub parser: Option<ParseFn>,
}

impl LanguageSpec {
    /// True when a parser is registered, i.e. a file in this language can be
    /// measured rather than merely counted.
    pub fn registered(&self) -> bool {
        self.parser.is_some()
    }

    pub fn extensions_display(&self) -> String {
        if self.extensions.is_empty() {
            "(content only)".to_string()
        } else {
            self.extensions.join(",")
        }
    }
}

/// The extension -> language -> parser table.
///
/// One grammar can need more than one parser: `.ts` and `.tsx` are the same
/// language row but a different lexer. `overrides` carries those exceptions so
/// a language stays one row in every report while each extension is measured
/// by the parser that is actually right for it.
#[derive(Clone, Debug)]
pub struct Registry {
    languages: Vec<LanguageSpec>,
    overrides: Vec<(String, String, ParseFn)>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}

impl Registry {
    /// The 42 languages of `src/langs.rs`, none registered yet.
    ///
    /// Shared suffixes are deliberately not guessed: `.h` can be C, C++ or
    /// Objective-C and `.m` can be Objective-C or MATLAB, so those files stay
    /// outside the scan until the engine's content resolver is wired in. A
    /// guessed language would be a wrong measurement, which is worse than none.
    pub fn new() -> Self {
        let mut languages = Vec::with_capacity(42);
        let mut add = |name: &'static str, extensions: &'static [&'static str]| {
            languages.push(LanguageSpec { name, extensions, parser: None });
        };
        add("Python", &["py", "pyi"]);
        add("JavaScript", &["js", "mjs", "cjs", "jsx"]);
        add("TypeScript", &["ts", "tsx", "mts", "cts"]);
        add("C", &["c"]);
        add("C++", &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp"]);
        add("Objective-C", &["mm"]);
        add("Perl", &["pm"]);
        add("Go", &["go"]);
        add("Rust", &["rs"]);
        add("PHP", &["php", "phtml"]);
        add("Java", &["java"]);
        add("Kotlin", &["kt", "kts"]);
        add("C#", &["cs"]);
        add("Ruby", &["rb", "rake", "gemspec"]);
        add("Swift", &["swift"]);
        add("Scala", &["scala", "sc"]);
        add("Dart", &["dart"]);
        add("Elixir", &["ex", "exs"]);
        add("Haskell", &["hs", "lhs"]);
        add("Lua", &["lua"]);
        add("R", &["r", "rmd"]);
        add("Solidity", &["sol"]);
        add("SQL", &["sql", "ddl"]);
        add("Shell", &["sh", "bash", "zsh", "ksh"]);
        add("PowerShell", &["ps1", "psm1"]);
        add("Groovy", &["groovy", "gradle"]);
        add("Julia", &["jl"]);
        add("Visual Basic", &["vb", "vbs", "bas", "cls"]);
        add("Pascal", &["pas", "dpr", "pp"]);
        add("Fortran", &["f90", "f95", "f03", "f08", "for", "f"]);
        add("COBOL", &["cob", "cbl", "cpy"]);
        add("Nim", &["nim"]);
        add("Crystal", &["cr"]);
        add("Zig", &["zig"]);
        add("D", &["d"]);
        add("F#", &["fs", "fsx"]);
        add("OCaml", &["ml", "mli"]);
        add("Erlang", &["erl", "hrl"]);
        add("Clojure", &["clj", "cljs", "cljc"]);
        add("Elm", &["elm"]);
        add("Tcl", &["tcl"]);
        add("AWK", &["awk"]);
        Registry { languages, overrides: Vec::new() }
    }

    /// Attach a parser to one extension of a language. Returns false unless the
    /// language exists and the extension really belongs to it, so a typo cannot
    /// create a parser that no file will ever reach.
    pub fn set_extension_parser(&mut self, language: &str, extension: &str, parser: ParseFn) -> bool {
        let owned = self.language_for_extension(extension).map(|spec| spec.name) == Some(language);
        if !owned {
            return false;
        }
        match self.overrides.iter_mut().find(|(name, ext, _)| name == language && ext == extension) {
            Some(slot) => slot.2 = parser,
            None => self.overrides.push((language.to_string(), extension.to_string(), parser)),
        }
        true
    }

    /// The parser that measures `extension` of `language`: the extension
    /// override when there is one, otherwise the language's parser.
    pub fn parser_for(&self, language: &str, extension: &str) -> Option<ParseFn> {
        if let Some((_, _, parser)) =
            self.overrides.iter().find(|(name, ext, _)| name == language && ext == extension)
        {
            return Some(*parser);
        }
        self.language(language).and_then(|spec| spec.parser)
    }

    /// Attach a parser to a language by its exact name. Returns false when the
    /// name is not in the table, so a typo cannot silently register nothing.
    pub fn set_parser(&mut self, language: &str, parser: ParseFn) -> bool {
        match self.languages.iter_mut().find(|spec| spec.name == language) {
            Some(spec) => {
                spec.parser = Some(parser);
                true
            }
            None => false,
        }
    }

    /// The language for a lower-case extension, if the project knows it.
    pub fn language_for_extension(&self, extension: &str) -> Option<&LanguageSpec> {
        self.languages.iter().find(|spec| spec.extensions.iter().any(|&ext| ext == extension))
    }

    pub fn language(&self, name: &str) -> Option<&LanguageSpec> {
        self.languages.iter().find(|spec| spec.name == name)
    }

    pub fn languages(&self) -> &[LanguageSpec] {
        &self.languages
    }

    pub fn registered_count(&self) -> usize {
        self.languages.iter().filter(|spec| spec.registered()).count()
    }
}

/// The registry the binary uses: every language the project names, with the
/// parsers that exist in this build.
pub fn builtin() -> Registry {
    #[allow(unused_mut)]
    let mut registry = Registry::new();
    #[cfg(feature = "deep-grammars")]
    crate::grammars::register(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_the_projects_42_languages() {
        let registry = Registry::new();
        assert_eq!(registry.languages().len(), 42);
        assert_eq!(registry.registered_count(), 0);
        assert_eq!(registry.language_for_extension("py").unwrap().name, "Python");
        assert!(registry.language_for_extension("PY").is_none(), "extension lookup is lower-case");
        assert!(registry.language_for_extension("h").is_none(), "shared suffixes are not guessed");
        assert!(registry.language_for_extension("m").is_none(), ".m is content-resolved, not guessed");
        assert!(registry.language("Rust").is_some());
        assert!(registry.language("rust").is_none());
    }

    #[test]
    fn registration_targets_a_real_name_or_nothing() {
        fn parser(_: &[u8]) -> ParseFacts {
            ParseFacts::clean(1, 0)
        }
        let mut registry = Registry::new();
        assert!(registry.set_parser("Python", parser));
        assert!(!registry.set_parser("python", parser));
        assert!(registry.language("Python").unwrap().registered());
        assert_eq!(registry.registered_count(), 1);
    }
}
