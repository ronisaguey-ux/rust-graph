//! C, C++ and Objective-C through `grammar!`: the third, hardest batch.
//!
//! The three grammars share a skeleton (declaration specifiers, declarators,
//! statements, one precedence table) and the step-lexer helpers below, but each
//! is its own `grammar!` invocation: the DSL has no way to say "this grammar
//! plus these rules", and C++ and Objective-C differ in enough tokens that
//! parameterising one definition would cost more than it saved.
//!
//! What the three of them have in common, and how the hard parts are handled:
//!
//! - **Preprocessor lines are trivia.** `#define`, `#if`, `#include` and the
//!   rest are lexed as one `preprocessor` token per logical line (backslash
//!   continuations included), declared `trivia`, so the parser never sees them
//!   and a file full of macros parses exactly like the same file with the
//!   directives blanked out. The macro *uses* are ordinary expressions.
//! - **`>` is never part of `>>`.** The lexer splits a `>` that is followed by
//!   a `>` into `>joined` and lets the next step produce the second `>`; both
//!   template argument lists and the shift operator accept the pair. So
//!   `std::vector<std::vector<int>>` closes two lists and `a >> b` shifts,
//!   with no lexer state.
//! - **`x * y;` is an expression.** Declarations must begin with a declaration
//!   specifier keyword (or, in C++, a user type name), so a statement that
//!   starts with an identifier is always an expression statement: the classic
//!   C ambiguity is resolved statically, in favour of multiplication. There is
//!   no shift/reduce conflict to report because the runtime is LL: the choice
//!   is made on the first token (for C++) or the first two, and the decision is
//!   a plain lookahead table entry, not a conflict.
//! - **Braces are plain tokens.** A `{` has to be testable by a predicate
//!   (`&(declaration_specifiers _declarator "{")`) to tell a function
//!   definition from a prototype of the same signature without parsing the
//!   body twice, and the DSL only lets a delimiter open and close a whole rule.
//!   Parens and brackets stay real delimiters (they give the firewall where it
//!   pays); a mismatched brace is recovered by the statement loop instead.
//! - **Typedef names are not tracked.** C has no symbol table here, so a
//!   declaration whose type is a bare typedef name is read as an expression
//!   statement. C++ accepts a leading user type (`Foo x;`) and a scoped one
//!   (`std::string x;`), but not a keyword-qualified bare user type
//!   (`const Foo x;`): telling it from `const int x;` needs a symbol table.
//! - **Objective-C messages are postfix.** `[receiver selector]` is one
//!   postfix row, so `a[0]` and `[arr objectAtIndex:0]` both parse; so does a
//!   nested `[[a b] c]`.

pub mod c;
pub mod cpp;
pub mod objc;

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, WHITESPACE};

/// Whitespace and comments, the trivia every C-family lexer shares.
pub(crate) fn trivia(c: &mut Cursor) -> Option<Kind> {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
            Some(WHITESPACE)
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n');
            Some(COMMENT)
        }
        b'/' if c.peek_at(1) == b'*' => {
            c.bump_n(2);
            while !c.at_end() && !c.starts_with(b"*/") {
                c.bump();
            }
            c.bump_n(2);
            Some(COMMENT)
        }
        0xc2 if c.peek_at(1) == 0xa0 => {
            c.bump_n(2);
            Some(WHITESPACE)
        }
        _ => None,
    }
}

/// Whether only spaces and tabs stand between the cursor and its line start.
pub(crate) fn at_line_start(c: &Cursor) -> bool {
    let mut back = 1;
    loop {
        let byte = c.behind(back);
        match byte {
            b'\n' | 0 => return true,
            b' ' | b'\t' => back += 1,
            b'\r' => return true,
            _ => return false,
        }
    }
}

