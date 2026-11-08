//! The language table. A language is data: its lexical rules, how its blocks
//! close, and the words that introduce a definition. Adding one means adding a
//! row, and no new parser, unless it needs a block style nobody else has.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blocks {
    /// `{ }` with `;` or newlines between statements.
    Brace,
    /// Significant indentation (Python, Nim).
    Indent,
    /// Keyword-opened blocks closed by `end` (Ruby, Lua, Julia, MATLAB, Elixir).
    End,
    /// `procedure X; begin ... end;` (Pascal family).
    BeginEnd,
    /// `Sub X ... End Sub`: only definitions are closed by name (VB, Fortran).
    NamedEnd,
    /// A definition starts in column one and runs to the next (Haskell, SQL, COBOL).
    Column,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interp {
    None,
    /// `f"{x}"`.
    PyFormat,
    /// Backtick strings with `${x}`.
    Template,
    /// `"#{x}"`.
    Hash,
    /// `"$x"` and `"${x}"`.
    Dollar,
    /// `"\(x)"`.
    Swift,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Imports {
    None,
    Python,
    Js,
    /// `#include "x"` and `#import "x"`.
    Include,
    /// `keyword a.b.c`, `a::b`, or a quoted path.
    Path,
}

#[derive(Clone, Copy)]
pub struct Language {
    pub name: &'static str,
    /// Short and stable: findings and rule ids are derived from it.
    pub tag: &'static str,
    pub extensions: &'static [&'static str],
    pub blocks: Blocks,
    pub line_comments: &'static [&'static str],
    pub block_comments: &'static [(&'static str, &'static str)],
    pub quotes: &'static [u8],
    pub triple_quotes: bool,
    pub interp: Interp,
    /// `$x`, `@x`, `%x` name the same variable as `x`.
    pub sigils: bool,
    /// Bytes other than `[A-Za-z0-9_]` that may continue an identifier.
    pub ident_extra: &'static [u8],
    pub regex_literals: bool,
    pub case_insensitive: bool,
    /// A statement can only end at `;`, so a newline never terminates one.
    pub semicolons: bool,
    /// A trailing operator carries the statement onto the next line. Python's
    /// lines only continue inside brackets or after `\`, so `import *` ends.
    pub operator_continues: bool,
    /// Calls may omit parentheses at the start of a statement (`system "ls"`).
    pub command_calls: bool,
    /// Calls are juxtaposition everywhere (`f x y`).
    pub apply_calls: bool,
    /// Top-level names are visible across files without an import.
    pub global_names: bool,
    /// Words that introduce a function or method.
    pub defs: &'static [&'static str],
    /// Words that introduce a class-like container.
    pub classes: &'static [&'static str],
    /// `End`: statement-opening words that need a closer.
    pub openers: &'static [&'static str],
    pub closers: &'static [&'static str],
    /// `End`: words that open a block wherever they appear in a statement.
    pub inline_openers: &'static [&'static str],
    pub imports: Imports,
    pub import_words: &'static [&'static str],
    /// Parameters every function has implicitly (`@_`, `$1`).
    pub implicit_params: &'static [&'static str],
    /// Words that read the next argument (`shift`).
    pub arg_words: &'static [&'static str],
    /// Words whose next name is called (`CALL X`, `PERFORM X`).
    pub call_words: &'static [&'static str],
    /// `<<EOF` bodies are string data.
    pub heredocs: bool,
    /// A parameter is `type name`, not `name type`.
    pub type_first: bool,
    /// Calls are written `(f x y)`.
    pub prefix_calls: bool,
    /// A bare name inside a class means a member of that class first.
    pub member_calls: bool,
    /// `.` before whitespace ends a statement (Erlang forms, COBOL sentences).
    pub period_ends: bool,
    /// A statement runs to that period, however many lines it takes.
    pub period_only: bool,
}

