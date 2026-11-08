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

pub(crate) mod c;
pub(crate) mod cpp;
pub(crate) mod objc;

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
            nested: nodes.iter().filter(|node| node.kind == ERROR && node.flags & 0 == 0).count(),
        }
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    /// The file names directly inside a fixture directory, sorted.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("{dir:?}: {error}"))
            .filter(|entry| entry.as_ref().unwrap().path().is_file())
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn valid(folder: &str, language: &'static Language) {
        let mut names = names_in(&fixtures().join(folder));
        names.sort();
        let mut count = 0;
        for name in &names {
            let path = fixtures().join(folder).join(name);
            let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            assert!(!src.is_empty(), "{name}: empty fixture");
            let result = parse(language, &src);
            assert!(result.complete, "{name}: the parse did not finish");
            assert_eq!(result.root, language.name(language.root), "{name}: root");
            assert_eq!(result.errors, 0, "{name}: {} error nodes:\n{}", result.errors, tree(&path, language));
            count += 1;
        }
        assert!(count >= 8, "fixtures/{folder}: only {count} files");
    }

    fn malformed(folder: &str, language: &'static Language) {
        let names = names_in(&fixtures().join(folder).join("broken"));
        assert!(names.len() >= 3, "fixtures/{folder}/broken: only {} files", names.len());
        for name in &names {
            let path = fixtures().join(folder).join("broken").join(name);
            let src = fs::read(&path).unwrap();
            let result = parse(language, &src);
            assert!(result.complete, "{name}: a malformed file must still produce a tree");
            assert_eq!(result.root, language.name(language.root), "{name}: root did not survive");
            assert!(result.nested > 0, "{name}: no ERROR node inside the root:\n{}", tree(&path, language));
        }
    }

    fn tree(path: &Path, language: &'static Language) -> String {
        let src = fs::read(path).unwrap();
        let mut workspace = Workspace::default();
        workspace.parse(&src, language, &Budget::default()).unwrap();
        workspace.tree.sexp(language, true)
    }

    #[test]
    fn cpp_fixtures_parse_cleanly() {
        valid("cpp", &super::cpp::CPP);
    }

    #[test]
    fn cpp_malformed_keeps_the_root() {
        malformed("cpp", &super::cpp::CPP);
    }

    #[test]
    fn c_fixtures_parse_cleanly() {
        valid("c", &super::c::C);
    }

    #[test]
    fn c_malformed_keeps_the_root() {
        malformed("c", &super::c::C);
    }

    /// Not an assertion: prints what each C++ fixture's root holds.
    #[test]
    fn dump_cpp() {
        dump("cpp", &super::cpp::CPP);
    }

    fn dump(folder: &str, language: &'static crate::deep::syntax::Language) {
        for name in names_in(&fixtures().join(folder)) {
            let src = fs::read(fixtures().join(folder).join(&name)).unwrap();
            let result = parse(language, &src);
            println!("{name}|{}|errors={}|{}", result.complete, result.errors, result.top.join(" "));
        }
        for name in names_in(&fixtures().join(folder).join("broken")) {
            let src = fs::read(fixtures().join(folder).join("broken").join(&name)).unwrap();
            let result = parse(language, &src);
            println!("broken/{name}|{}|errors={}|{}", result.complete, result.errors, result.top.join(" "));
        }
    }

    /// Not an assertion: prints what each C fixture's root holds, so the
    /// expectation tables can be written from a real parse.
    #[test]
    fn dump_c() {
        let names = names_in(&fixtures().join("c"));
        for name in names {
            let src = fs::read(fixtures().join("c").join(&name)).unwrap();
            let result = parse(&super::c::C, &src);
            println!("{name}|{}|errors={}|{}", result.complete, result.errors, result.top.join(" "));
        }
        let names = names_in(&fixtures().join("c/broken"));
        for name in names {
            let src = fs::read(fixtures().join("c/broken").join(&name)).unwrap();
            let result = parse(&super::c::C, &src);
            println!("broken/{name}|{}|errors={}|{}", result.complete, result.errors, result.top.join(" "));
        }
    }
}

#[cfg(test)]
mod debug_trees {
    use std::fs;
    use std::path::Path;

    use crate::deep::{Budget, Workspace};

    #[test]
    fn first_error() {
        let Ok(name) = std::env::var("FIXTURE") else { return };
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(&name);
        let src = fs::read(&path).unwrap();
        let language = if name.ends_with(".c") { "c" } else { "cpp" };
        let language: &'static _ = match language {
            "c" => &crate::batch_c::c::C,
            _ => &crate::batch_c::cpp::CPP,
        };
        let mut workspace = Workspace::default();
        workspace.parse(&src, language, &Budget::default()).unwrap();
        let sexp = workspace.tree.sexp(language, false);
        let at = sexp.find("(ERROR").unwrap_or(0);
        let start = sexp[..at].rfind("(").unwrap_or(0).saturating_sub(300);
        println!("{}", &sexp[start..sexp.len().min(at + 700)]);
    }
}
