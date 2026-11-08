//! Drop-in proof: the old lexer (`src/deep/lexer.rs`, still driving every
//! grammar) and `deep-lex` over the same 40 inputs.
//!
//! The golden files in `fixtures/differential/golden/` were produced by
//! running the old lexer over `fixtures/differential/<language>/` and recording
//! `(kind name, class, start, end)` for every token, trivia included. The
//! generator is `tests/gen/old_lexer_golden.rs`; see its header for the exact
//! command.
//!
//! What is compared:
//!
//! * every **significant** token: `(start, end)` must be byte-identical and the
//!   token must fall in the same category (identifier, number, string, regex,
//!   template part, punctuation, error);
//! * every **layout** token (NEWLINE, INDENT, DEDENT): same sequence and the
//!   same zero-width offset;
//! * punctuation must cover exactly the same bytes.
//!
//! Trivia (whitespace, comments) is filtered: the old lexer stores it so token
//! spans concatenate back to the source, `deep-lex` leaves it on the floor and
//! never hands it to a parser. That is a representation difference, not a
//! boundary difference.
//!
//! Known differences, asserted rather than tolerated:
//!
//! 1. **JavaScript `>joined`**: the old JS lexer emits one `>` per token so the
//!    grammar can close `Map<K, V>>`. `deep-lex` does maximal munch (`>>` is one
//!    `GtGt`); joining generic closers is a grammar decision, not a lexical
//!    one, and every non-JS language already reads `>>` as one token.
//! 2. **Bad literals**: an unterminated string/template is `BAD_STR` /
//!    `bad_literal` in the old lexers and `String` / template part here. The
//!    span is identical; `deep-lex` has no per-language bad kind, so the grammar
//!    checks the closing byte instead. The test asserts the span stays equal.

use std::fs;
use std::path::PathBuf;

pub use deep_lex::{DeepLexer, Kind, Language, Lexer, Profile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
    Ident,
    Number,
    Str,
    Regex,
    Template,
    Punct,
    Error,
    Layout(LayoutKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    Newline,
    Indent,
    Dedent,
}

/// One old-lexer token as the golden file records it.
#[derive(Clone, Debug)]
pub struct Old {
    pub name: String,
    pub class: String,
    pub start: u32,
    pub end: u32,
}

impl Old {
    fn text<'a>(&self, src: &'a [u8]) -> &'a [u8] {
        &src[self.start as usize..self.end as usize]
    }

    fn layout(&self) -> Option<LayoutKind> {
        match self.name.as_str() {
            "newline" => Some(LayoutKind::Newline),
            "indent" => Some(LayoutKind::Indent),
            "dedent" => Some(LayoutKind::Dedent),
            _ => None,
        }
    }

    fn cat(&self, src: &[u8]) -> Cat {
        if let Some(layout) = self.layout() {
            return Cat::Layout(layout);
        }
        match self.name.as_str() {
            "regex" => return Cat::Regex,
            "template_string" => return Cat::Template,
            "string" | "string_literal" | "raw_string" => return Cat::Str,
            "bad_string" => return Cat::Str,
            "bad_literal" => {
                return if self.text(src).first() == Some(&b'`') {
                    Cat::Template
                } else {
                    Cat::Str
                }
            }
            "number" => return Cat::Number,
            _ => {}
        }
        if self.class == "open" || self.class == "close" {
            return Cat::Punct;
        }
        if self.class == "bad" {
            return match self.text(src).first() {
                Some(b) if b.is_ascii_alphabetic() || *b == b'_' => Cat::Ident,
                _ => Cat::Error,
            };
        }
        let Some(&b0) = self.text(src).first() else {
            return Cat::Error;
        };
        if b0.is_ascii_alphabetic() || b0 == b'_' || b0 == b'$' || b0 >= 0x80 {
            Cat::Ident
        } else if b0.is_ascii_digit()
            || (b0 == b'-' && self.text(src).get(1).map_or(false, u8::is_ascii_digit))
        {
            Cat::Number
        } else if b0 == b'"' || b0 == b'\'' {
            Cat::Str
        } else {
            Cat::Punct
        }
    }
}

pub fn deep_cat(kind: Kind) -> Cat {
    match kind {
        Kind::Ident => Cat::Ident,
        Kind::Number => Cat::Number,
        Kind::String | Kind::RawString => Cat::Str,
        Kind::Regex => Cat::Regex,
        Kind::TemplateHead | Kind::TemplateMiddle | Kind::TemplateTail => Cat::Template,
        Kind::Error => Cat::Error,
        Kind::Newline => Cat::Layout(LayoutKind::Newline),
        Kind::Indent => Cat::Layout(LayoutKind::Indent),
        Kind::Dedent => Cat::Layout(LayoutKind::Dedent),
        _ => Cat::Punct,
    }
}

pub fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/differential")
}

/// One corpus section: a path (`python/01_basic.py`) and its exact bytes.
pub struct Source {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Read `corpus.txt`. A section header is `#name <path> <byte length>` on its
/// own line; the body is exactly that many bytes, so a file that does not end
/// in a newline (or ends in several) survives the round trip.
pub fn read_corpus() -> Vec<Source> {
    let path = fixture_root().join("corpus.txt");
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut rest: &[u8] = &bytes;
    let mut out = Vec::new();
    while let Some(at) = find(rest, b"#name ") {
        rest = &rest[at + 6..];
        let line_end = rest.iter().position(|&b| b == b'\n').expect("header newline");
        let header = std::str::from_utf8(&rest[..line_end]).expect("header is ASCII");
        let (name, length) = header.rsplit_once(' ').expect("path and length");
        let length: usize = length.parse().expect("byte length");
        let body = &rest[line_end + 1..];
        out.push(Source {
            name: name.to_string(),
            bytes: body[..length].to_vec(),
        });
        rest = &body[length..];
    }
    out
}

pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// All golden sections in corpus order.
pub fn read_golden() -> Vec<(String, Vec<Old>)> {
    let path = fixture_root().join("golden.tsv");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out: Vec<(String, Vec<Old>)> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix("#name ") {
            let name = header.rsplit_once(' ').expect("path and count").0;
            out.push((name.to_string(), Vec::new()));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 4, "malformed golden line: {line}");
        out.last_mut().expect("a section header comes first").1.push(Old {
            name: fields[0].to_string(),
            class: fields[1].to_string(),
            start: fields[2].parse().unwrap(),
            end: fields[3].parse().unwrap(),
        });
    }
    out
}