const BASE: Language = Language {
    name: "",
    tag: "",
    extensions: &[],
    blocks: Blocks::Brace,
    line_comments: &["//"],
    block_comments: &[("/*", "*/")],
    quotes: b"\"'",
    triple_quotes: false,
    interp: Interp::None,
    sigils: false,
    ident_extra: b"",
    regex_literals: false,
    case_insensitive: false,
    semicolons: true,
    operator_continues: true,
    command_calls: false,
    apply_calls: false,
    global_names: false,
    defs: &[],
    classes: &["class", "struct", "interface", "enum"],
    openers: &[],
    closers: &["end"],
    inline_openers: &[],
    imports: Imports::None,
    import_words: &[],
    implicit_params: &[],
    arg_words: &[],
    call_words: &[],
    heredocs: false,
    type_first: false,
    prefix_calls: false,
    member_calls: false,
    period_ends: false,
    period_only: false,
};

const C_LIKE: Language = Language { type_first: true, global_names: true, imports: Imports::Include, classes: &["struct", "class", "union", "namespace"], ..BASE };
const JS_LIKE: Language = Language {
    quotes: b"\"'`",
    interp: Interp::Template,
    regex_literals: true,
    semicolons: false,
    defs: &["function"],
    classes: &["class", "interface"],
    imports: Imports::Js,
    ..BASE
};
const HASH_COMMENT: &[&str] = &["#"];

pub const C: Language = Language { name: "C", tag: "C", extensions: &["c"], ..C_LIKE };
pub const CPP: Language = Language {
    name: "C++",
    member_calls: true,
    tag: "CPP",
    extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp"],
    ..C_LIKE
};
pub const OBJC: Language = Language {
    name: "Objective-C",
    tag: "OBJC",
    extensions: &["mm"],
    classes: &["struct", "class"],
    ..C_LIKE
};
pub const MATLAB: Language = Language {
    name: "MATLAB",
    tag: "M",
    blocks: Blocks::End,
    line_comments: &["%"],
    block_comments: &[("%{", "%}")],
    quotes: b"\"",
    semicolons: false,
    defs: &["function"],
    classes: &["classdef"],
    openers: &["if", "for", "while", "switch", "try", "parfor"],
    ..BASE
};
pub const PERL: Language = Language {
    name: "Perl",
    tag: "PL",
    extensions: &["pm"],
    line_comments: HASH_COMMENT,
    block_comments: &[],
    interp: Interp::Dollar,
    sigils: true,
    regex_literals: true,
    command_calls: true,
    defs: &["sub"],
    classes: &["package"],
    imports: Imports::Path,
    import_words: &["use", "require"],
    implicit_params: &["_"],
    arg_words: &["shift"],
    heredocs: true,
    ..BASE
};

