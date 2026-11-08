//! `grammar!` for the deep scan, on the std `proc_macro` API alone: no `syn`,
//! no `quote`, no dependencies.
//!
//! ```text
//! grammar! { JSON {
//!     name = "json";                 // the language's name; default: JSON in lower case
//!     lex = super::lex;              // fn(&mut Cursor) -> Kind, written by hand
//!     runtime = crate::deep;         // where the runtime lives; this is the default
//!
//!     tokens {
//!         LBRACE = "{" open braces,  // delimiters pair within a family
//!         RBRACE = "}" close braces,
//!         COMMA = ",",
//!         STRING = "string",
//!         WORD = "word" bad,         // an error by itself
//!         // ... `trivia` for tokens the parser never sees
//!     }
//!
//!     document: _value*,             // the first rule is the root
//!     _value: object | STRING,       // `_`: no node of its own
//!     object: "{" (pair % ",") "}",  // a group: the body cannot see past `}`
//!     pair: STRING ":" _value,
//! } }
//! ```
//!
//! A rule body is a choice of sequences of items: `"text"` (a token by its text),
//! `NAME` (a token), `rule`, `( ... )`, `[ ... ]` or `x?` (optional), `x*`, `x+`,
//! and `x % sep` (any number of `x` separated by the token `sep`).
//!
//! The macro emits a kind constant per token and per named rule (`pair` ->
//! `PAIR`), the kind table, `pub static JSON: Language`, and `parse_<rule>`
//! functions on the runtime. What it checks and how recovery is derived are in
//! `check.rs` and `emit.rs`.

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

mod check;
mod emit;
mod read;

#[proc_macro]
pub fn grammar(input: TokenStream) -> TokenStream {
    match read::grammar(convert(input)).and_then(check::check) {
        Ok(checked) => emit::emit(&checked).parse().expect("grammar!: the generated code is valid Rust"),
        Err(error) => error.into_stream(),
    }
}

/// Where an error points. `None` in unit tests, which run outside the compiler.
pub(crate) type Spot = Option<Span>;

/// The macro's input as plain data, so everything past this file can be tested
/// without the compiler.
#[derive(Clone, Debug)]
pub(crate) struct Tok {
    pub tree: Tree,
    pub spot: Spot,
}

#[derive(Clone, Debug)]
pub(crate) enum Tree {
    Ident(String),
    /// The character, and whether the next token is a punctuation joined to it.
    Punct(char, bool),
    /// The literal as written, quotes and all.
    Literal(String),
    Group(Delim, Vec<Tok>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Delim {
    Paren,
    Bracket,
    Brace,
}

impl Tree {
    pub fn describe(&self) -> String {
        match self {
            Tree::Ident(word) => word.clone(),
            Tree::Punct(c, _) => c.to_string(),
            Tree::Literal(literal) => literal.clone(),
            Tree::Group(Delim::Paren, _) => "( ... )".into(),
            Tree::Group(Delim::Bracket, _) => "[ ... ]".into(),
            Tree::Group(Delim::Brace, _) => "{ ... }".into(),
        }
    }
}

fn convert(stream: TokenStream) -> Vec<Tok> {
    let mut out = Vec::new();
    for tree in stream {
        let spot = Some(tree.span());
        let tree = match tree {
            TokenTree::Ident(ident) => Tree::Ident(ident.to_string()),
            TokenTree::Punct(punct) => Tree::Punct(punct.as_char(), punct.spacing() == Spacing::Joint),
            TokenTree::Literal(literal) => Tree::Literal(literal.to_string()),
            TokenTree::Group(group) => {
                let inner = convert(group.stream());
                match group.delimiter() {
                    Delimiter::Parenthesis => Tree::Group(Delim::Paren, inner),
                    Delimiter::Bracket => Tree::Group(Delim::Bracket, inner),
                    Delimiter::Brace => Tree::Group(Delim::Brace, inner),
                    // An invisible group from a `macro_rules!` expansion: its tokens, inline.
                    Delimiter::None => {
                        out.extend(inner);
                        continue;
                    }
                }
            }
        };
        out.push(Tok { tree, spot });
    }
    out
}

#[derive(Debug)]
pub(crate) struct Error {
    pub message: String,
    pub spot: Spot,
}

impl Error {
    pub fn at(spot: Spot, message: impl Into<String>) -> Self {
        Error { message: message.into(), spot }
    }

    /// `compile_error!("grammar!: ...");`, pointing at the offending tokens.
    fn into_stream(self) -> TokenStream {
        let span = self.spot.unwrap_or_else(Span::call_site);
        let mut message = Literal::string(&format!("grammar!: {}", self.message));
        message.set_span(span);
        let mut arguments = Group::new(Delimiter::Parenthesis, TokenStream::from(TokenTree::Literal(message)));
        arguments.set_span(span);
        let mut bang = Punct::new('!', Spacing::Alone);
        bang.set_span(span);
        let mut semicolon = Punct::new(';', Spacing::Alone);
        semicolon.set_span(span);
        [TokenTree::Ident(Ident::new("compile_error", span)), bang.into(), arguments.into(), semicolon.into()].into_iter().collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Delim, Tok, Tree};

    const PUNCT: &str = "!#$%&*+,-./:;<=>?@^|~";

    /// Rust-like text to [`Tok`]s, as the compiler would hand them over: enough
    /// for the grammars in the unit tests (idents, strings, numbers, punctuation,
    /// groups and `//` comments).
    pub fn toks(src: &str) -> Vec<Tok> {
        let chars: Vec<char> = src.chars().collect();
        let mut at = 0;
        let out = group(&chars, &mut at, None);
        assert_eq!(at, chars.len(), "unbalanced test input");
        out
    }

    fn group(chars: &[char], at: &mut usize, close: Option<char>) -> Vec<Tok> {
        let mut out = Vec::new();
        while *at < chars.len() {
            let c = chars[*at];
            let next = chars.get(*at + 1).copied();
            if c.is_whitespace() {
                *at += 1;
            } else if c == '/' && next == Some('/') {
                while *at < chars.len() && chars[*at] != '\n' {
                    *at += 1;
                }
            } else if Some(c) == close {
                *at += 1;
                return out;
            } else if let Some((delim, closer)) = match c {
                '(' => Some((Delim::Paren, ')')),
                '[' => Some((Delim::Bracket, ']')),
                '{' => Some((Delim::Brace, '}')),
                _ => None,
            } {
                *at += 1;
                let inner = group(chars, at, Some(closer));
                out.push(Tok { tree: Tree::Group(delim, inner), spot: None });
            } else if c == '"' {
                let start = *at;
                *at += 1;
                while chars[*at] != '"' {
                    *at += if chars[*at] == '\\' { 2 } else { 1 };
                }
                *at += 1;
                out.push(Tok { tree: Tree::Literal(chars[start..*at].iter().collect()), spot: None });
            } else if c.is_alphanumeric() || c == '_' {
                let start = *at;
                while *at < chars.len() && (chars[*at].is_alphanumeric() || chars[*at] == '_') {
                    *at += 1;
                }
                let word: String = chars[start..*at].iter().collect();
                let tree = if c.is_ascii_digit() { Tree::Literal(word) } else { Tree::Ident(word) };
                out.push(Tok { tree, spot: None });
            } else {
                assert!(PUNCT.contains(c), "unexpected {c:?} in test input");
                *at += 1;
                out.push(Tok { tree: Tree::Punct(c, next.is_some_and(|next| PUNCT.contains(next))), spot: None });
            }
        }
        assert!(close.is_none(), "unclosed group in test input");
        out
    }
}