pub fn profile_for(path: &str) -> &'static Profile {
    let language = path.split('/').next().expect("language directory");
    match language {
        "python" => &deep_lex::PYTHON,
        "json" => &deep_lex::JSON,
        "javascript" => &deep_lex::JAVASCRIPT,
        "typescript" => &deep_lex::TYPESCRIPT,
        other => panic!("no profile for {other}"),
    }
}

/// Join the old JavaScript lexer's per-byte `>joined` tokens back into the one
/// token a maximal-munch lexer produces.
pub fn rejoin_joined(tokens: Vec<Old>) -> Vec<Old> {
    let mut out: Vec<Old> = Vec::with_capacity(tokens.len());
    for token in tokens {
        let continues = out
            .last()
            .map_or(false, |previous| previous.name == ">joined" && previous.end == token.start);
        if token.name == ">joined" || (token.name == ">" && continues) {
            match out.last_mut() {
                Some(previous) if continues => previous.end = token.end,
                _ => {
                    let mut run = token;
                    run.name = ">joined".to_string();
                    out.push(run);
                }
            }
        } else {
            out.push(token);
        }
    }
    out
}

pub fn deep_tokens(src: &[u8], profile: &'static Profile) -> Vec<(Kind, u32, u32)> {
    let mut lexer = DeepLexer::for_profile(src, profile);
    let mut out = Vec::new();
    while let Some(token) = lexer.next_token(0) {
        out.push((Kind::from_symbol(token.symbol).unwrap(), token.start, token.end));
    }
    out
}

/// Compare one file. Returns the differences, each naming the token and why.
pub fn compare(src: &[u8], golden: Vec<Old>, profile: &'static Profile) -> Vec<String> {
    let old: Vec<Old> = if profile.language == Language::JavaScript {
        rejoin_joined(golden.into_iter().filter(|t| t.class != "trivia").collect())
    } else {
        golden.into_iter().filter(|t| t.class != "trivia").collect()
    };
    let deep = deep_tokens(src, profile);
    let mut diffs = Vec::new();
    let mut di = 0usize;

    // The old lexer emits a whole template literal as one token; deep-lex
    // splits it at `${`/`}`. Comparing the group's total span proves coverage.
    for (index, token) in old.iter().enumerate() {
        if token.layout().is_some() {
            // Layout must line up position for position: a missing DEDENT is a
            // structural difference, never something to scan past.
            let found = deep
                .get(di)
                .filter(|(kind, _, _)| matches!(kind, Kind::Newline | Kind::Indent | Kind::Dedent))
                .map(|_| di);
            match found {
                Some(at) => {
                    let (kind, start, end) = deep[at];
                    if deep_cat(kind) != token.cat(src) {
                        diffs.push(format!(
                            "layout {} at {}..{}: old {}, deep {:?}",
                            token.name, token.start, token.end, "", kind
                        ));
                    } else if (start, end) != (token.start, token.end) {
                        diffs.push(format!(
                            "layout offset {} at old {}..{} vs deep {start}..{end}",
                            token.name, token.start, token.end
                        ));
                    }
                    di = at + 1;
                }
                None => diffs.push(format!(
                    "missing layout {} at {}..{} (old token {index})",
                    token.name, token.start, token.end
                )),
            }
            continue;
        }

        if token.cat(src) == Cat::Template && token.name == "template_string" {
            let start = token.start;
            let mut at = di;
            while at < deep.len() && deep[at].2 <= token.end {
                at += 1;
            }
            let covered = at > di && deep[di].1 == start && deep[at - 1].2 == token.end;
            if !covered {
                diffs.push(format!(
                    "template {:?} at {}..{} covers deep {:?}",
                    token.name,
                    token.start,
                    token.end,
                    &deep[di..at]
                ));
            }
            di = at;
            continue;
        }

        let Some(&(kind, start, end)) = deep.get(di) else {
            diffs.push(format!(
                "missing token {} at {}..{}",
                token.name, token.start, token.end
            ));
            continue;
        };
        let want = token.cat(src);
        let got = deep_cat(kind);
        if (start, end) != (token.start, token.end) {
            diffs.push(format!(
                "span: old {} {}..{} vs deep {kind:?} {start}..{end}",
                token.name, token.start, token.end
            ));
        } else if want != got {
            diffs.push(format!(
                "category: old {} {}..{} {want:?} vs deep {kind:?} {got:?}",
                token.name, token.start, token.end
            ));
        } else if want == Cat::Punct {
            let old_text = token.text(src);
            let new_text = &src[start as usize..end as usize];
            if old_text != new_text {
                diffs.push(format!(
                    "punctuation text {}..{}: old {:?} vs deep {:?}",
                    token.start, token.end, old_text, new_text
                ));
            }
        }
        di += 1;
    }
    if di < deep.len() {
        diffs.push(format!(
            "{} trailing deep tokens, first {di}: {:?}",
            deep.len() - di,
            &deep[di..(di + 3).min(deep.len())]
        ));
    }
    diffs
}