/// Rows with a suffix of their own. `.h`, `.m`, `.pl` and `.ts` go through
/// `by_content` first, because the suffix does not say what they hold.
pub const LANGUAGES: &[Language] = &[
    Language {
        name: "Python",
        tag: "PY",
        extensions: &["py", "pyi"],
        blocks: Blocks::Indent,
        line_comments: HASH_COMMENT,
        block_comments: &[],
        triple_quotes: true,
        interp: Interp::PyFormat,
        semicolons: false,
        operator_continues: false,
            defs: &["def"],
        classes: &["class"],
        imports: Imports::Python,
        ..BASE
    },
    Language { name: "JavaScript", tag: "JS", extensions: &["js", "mjs", "cjs", "jsx"], ..JS_LIKE },
    Language { name: "TypeScript", tag: "TS", extensions: &["ts", "tsx", "mts", "cts"], ..JS_LIKE },
    C,
    CPP,
    OBJC,
    PERL,
    Language {
        name: "Go",
        tag: "GO",
        extensions: &["go"],
        quotes: b"\"`",
        semicolons: false,
            global_names: true,
        defs: &["func"],
        classes: &[],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "Rust",
        tag: "RS",
        extensions: &["rs"],
        quotes: b"\"",
            defs: &["fn"],
        classes: &["impl", "trait", "struct", "enum", "mod"],
        imports: Imports::Path,
        import_words: &["use"],
        ..BASE
    },
    Language {
        name: "PHP",
        tag: "PHP",
        extensions: &["php", "phtml"],
        line_comments: &["//", "#"],
        interp: Interp::Dollar,
        sigils: true,
        global_names: true,
        defs: &["function"],
        classes: &["class", "interface", "trait"],
        imports: Imports::Path,
        import_words: &["use", "require", "require_once", "include", "include_once"],
        type_first: true,
        heredocs: true,
        ..BASE
    },
    Language {
        name: "Java",
        member_calls: true,
        tag: "JAVA",
        extensions: &["java"],
        global_names: true,
        imports: Imports::Path,
        import_words: &["import"],
        type_first: true,
        ..BASE
    },
    Language {
        name: "Kotlin",
        member_calls: true,
        tag: "KT",
        extensions: &["kt", "kts"],
        interp: Interp::Dollar,
        semicolons: false,
        global_names: true,
        defs: &["fun"],
        classes: &["class", "interface", "object"],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "C#",
        member_calls: true,
        tag: "CS",
        extensions: &["cs"],
        global_names: true,
        classes: &["class", "struct", "interface", "namespace"],
        imports: Imports::Path,
        import_words: &["using"],
        type_first: true,
        interp: Interp::PyFormat,
        ..BASE
    },
    Language {
        name: "Ruby",
        member_calls: true,
        tag: "RB",
        extensions: &["rb", "rake", "gemspec"],
        blocks: Blocks::End,
        line_comments: HASH_COMMENT,
        block_comments: &[("=begin", "=end")],
        interp: Interp::Hash,
        ident_extra: b"?!",
        regex_literals: true,
        semicolons: false,
            command_calls: true,
        defs: &["def"],
        classes: &["class", "module"],
        openers: &["if", "unless", "while", "until", "for", "case", "begin"],
        inline_openers: &["do"],
        imports: Imports::Path,
        import_words: &["require", "require_relative", "load"],
        heredocs: true,
        ..BASE
    },
    Language {
        name: "Swift",
        member_calls: true,
        tag: "SWIFT",
        extensions: &["swift"],
        quotes: b"\"",
        interp: Interp::Swift,
        semicolons: false,
        global_names: true,
        defs: &["func"],
        classes: &["class", "struct", "enum", "protocol", "extension"],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "Scala",
        member_calls: true,
        tag: "SCALA",
        extensions: &["scala", "sc"],
        interp: Interp::Dollar,
        semicolons: false,
        defs: &["def"],
        classes: &["class", "object", "trait"],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "Dart",
        member_calls: true,
        tag: "DART",
        extensions: &["dart"],
        interp: Interp::Dollar,
        classes: &["class", "mixin", "extension"],
        imports: Imports::Path,
        import_words: &["import"],
        type_first: true,
        ..BASE
    },
    Language {
        name: "Elixir",
        member_calls: true,
        tag: "EX",
        extensions: &["ex", "exs"],
        blocks: Blocks::End,
        line_comments: HASH_COMMENT,
        block_comments: &[],
        quotes: b"\"",
        triple_quotes: true,
        interp: Interp::Hash,
        ident_extra: b"?!",
        semicolons: false,
        defs: &["def", "defp", "defmacro"],
        classes: &["defmodule"],
        openers: &["if", "unless", "case", "cond", "with", "for"],
        inline_openers: &["fn"],
        imports: Imports::Path,
        import_words: &["alias", "import", "require", "use"],
        ..BASE
    },
    Language {
        name: "Haskell",
        tag: "HS",
        extensions: &["hs", "lhs"],
        blocks: Blocks::Column,
        line_comments: &["--"],
        block_comments: &[("{-", "-}")],
        quotes: b"\"",
        ident_extra: b"'",
        semicolons: false,
        apply_calls: true,
        classes: &[],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "Lua",
        tag: "LUA",
        extensions: &["lua"],
        blocks: Blocks::End,
        line_comments: &["--"],
        block_comments: &[("--[[", "]]")],
        semicolons: false,
            defs: &["function"],
        classes: &[],
        openers: &["if", "for", "while", "repeat"],
        closers: &["end", "until"],
        inline_openers: &["function"],
        imports: Imports::Path,
        import_words: &["require"],
        ..BASE
    },
    Language {
        name: "R",
        tag: "R",
        extensions: &["r", "rmd"],
        line_comments: HASH_COMMENT,
        block_comments: &[],
        semicolons: false,
        defs: &["function"],
        classes: &[],
        imports: Imports::Path,
        import_words: &["library", "source"],
        ..BASE
    },
    Language {
        name: "Solidity",
        member_calls: true,
        tag: "SOL",
        extensions: &["sol"],
        defs: &["function", "modifier"],
        classes: &["contract", "library", "interface"],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "SQL",
        tag: "SQL",
        extensions: &["sql", "ddl"],
        blocks: Blocks::Column,
        line_comments: &["--"],
        quotes: b"'",
        case_insensitive: true,
        semicolons: false,
        defs: &["procedure", "function"],
        classes: &[],
        ..BASE
    },
    Language {
        name: "Shell",
        tag: "SH",
        extensions: &["sh", "bash", "zsh", "ksh"],
        line_comments: HASH_COMMENT,
        block_comments: &[],
        interp: Interp::Dollar,
        sigils: true,
        ident_extra: b"-",
        semicolons: false,
        command_calls: true,
        defs: &["function"],
        classes: &[],
        imports: Imports::Path,
        import_words: &["source"],
        heredocs: true,
        implicit_params: &["1", "2", "3", "4"],
        ..BASE
    },
    Language {
        name: "PowerShell",
        tag: "PS1",
        extensions: &["ps1", "psm1"],
        line_comments: HASH_COMMENT,
        block_comments: &[("<#", "#>")],
        interp: Interp::Dollar,
        sigils: true,
        ident_extra: b"-",
        case_insensitive: true,
        semicolons: false,
        command_calls: true,
        defs: &["function", "filter"],
        classes: &["class"],
        ..BASE
    },
    Language {
        name: "Groovy",
        member_calls: true,
        tag: "GROOVY",
        extensions: &["groovy", "gradle"],
        interp: Interp::Dollar,
        semicolons: false,
        defs: &["def"],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "Julia",
        tag: "JL",
        extensions: &["jl"],
        blocks: Blocks::End,
        line_comments: HASH_COMMENT,
        block_comments: &[("#=", "=#")],
        quotes: b"\"`",
        triple_quotes: true,
        interp: Interp::Dollar,
        ident_extra: b"!",
        semicolons: false,
            defs: &["function"],
        classes: &["struct", "module"],
        openers: &["if", "for", "while", "try", "begin", "let", "quote", "macro"],
        inline_openers: &["do"],
        imports: Imports::Path,
        import_words: &["using", "import", "include"],
        ..BASE
    },
    Language {
        name: "Visual Basic",
        tag: "VB",
        extensions: &["vb", "vbs", "bas", "cls"],
        blocks: Blocks::NamedEnd,
        line_comments: &["'"],
        block_comments: &[],
        quotes: b"\"",
        case_insensitive: true,
        semicolons: false,
        defs: &["sub", "function"],
        classes: &[],
        call_words: &["call"],
        ..BASE
    },
    Language {
        name: "Pascal",
        tag: "PAS",
        extensions: &["pas", "dpr", "pp"],
        blocks: Blocks::BeginEnd,
        line_comments: &["//"],
        block_comments: &[("{", "}"), ("(*", "*)")],
        quotes: b"'",
        case_insensitive: true,
        defs: &["procedure", "function", "constructor", "destructor"],
        classes: &[],
        openers: &["begin", "case", "try", "repeat"],
        closers: &["end", "until"],
        imports: Imports::Path,
        import_words: &["uses"],
        ..BASE
    },
    Language {
        name: "Fortran",
        tag: "F90",
        extensions: &["f90", "f95", "f03", "f08", "for", "f"],
        blocks: Blocks::NamedEnd,
        line_comments: &["!"],
        block_comments: &[],
        case_insensitive: true,
        semicolons: false,
        defs: &["subroutine", "function"],
        classes: &[],
        imports: Imports::Path,
        import_words: &["use"],
        call_words: &["call"],
        ..BASE
    },
    Language {
        name: "COBOL",
        period_ends: true,
        tag: "COBOL",
        extensions: &["cob", "cbl", "cpy"],
        blocks: Blocks::Column,
        line_comments: &["*>"],
        block_comments: &[],
        case_insensitive: true,
        ident_extra: b"-",
        semicolons: false,
        classes: &[],
        call_words: &["perform", "call"],
        ..BASE
    },
    Language {
        name: "Nim",
        tag: "NIM",
        extensions: &["nim"],
        blocks: Blocks::Indent,
        line_comments: HASH_COMMENT,
        block_comments: &[("#[", "]#")],
        quotes: b"\"",
        triple_quotes: true,
        semicolons: false,
        defs: &["proc", "func", "method", "iterator"],
        classes: &["type"],
        imports: Imports::Path,
        import_words: &["import", "include"],
        ..BASE
    },
    Language {
        name: "Crystal",
        member_calls: true,
        tag: "CR",
        extensions: &["cr"],
        blocks: Blocks::End,
        line_comments: HASH_COMMENT,
        block_comments: &[],
        quotes: b"\"",
        interp: Interp::Hash,
        ident_extra: b"?!",
        semicolons: false,
        command_calls: true,
        defs: &["def"],
        classes: &["class", "module", "struct"],
        openers: &["if", "unless", "while", "until", "case", "begin"],
        inline_openers: &["do"],
        imports: Imports::Path,
        import_words: &["require"],
        heredocs: true,
        ..BASE
    },
    Language {
        name: "Zig",
        tag: "ZIG",
        extensions: &["zig"],
        block_comments: &[],
        quotes: b"\"",
        defs: &["fn"],
        classes: &[],
        ..BASE
    },
    Language {
        name: "D",
        member_calls: true,
        tag: "D",
        extensions: &["d"],
        quotes: b"\"`",
        imports: Imports::Path,
        import_words: &["import"],
        type_first: true,
        ..BASE
    },
    Language {
        name: "F#",
        tag: "FS",
        extensions: &["fs", "fsx"],
        blocks: Blocks::Column,
        block_comments: &[("(*", "*)")],
        quotes: b"\"",
        semicolons: false,
        apply_calls: true,
        defs: &["let"],
        classes: &[],
        imports: Imports::Path,
        import_words: &["open"],
        ..BASE
    },
    Language {
        name: "OCaml",
        tag: "ML",
        extensions: &["ml", "mli"],
        blocks: Blocks::Column,
        line_comments: &[],
        block_comments: &[("(*", "*)")],
        quotes: b"\"",
        ident_extra: b"'",
        semicolons: false,
        apply_calls: true,
        defs: &["let"],
        classes: &[],
        imports: Imports::Path,
        import_words: &["open"],
        ..BASE
    },
    Language {
        name: "Erlang",
        period_ends: true,
        period_only: true,
        tag: "ERL",
        extensions: &["erl", "hrl"],
        blocks: Blocks::Column,
        line_comments: &["%"],
        block_comments: &[],
        quotes: b"\"",
        semicolons: false,
        classes: &[],
        ..BASE
    },
    Language {
        name: "Clojure",
        tag: "CLJ",
        extensions: &["clj", "cljs", "cljc"],
        blocks: Blocks::Column,
        line_comments: &[";"],
        block_comments: &[],
        quotes: b"\"",
        ident_extra: b"-?!*'",
        semicolons: false,
        defs: &["defn", "defn-", "defmacro"],
        classes: &[],
        prefix_calls: true,
        ..BASE
    },
    Language {
        name: "Elm",
        tag: "ELM",
        extensions: &["elm"],
        blocks: Blocks::Column,
        line_comments: &["--"],
        block_comments: &[("{-", "-}")],
        quotes: b"\"",
        semicolons: false,
        apply_calls: true,
        classes: &[],
        imports: Imports::Path,
        import_words: &["import"],
        ..BASE
    },
    Language {
        name: "Tcl",
        tag: "TCL",
        extensions: &["tcl"],
        line_comments: HASH_COMMENT,
        block_comments: &[],
        quotes: b"\"",
        interp: Interp::Dollar,
        sigils: true,
        semicolons: false,
        command_calls: true,
        defs: &["proc"],
        classes: &[],
        ..BASE
    },
    Language {
        name: "AWK",
        tag: "AWK",
        extensions: &["awk"],
        line_comments: HASH_COMMENT,
        block_comments: &[],
        quotes: b"\"",
        regex_literals: true,
        semicolons: false,
        global_names: true,
        defs: &["function", "func"],
        classes: &[],
        ..BASE
    },
];