/// A preprocessor directive: from its `#` through the end of its logical line,
/// joining backslash-newline continuations. True if one was consumed.
pub(crate) fn directive(c: &mut Cursor) -> bool {
    if c.peek() != b'#' || !at_line_start(c) {
        return false;
    }
    loop {
        match c.peek() {
            0 if c.at_end() => return true,
            b'\n' | b'\r' => return true,
            b'\\' if matches!(c.peek_at(1), b'\n' | b'\r') => {
                c.bump();
                c.eat(b'\r');
                c.eat(b'\n');
            }
            // A `/* ... */` inside a directive stays inside it: a newline in a
            // block comment does not end the directive.
            b'/' if c.peek_at(1) == b'*' => {
                c.bump_n(2);
                while !c.at_end() && !c.starts_with(b"*/") {
                    c.bump();
                }
                c.bump_n(2);
            }
            _ => c.bump(),
        }
    }
}

/// The body of a quoted literal after its opening quote. True if it closed.
pub(crate) fn quoted(c: &mut Cursor, quote: u8) -> bool {
    c.quoted(quote, true)
}

/// The letters a word may carry before a literal, in the C family: `L`, `u`,
/// `U`, `u8`. Returns whether they make a string (`true`) or a character
/// literal (`false`), or `None` when the word is not a prefix at all.
pub(crate) fn literal_prefix(word: &[u8]) -> Option<bool> {
    match word {
        b"L" | b"U" => Some(true),
        b"u" => Some(true),
        b"u8" => Some(true),
        _ => None,
    }
}

/// A number: decimal, hex, octal and binary integers, floats with exponents
/// and hex floats, and the `u`/`l`/`f`/`z`/`i` suffixes. With `separators`, a
/// `'` between digits is part of the number (C++14), not a stray quote.
pub(crate) fn number(c: &mut Cursor, separators: bool) {
    let sep = |byte: u8| separators && byte == b'\'';
    let digits = |byte: u8| byte.is_ascii_digit() || sep(byte);
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_hexdigit() || sep(byte) || byte == b'.');
        if matches!(c.peek(), b'p' | b'P') {
            c.bump();
            c.eat(b'+');
            c.eat(b'-');
            c.eat_while(digits);
        }
    } else if c.peek() == b'0' && matches!(c.peek_at(1), b'b' | b'B') {
        c.bump_n(2);
        c.eat_while(|byte| matches!(byte, b'0' | b'1') || sep(byte));
    } else {
        c.eat_while(digits);
        if c.peek() == b'.' {
            c.bump();
            c.eat_while(digits);
        }
        if matches!(c.peek(), b'e' | b'E') && (c.peek_at(1).is_ascii_digit() || (matches!(c.peek_at(1), b'+' | b'-') && c.peek_at(2).is_ascii_digit())) {
            c.bump_n(2);
            c.eat_while(digits);
        }
    }
    c.eat_while(|byte| matches!(byte, b'u' | b'U' | b'l' | b'L' | b'f' | b'F' | b'z' | b'Z' | b'i' | b'I'));
}