fn line_starts(text: &str, prefix: &str) -> bool {
    text.lines().any(|line| line.trim_start().starts_with(prefix))
}

fn word_then(text: &str, word: &str, ok: impl Fn(&str) -> bool) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.strip_prefix(word).is_some_and(|rest| rest.starts_with([' ', '\t']) && ok(rest.trim_start()))
    })
}

fn is_objc(text: &str) -> bool {
    ["#import", "@interface", "@implementation", "@protocol", "@end", "- (", "+ ("]
        .iter()
        .any(|marker| line_starts(text, marker))
}

fn is_cpp(text: &str) -> bool {
    text.contains("std::")
        || text.contains("template <")
        || text.contains("template<")
        || word_then(text, "namespace", |rest| !rest.is_empty())
        || word_then(text, "class", |rest| rest.contains('{') || rest.contains(':'))
        || ["public:", "private:", "protected:"].iter().any(|word| line_starts(text, word))
}

fn is_matlab(text: &str) -> bool {
    line_starts(text, "function") || line_starts(text, "classdef") || line_starts(text, "%")
}

fn is_perl(text: &str) -> bool {
    (text.starts_with("#!") && text.lines().next().is_some_and(|line| line.contains("perl")))
        || word_then(text, "use", |rest| rest.starts_with(|c: char| c.is_ascii_alphabetic()))
        || word_then(text, "my", |rest| rest.starts_with(['$', '@', '%', '(']))
        || word_then(text, "sub", |rest| rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
        || word_then(text, "package", |rest| rest.trim_end().ends_with(';'))
}

fn is_prolog(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line.starts_with(":-")
            || line.contains(":- ")
            || line.ends_with(":-")
            || (line.ends_with(").") && line.starts_with(|c: char| c.is_ascii_lowercase()))
    })
}