#[inline]
pub(crate) fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
pub(crate) fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::deep::syntax::{Language, ERROR};
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    /// `(file, top-level node kinds)` for every valid fixture, in order.
    const C_VALID: &[(&str, &str)] = &[
        ("control.c", "function_definition function_definition function_definition function_definition function_definition"),
        ("expressions.c", "function_definition function_definition"),
        ("functions.c", "declaration_statement function_definition function_definition function_definition function_definition"),
        ("hello.c", "declaration_statement function_definition function_definition"),
        ("initializers.c", "declaration_statement declaration_statement static_assert_statement declaration_statement declaration_statement declaration_statement function_definition"),
        ("macros.c", "declaration_statement declaration_statement declaration_statement function_definition function_definition"),
        ("pointers.c", "declaration_statement declaration_statement declaration_statement declaration_statement function_definition function_definition function_definition function_definition"),
        ("structs.c", "declaration_statement declaration_statement declaration_statement declaration_statement declaration_statement function_definition function_definition function_definition"),
    ];

    const CPP_VALID: &[(&str, &str)] = &[
        ("classes.cpp", "namespace_definition function_definition"),
        ("containers.cpp", "namespace_definition function_definition function_definition"),
        ("inheritance.cpp", "namespace_definition"),
        ("objects.cpp", "declaration_statement function_definition function_definition function_definition"),
        ("preprocessor.cpp", "namespace_definition function_definition"),
        ("statements.cpp", "namespace_definition"),
        ("strings.cpp", "function_definition function_definition function_definition function_definition function_definition"),
        ("templates.cpp", "template_declaration template_declaration function_definition function_definition function_definition"),
    ];

    const OBJC_VALID: &[(&str, &str)] = &[
        ("collections.m", "function_definition"),
        ("exceptions.m", "function_definition function_definition function_definition objc_interface objc_implementation"),
        ("expressions.m", "declaration_statement declaration_statement function_definition function_definition function_definition function_definition"),
        ("functions.m", "declaration_statement function_definition function_definition function_definition function_definition function_definition"),
        ("hello.m", "objc_implementation function_definition"),
        ("loops.m", "function_definition function_definition function_definition function_definition function_definition"),
        ("properties.m", "objc_protocol objc_interface objc_implementation"),
        ("protocols.m", "objc_class_declaration objc_protocol objc_interface objc_implementation"),
        ("strings.m", "declaration_statement user_function function_definition user_function"),
    ];

    /// What one parse produced, as the tests compare it.
    struct Parsed {
        complete: bool,
        root: &'static str,
        errors: usize,
        top: Vec<&'static str>,
        nested: usize,
    }

    fn parse(language: &'static Language, src: &[u8]) -> Parsed {
        let mut workspace = Workspace::default();
        let outcome = workspace.parse(src, language, &Budget::default()).unwrap_or_else(|_| panic!("{}: too large", language.name));
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
            nested: nodes.iter().filter(|node| node.kind == ERROR && node.flags & TOKEN == 0).count(),
        }
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("{dir:?}: {error}"))
            .filter(|entry| entry.as_ref().unwrap().path().is_file())
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

    /// Every valid fixture: the directory and the table agree, the parse
    /// finishes, the root is the language's, there are no errors, and the
    /// top-level kinds are exactly what the table says.
    fn valid(folder: &str, language: &'static Language, table: &[(&str, &str)]) {
        let on_disk = names_in(&fixtures().join(folder));
        let listed: Vec<String> = table.iter().map(|(name, _)| (*name).to_string()).collect();
        assert_eq!(on_disk, listed, "fixtures/{folder}: the table and the directory disagree");
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

    /// Every malformed fixture: the root survives and carries the error inside.
    fn malformed(folder: &str, language: &'static Language) {
        let names = names_in(&fixtures().join(folder).join("broken"));
        assert!(names.len() >= 3, "fixtures/{folder}/broken: only {} files", names.len());
        for name in &names {
            let path = fixtures().join(folder).join("broken").join(name);
            let src = fs::read(&path).unwrap();
            let result = parse(language, &src);
            assert!(result.complete, "{name}: a malformed file must still produce a tree");
            assert_eq!(result.root, language.name(language.root), "{name}: the root did not survive");
            assert!(result.nested > 0, "{name}: no ERROR node inside the root:\n{}", tree(&path, language));
        }
    }

    #[test]
    fn c_fixtures_parse_cleanly() {
        valid("c", &super::c::C, C_VALID);
    }

    #[test]
    fn cpp_fixtures_parse_cleanly() {
        valid("cpp", &super::cpp::CPP, CPP_VALID);
    }

    #[test]
    fn objc_fixtures_parse_cleanly() {
        valid("objc", &super::objc::OBJC, OBJC_VALID);
    }

    #[test]
    fn malformed_fixtures_keep_the_root_and_report_inside_it() {
        malformed("c", &super::c::C);
        malformed("cpp", &super::cpp::CPP);
        malformed("objc", &super::objc::OBJC);
    }

    #[test]
    fn the_three_languages_name_their_roots() {
        assert_eq!((super::c::C.name, super::c::C.name(super::c::C.root)), ("c", "translation_unit"));
        assert_eq!((super::cpp::CPP.name, super::cpp::CPP.name(super::cpp::CPP.root)), ("cpp", "translation_unit"));
        assert_eq!((super::objc::OBJC.name, super::objc::OBJC.name(super::objc::OBJC.root)), ("objc", "translation_unit"));
    }

    /// The significant kinds of the first file's first tokens, trivia dropped.
    fn significant(language: &'static Language, src: &str) -> Vec<&'static str> {
        let mut workspace = Workspace::default();
        workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
        (0..workspace.tokens.len())
            .filter(|&at| !language.is_trivia(workspace.tokens.kind[at]))
            .map(|at| language.name(workspace.tokens.kind[at]))
            .collect()
    }

    fn directives(language: &'static Language, src: &str) -> usize {
        let mut workspace = Workspace::default();
        workspace.parse(src.as_bytes(), language, &Budget::default()).unwrap();
        (0..workspace.tokens.len()).filter(|&at| language.name(workspace.tokens.kind[at]) == "preprocessor").count()
    }

    /// Hard problem 1: a file full of macros parses, and every directive is a
    /// trivia token the grammar never sees, one per logical line.
    #[test]
    fn preprocessor_lines_are_trivia_and_the_file_still_parses() {
        let path = fixtures().join("c/macros.c");
        let src = fs::read(&path).unwrap();
        assert!(directives(&super::c::C, std::str::from_utf8(&src).unwrap()) >= 8);
        assert_eq!(parse(&super::c::C, &src).errors, 0, "the macro fixture has errors");
        // A directive does not swallow the next line, and a continuation is one token.
        let src = "#define A 1\n#define B(x) \\\n  (x) + A\nint main(void) { return B(2); }\n";
        assert_eq!(directives(&super::c::C, src), 2);
        assert_eq!(parse(&super::c::C, src.as_bytes()).errors, 0);
        assert_eq!(
            significant(&super::c::C, "#define X 1\nint x = X;\n"),
            ["int", "identifier", "=", "identifier", ";"]
        );
        assert_eq!(directives(&super::cpp::CPP, "#if 1\n#endif\nint x;\n"), 2);
    }

    /// Hard problem 2: a `>` directly before another is emitted as two tokens,
    /// so nested template lists close and a shift is the same pair.
    #[test]
    fn a_double_angle_is_two_tokens() {
        assert_eq!(
            significant(&super::cpp::CPP, "std::vector<std::vector<int>> grid;\n"),
            ["identifier", "::", "identifier", "<", "identifier", "::", "identifier", "<", "int", ">joined", ">", "identifier", ";"]
        );
        assert_eq!(
            significant(&super::cpp::CPP, "int f(int a, int b) { return a >> b; }\n").iter().filter(|kind| **kind == ">joined").count(),
            1
        );
        // Both readings parse: the declaration needs two closers, the shift one.
        for src in ["std::vector<std::vector<int>> grid;\n", "int f(int a, int b) { return a >> b; }\n"] {
            assert_eq!(parse(&super::cpp::CPP, src.as_bytes()).errors, 0, "{src}");
        }
        // Three levels close with one joined pair and one plain `>`.
        let kinds = significant(&super::cpp::CPP, "std::vector<std::vector<std::vector<int>>> deep;\n");
        assert_eq!(kinds.iter().filter(|kind| **kind == ">joined").count(), 2);
        assert_eq!(kinds.iter().filter(|kind| **kind == ">").count(), 1);
        assert_eq!(parse(&super::cpp::CPP, "std::vector<std::vector<std::vector<int>>> deep;\n".as_bytes()).errors, 0);
    }

    /// Hard problem 3: `x * y;` is an expression statement, never a
    /// declaration; the choice is one lookahead token, not a shift/reduce pair.
    #[test]
    fn a_pointer_like_statement_is_multiplication() {
        let src = b"void f(void) {\n    x * y;\n}\n";
        let mut workspace = Workspace::default();
        workspace.parse(src, &super::c::C, &Budget::default()).unwrap();
        let sexp = workspace.tree.sexp(&super::c::C, true);
        assert!(sexp.contains("(expression_statement (binary_expression"), "{sexp}");
        assert_eq!(parse(&super::c::C, src).errors, 0);
        // `int *p;` is still a declaration: the first token decides.
        let src = b"void f(void) {\n    int *p;\n}\n";
        let mut workspace = Workspace::default();
        workspace.parse(src, &super::c::C, &Budget::default()).unwrap();
        assert!(workspace.tree.sexp(&super::c::C, true).contains("(declaration_statement"));
        assert_eq!(parse(&super::c::C, src).errors, 0);
    }
}