const MPEG_SYNC: u8 = 0x47;
const MPEG_LAYOUTS: [(usize, usize); 2] = [(188, 0), (192, 4)];
const MPEG_MIN_PACKETS: usize = 4;
const MPEG_MAX_PACKETS: usize = 16;

fn is_transport_stream(head: &[u8]) -> bool {
    MPEG_LAYOUTS.iter().any(|&(size, offset)| {
        let packets = (head.len() / size).min(MPEG_MAX_PACKETS);
        packets >= MPEG_MIN_PACKETS && (0..packets).all(|index| head[index * size + offset] == MPEG_SYNC)
    })
}

fn is_qt_translation(text: &str) -> bool {
    let mut rest = text.trim_start();
    if rest.starts_with("<?xml") {
        rest = rest.split_once("?>").map_or("", |pair| pair.1).trim_start();
    }
    if let Some(after) = rest.strip_prefix("<!DOCTYPE TS>") {
        rest = after.trim_start();
    }
    rest.strip_prefix("<TS").is_some_and(|after| after.starts_with([' ', '>', '\t', '\n', '\r']))
}

fn row(tag: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|language| language.tag == tag)
}

/// Which row applies, or None for a file with no grammar here. The suffix
/// decides unless several languages share it; then the bytes do.
pub fn by_content(suffix: &str, head: &[u8]) -> Option<&'static Language> {
    let text = || String::from_utf8_lossy(head);
    match suffix {
        "h" => {
            let text = text();
            if is_objc(&text) {
                row("OBJC")
            } else if is_cpp(&text) {
                row("CPP")
            } else {
                row("C")
            }
        }
        "m" => {
            let text = text();
            if is_objc(&text) {
                row("OBJC")
            } else if is_matlab(&text) || !(text.contains('{') || text.trim().is_empty()) {
                Some(&MATLAB)
            } else {
                row("OBJC")
            }
        }
        "pl" => {
            let text = text();
            if !is_perl(&text) && is_prolog(&text) {
                None
            } else {
                row("PL")
            }
        }
        "ts" => {
            if is_transport_stream(head) || is_qt_translation(&text()) {
                None
            } else {
                row("TS")
            }
        }
        _ => LANGUAGES.iter().find(|language| language.extensions.contains(&suffix)),
    }
}

/// Every suffix the engine understands, for `rust-graph languages`.
pub fn all_suffixes() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut rows: Vec<(&str, &str, &str)> = LANGUAGES
        .iter()
        .flat_map(|language| language.extensions.iter().map(|ext| (*ext, language.tag, language.name)))
        .collect();
    rows.extend([("h", "C", "C"), ("m", "OBJC", "Objective-C or MATLAB, by content"), ("pl", "PL", "Perl")]);
    rows.sort_unstable();
    rows.dedup_by_key(|row| row.0);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(suffix: &str, body: &str) -> Option<&'static str> {
        by_content(suffix, body.as_bytes()).map(|language| language.tag)
    }

    #[test]
    fn shared_suffixes_are_decided_by_content() {
        assert_eq!(tag("h", "struct v { int a; };\n"), Some("C"));
        assert_eq!(tag("h", "namespace n {\n}\n"), Some("CPP"));
        assert_eq!(tag("h", "#import <F/F.h>\n@interface V\n@end\n"), Some("OBJC"));
        assert_eq!(tag("m", "function y = f(c)\nend\n"), Some("M"));
        assert_eq!(tag("m", "int f(void) { return 0; }\n"), Some("OBJC"));
        assert_eq!(tag("pl", "use strict;\nsub f { 1 }\n"), Some("PL"));
        assert_eq!(tag("pl", "parent(tom, bob).\n"), None);
        assert_eq!(tag("ts", "export const y = 1;\n"), Some("TS"));
        assert_eq!(tag("ts", "<!DOCTYPE TS>\n<TS version=\"2.1\">\n"), None);
    }

    #[test]
    fn a_transport_stream_needs_more_than_one_sync_byte() {
        let mut packets = Vec::new();
        for _ in 0..8 {
            packets.push(b'G');
            packets.extend([b' '; 187]);
        }
        assert_eq!(by_content("ts", &packets).map(|l| l.tag), None);
        assert_eq!(tag("ts", &"Gadget.init();\n".repeat(200)), Some("TS"));
    }

    #[test]
    fn the_table_covers_more_than_forty_languages() {
        assert!(LANGUAGES.len() + 1 >= 40, "{}", LANGUAGES.len());
    }
}
