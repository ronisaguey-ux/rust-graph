//! Turns the tokens of one expression into its flow form. The form records how
//! values combine, not what they mean: a reader decides what is dangerous.
//!
//!     0                    a constant
//!     ["n","a.b"]          a name or attribute path
//!     ["m",[e,...]]        values combined (operator, format string, mapping)
//!     ["t",[e,...]]        a tuple or list, element by element
//!     ["c",id,recv,[arg]]  a call; `id` indexes the symbol's call table
//!     ["k","name",e]       a keyword argument, only inside a call's arguments

use crate::facts::Call;
use crate::json::push_string;
use crate::langs::Language;
use crate::lexer::{Kind, Lexer, Token};

/// Words that open a statement whose header is an expression of its own.
#[cfg(feature = "event-split")]
const HEADER_WORDS: [&str; 9] = ["if", "while", "for", "foreach", "switch", "catch", "unless", "until", "with"];
/// Words that declare the variable a `for` header binds.
#[cfg(feature = "event-split")]
const DECL_WORDS: [&str; 6] = ["const", "let", "var", "mut", "final", "my"];

/// Words that say something about a value without being one.
const NOISE: [&str; 38] = [
    "await", "yield", "new", "not", "typeof", "delete", "void", "throw", "raise", "async", "lambda", "in", "is", "and",
    "or", "if", "else", "for", "of", "as", "then", "do", "instanceof", "sizeof", "return", "my", "local", "let", "var",
    "const", "final", "mut", "ref", "unsafe", "defer", "go", "our", "pub",
];
const NAMED_ARGS_WITH_COLON: [&str; 5] = ["RB", "CR", "SWIFT", "DART", "EX"];

pub fn lexeme(src: &[u8], token: &Token, lang: &Language) -> String {
    let text = String::from_utf8_lossy(&src[token.start as usize..token.end as usize]);
    if lang.case_insensitive {
        text.to_ascii_lowercase()
    } else {
        text.into_owned()
    }
}

/// The class a call constructs, going by the convention that classes are capitalised.
fn capitalised(path: &str) -> Option<String> {
    let last = path.rsplit('.').next()?;
    (last.starts_with(|ch: char| ch.is_uppercase()) && !last.contains('(')).then(|| path.to_string())
}

pub struct Emit<'a> {
    pub src: &'a [u8],
    pub lang: &'static Language,
    pub toks: &'a [Token],
    pub pos: usize,
    pub calls: &'a mut Vec<Call>,
    /// The name this function's receiver goes by, besides `self` and `this`.
    pub receiver: &'a str,
}

impl Emit<'_> {
    fn tok(&self, offset: usize) -> Option<&Token> {
        self.toks.get(self.pos + offset)
    }

    fn is_op(&self, offset: usize, op: &[u8]) -> bool {
        self.tok(offset).is_some_and(|token| {
            token.kind == Kind::Op && &self.src[token.start as usize..token.end as usize] == op
        })
    }

    fn is_arrow(&self, offset: usize) -> bool {
        self.tok(offset).is_some_and(|token| {
            token.kind == Kind::Op && &self.src[token.start as usize..token.end as usize] == b"=>"
        })
    }

    fn op_byte(&self, offset: usize) -> u8 {
        match self.tok(offset) {
            Some(token) if token.kind == Kind::Op && token.end - token.start == 1 => self.src[token.start as usize],
            _ => 0,
        }
    }

    fn word(&self, offset: usize) -> Option<String> {
        self.tok(offset).filter(|token| token.kind == Kind::Ident).map(|token| lexeme(self.src, token, self.lang))
    }

    fn new_call(&mut self, path: String, line: u32) -> usize {
        self.calls.push(Call { path, line });
        self.calls.len() - 1
    }

    /// Comma-separated values up to `close` (0: the end of the tokens).
    pub fn list(&mut self, out: &mut String, close: u8, in_call: bool) -> usize {
        let mut count = 0;
        loop {
            let before = self.pos;
            let mark = out.len();
            if count > 0 {
                out.push(',');
            }
            let body = out.len();
            if in_call {
                self.argument(out, close);
            } else {
                self.mix(out, close);
            }
            let comma = self.op_byte(0) == b',';
            if self.pos == before && !comma {
                out.truncate(mark);
                break;
            }
            let empty = &out[body..] == "0" && self.pos == before;
            if empty {
                out.truncate(mark);
            } else {
                count += 1;
            }
            if comma {
                self.pos += 1;
                continue;
            }
            break;
        }
        if close != 0 && self.op_byte(0) == close {
            self.pos += 1;
        }
        count
    }

    /// `set(v) { … }` starting at `at`: the index just past the body when the
    /// tokens spell a method definition, and nothing otherwise. One extra word
    /// is allowed in front (`get name() {…}`, `async run() {…}`), because the
    /// accessor word is part of the same definition.
    fn method_end(&self, at: usize) -> Option<usize> {
        let op_at = |index: usize, op: &[u8]| {
            self.toks.get(index).is_some_and(|token| {
                token.kind == Kind::Op && &self.src[token.start as usize..token.end as usize] == op
            })
        };
        // A method is an object entry, so it follows the opening brace or a
        // comma. Without this, `if num_truthy(h) {` in a Rust block read as a
        // method and the block's statements vanished from the value.
        let previous = at.checked_sub(1)?;
        if !op_at(previous, b"{") && !op_at(previous, b",") {
            return None;
        }
        let mut open = at + 1;
        if self.toks.get(open).is_some_and(|token| token.kind == Kind::Ident) {
            open += 1;
        }
        if !op_at(open, b"(") {
            return None;
        }
        let mut depth = 0usize;
        let mut index = open;
        loop {
            let token = self.toks.get(index)?;
            if token.kind == Kind::Op {
                match &self.src[token.start as usize..token.end as usize] {
                    b"(" => depth += 1,
                    b")" => {
                        depth -= 1;
                        if depth == 0 {
                            index += 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            index += 1;
        }
        if !op_at(index, b"{") {
            return None;
        }
        let mut depth = 0usize;
        loop {
            let token = self.toks.get(index)?;
            if token.kind == Kind::Op {
                match &self.src[token.start as usize..token.end as usize] {
                    b"{" => depth += 1,
                    b"}" => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(index + 1);
                        }
                    }
                    _ => {}
                }
            }
            index += 1;
        }
    }

    fn argument(&mut self, out: &mut String, close: u8) {
        if let Some(name) = self.word(0) {
            let colon = self.op_byte(1) == b':' && NAMED_ARGS_WITH_COLON.contains(&self.lang.tag);
            if self.op_byte(1) == b'=' || colon {
                self.pos += 2;
                out.push_str("[\"k\",");
                push_string(out, &name);
                out.push(',');
                self.mix(out, close);
                out.push(']');
                return;
            }
        }
        self.mix(out, close);
    }

    /// Everything up to a comma or closer, combined.
    pub fn mix(&mut self, out: &mut String, close: u8) {
        let start = out.len();
        let mut count = 0;
        while let Some(token) = self.tok(0) {
            let kind = token.kind;
            // `"abc" "def"`: Python and friends concatenate adjacent string
            // literals into one value. Counting two constants against the CST's
            // one node is a difference in how the text is spelled, not a fact.
            if kind == Kind::Str && self.pos > 0 && self.toks[self.pos - 1].kind == Kind::Str {
                self.pos += 1;
                continue;
            }
            if self.is_arrow(0) {
                // In Rust `=>` opens a match arm: the pattern is not part of the
                // value, so it is dropped and the arm's expression stands (the
                // constant form below cost rust 100 files there).
                if !matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") {
                    out.truncate(start);
                    count = 0;
                    self.pos += 1;
                    continue;
                }
                // `map(s => s.name)`: the CST writes one constant for the whole
                // arrow. Dropping only the pattern left the parameter and the
                // body standing as the argument.
                out.truncate(start);
                count = 1;
                self.pos += 1;
                let mut depth = 0usize;
                while let Some(token) = self.tok(0) {
                    let byte = self.op_byte(0);
                    if token.kind == Kind::Op {
                        match byte {
                            b'(' | b'[' | b'{' => depth += 1,
                            b')' | b']' | b'}' if depth > 0 => depth -= 1,
                            _ => {}
                        }
                        if depth == 0 && (byte == b',' || (close != 0 && byte == close)) {
                            break;
                        }
                    }
                    self.pos += 1;
                }
                out.push('0');
                break;
            }
            let at = out.len();
            if count > 0 {
                out.push(',');
            }
            let body = out.len();
            match kind {
                Kind::Ident => {
                    // `{ set(v) { … } }`: a method definition is one value to
                    // the CST, and the lexer read its body as part of the
                    // object. The key is dropped with it, exactly as the CST
                    // drops it, so the merge lists the same items.
                    if close == b'}' {
                        if let Some(end) = self.method_end(self.pos) {
                            self.pos = end;
                            out.push('0');
                        } else {
                            self.ident(out, close);
                        }
                    } else {
                        self.ident(out, close);
                    }
                }
                Kind::Str => self.string(out),
                Kind::Op => {
                    let byte = self.op_byte(0);
                    if byte == b',' || (close != 0 && byte == close) {
                        out.truncate(at);
                        break;
                    }
                    match byte {
                        b'(' => self.group(out),
                        b'[' => self.display(out, b']', "t"),
                        b'{' => self.display(out, b'}', "m"),
                        b'@' if matches!(self.lang.tag, "RB" | "CR") && self.word(1).is_some() => {
                            let name = self.word(1).unwrap_or_default();
                            let line = self.toks[self.pos].line;
                            self.pos += 2;
                            self.chain(out, String::new(), format!("self.{name}"), line);
                        }
                        _ => self.pos += 1,
                    }
                }
                _ => self.pos += 1,
            }
            if out.len() == body {
                out.truncate(at);
            } else {
                count += 1;
            }
        }
        match count {
            0 => out.push('0'),
            1 => {}
            _ => {
                out.insert_str(start, "[\"m\",[");
                out.push_str("]]");
            }
        }
    }

    fn group(&mut self, out: &mut String) {
        let line = self.toks[self.pos].line;
        self.pos += 1;
        if self.lang.prefix_calls {
            if let Some(name) = self.word(0) {
                self.pos += 1;
                let mut path = name;
                while self.op_byte(0) == b'/' || self.op_byte(0) == b'.' {
                    if let Some(next) = self.word(1) {
                        path.push('.');
                        path.push_str(&next);
                        self.pos += 2;
                    } else {
                        break;
                    }
                }
                let id = self.new_call(path, line);
                let mut args = String::new();
                self.list(&mut args, b')', false);
                out.push_str(&format!("[\"c\",{id},0,[{args}]]"));
                return;
            }
        }
        let mut items = String::new();
        let count = self.list(&mut items, b')', false);
        let value = match count {
            0 => String::new(),
            1 => items,
            _ => format!("[\"t\",[{items}]]"),
        };
        self.chain(out, value, String::new(), line);
    }

    fn display(&mut self, out: &mut String, close: u8, form: &str) {
        let line = self.toks[self.pos].line;
        self.pos += 1;
        let mut items = String::new();
        let count = self.list(&mut items, close, false);
        let value = if count == 0 { String::new() } else { format!("[\"{form}\",[{items}]]") };
        self.chain(out, value, String::new(), line);
    }

    fn string(&mut self, out: &mut String) {
        let token = &self.toks[self.pos];
        let line = token.line;
        let mut parts = String::new();
        let mut count = 0;
        if let Some(ranges) = &token.interp {
            for &(start, end) in ranges.iter() {
                let slice = &self.src[start as usize..end as usize];
                let interp_line = line
                    + self.src[token.start as usize..start as usize]
                        .iter()
                        .filter(|&&b| b == b'\n')
                        .count() as u32;
                let mut lexer = Lexer::nested(slice, self.lang, interp_line);
                let mut tokens = Vec::new();
                loop {
                    let inner = lexer.next();
                    match inner.kind {
                        Kind::Eof => break,
                        Kind::Newline => {}
                        _ => tokens.push(inner),
                    }
                }
                let mut nested =
                    Emit { src: slice, lang: self.lang, toks: &tokens, pos: 0, calls: self.calls, receiver: self.receiver };
                let mut value = String::new();
                nested.mix(&mut value, 0);
                if value != "0" {
                    if count > 0 {
                        parts.push(',');
                    }
                    parts.push_str(&value);
                    count += 1;
                }
            }
        }
        self.pos += 1;
        // A string with nothing interpolated in it is a constant, not an
        // absence: `name: "{"` is a field holding a value, and dropping the
        // value would leave the field's name standing for the whole pair.
        let value = match count {
            0 => "0".to_string(),
            1 => parts,
            _ => format!("[\"m\",[{parts}]]"),
        };
        self.chain(out, value, String::new(), line);
    }

    fn ident(&mut self, out: &mut String, close: u8) {
        let token = &self.toks[self.pos];
        let line = token.line;
        let word = lexeme(self.src, token, self.lang);
        if self.lang.call_words.contains(&word.as_str()) {
            self.pos += 1;
            if let Some(name) = self.word(0) {
                if self.op_byte(1) != b'(' {
                    self.pos += 1;
                    let id = self.new_call(name, line);
                    let mut args = String::new();
                    self.list(&mut args, close, false);
                    out.push_str(&format!("[\"c\",{id},0,[{args}]]"));
                }
            }
            return;
        }
        // `f(function (st) { ... })`: a function expression is a value, and the
        // CST records it as one constant. Read as a name, `function` became a
        // call whose arguments were its parameter list, and the statement was
        // blamed for a call the source never made.
        if word == "function" && self.lang.defs.contains(&word.as_str()) {
            self.pos += 1;
            if self.word(0).is_some() {
                self.pos += 1;
            }
            if self.op_byte(0) == b'(' {
                self.pos += 1;
                let mut params = String::new();
                self.list(&mut params, b')', false);
            }
            if self.op_byte(0) == b'{' {
                let mut depth = 0usize;
                loop {
                    let Some(token) = self.tok(0) else { break };
                    let (kind, start, end) = (token.kind, token.start as usize, token.end as usize);
                    self.pos += 1;
                    if kind == Kind::Op {
                        match &self.src[start..end] {
                            b"{" => depth += 1,
                            b"}" => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            out.push('0');
            return;
        }
        let go_word = matches!(word.as_str(), "go" | "defer") && self.lang.tag != "GO";        // `new` is a keyword in the JavaScript family and an ordinary name in
        // Rust: reading it as noise left `new.is_none()` calling nothing, and
        // the record differed by a constant that was never there.
        let noise = NOISE.contains(&word.as_str())
            && !(self.lang.tag == "RS" && word == "new")
            && !(self.lang.tag == "PY" && word == "local");
        if noise && !go_word {
            self.pos += 1;
            return;
        }
        self.pos += 1;
        if self.lang.arg_words.contains(&word.as_str()) {
            out.push_str("[\"n\",\"_\"]");
            return;
        }
        let head = if word == "this" || word == "self" || (!self.receiver.is_empty() && word == self.receiver) {
            "self".to_string()
        } else {
            word
        };
        self.chain(out, String::new(), head, line);
        if self.lang.command_calls || self.lang.apply_calls {
            self.juxtaposed(out, close, line);
        }
    }

    /// `system "ls"` and `f x y`: the name just emitted was a call after all.
    fn juxtaposed(&mut self, out: &mut String, close: u8, line: u32) {
        let Some(next) = self.tok(0) else { return };
        let is_value = matches!(next.kind, Kind::Ident | Kind::Str | Kind::Num)
            || (self.lang.apply_calls && self.op_byte(0) == b'(');
        if !is_value || next.line != line || !out.ends_with("\"]") {
            return;
        }
        if next.kind == Kind::Ident && self.word(0).is_some_and(|word| NOISE.contains(&word.as_str())) {
            return;
        }
        let Some(open) = out.rfind("[\"n\",") else { return };
        let path = out[open + 6..out.len() - 2].to_string();
        out.truncate(open);
        let id = self.new_call(path, line);
        let mut args = String::new();
        if self.lang.apply_calls {
            let mut count = 0;
            while let Some(token) = self.tok(0) {
                let atom = matches!(token.kind, Kind::Ident | Kind::Str | Kind::Num) || self.op_byte(0) == b'(';
                if !atom {
                    break;
                }
                let mut value = String::new();
                match token.kind {
                    Kind::Ident => {
                        let word = self.word(0).unwrap_or_default();
                        self.pos += 1;
                        let line = self.toks[self.pos - 1].line;
                        self.chain(&mut value, String::new(), word, line);
                    }
                    Kind::Str => self.string(&mut value),
                    Kind::Num => self.pos += 1,
                    _ => self.group(&mut value),
                }
                if !value.is_empty() {
                    if count > 0 {
                        args.push(',');
                    }
                    args.push_str(&value);
                    count += 1;
                }
            }
        } else {
            self.list(&mut args, close, true);
        }
        out.push_str(&format!("[\"c\",{id},0,[{args}]]"));
    }

    /// A value followed by any run of `.name`, `[index]` and `(args)`.
    fn chain(&mut self, out: &mut String, mut value: String, mut path: String, line: u32) {
        let mut constructed = None;
        loop {
            // `.collect::<Vec<_>>()`: a turbofish names a type, not a value.
            // Reading its words as names put `Vec` and `_` in the record where
            // the CST has a call and nothing else.
            if self.is_op(0, b"::") && self.is_op(1, b"<") {
                self.pos += 2;
                let mut depth = 1;
                while let Some(token) = self.tok(0) {
                    let text = &self.src[token.start as usize..token.end as usize];
                    if text == b"<" {
                        depth += 1;
                    } else if text == b">" {
                        depth -= 1;
                        if depth == 0 {
                            self.pos += 1;
                            break;
                        }
                    }
                    self.pos += 1;
                }
                continue;
            }
            let separator = self.is_op(0, b".")
                || self.is_op(0, b"->")
                || self.is_op(0, b"::")
                || self.is_op(0, b"?.")
                || (self.is_op(0, b":") && self.op_byte(2) == b'(' && self.lang.tag == "LUA");
            if separator {
                if let Some(name) = self.word(1) {
                    self.pos += 2;
                    path.push('.');
                    path.push_str(&name);
                    continue;
                }
                if path.is_empty() && self.is_op(0, b".") && self.tok(1).is_some_and(|t| t.kind == Kind::Num) {
                    self.pos += 2;
                    continue;
                }
            }
            if self.op_byte(0) == b'[' {
                self.pos += 1;
                let mut index = String::new();
                self.list(&mut index, b']', false);
                continue;
            }
            if self.op_byte(0) == b'(' {
                let line = self.toks[self.pos].line;
                self.pos += 1;
                let (receiver, mut target) = self.split_receiver(&value, &path);
                if target.starts_with('.') {
                    if let Some(class) = constructed.take() {
                        target = format!("{class}(){target}");
                    }
                }
                constructed = capitalised(&target);
                let id = self.new_call(target, line);
                let mut args = String::new();
                self.list(&mut args, b')', true);
                value = format!("[\"c\",{id},{receiver},[{args}]]");
                path.clear();
                continue;
            }
            break;
        }
        let _ = line;
        if path.is_empty() || path.starts_with('.') {
            out.push_str(&value);
        } else {
            out.push_str("[\"n\",");
            push_string(out, &path);
            out.push(']');
        }
    }

    fn split_receiver(&self, value: &str, path: &str) -> (String, String) {
        if path.is_empty() {
            return (if value.is_empty() { "0".into() } else { value.into() }, ".call".into());
        }
        if path.starts_with('.') {
            return (if value.is_empty() { "0".into() } else { value.into() }, path.into());
        }
        match path.rfind('.') {
            Some(dot) => {
                let mut receiver = String::from("[\"n\",");
                push_string(&mut receiver, &path[..dot]);
                receiver.push(']');
                (receiver, path.into())
            }
            None => ("0".into(), path.into()),
        }
    }
}

/// What a segment does with its value. The guard needs the marker to decide
/// which two rows on one line may pair, so it travels with the segment instead
/// of being guessed from the rendered text later.
#[cfg(feature = "event-split")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Assign,
    Call,
    Return,
    Yield,
    Import,
    Condition,
    Value,
}

/// One top-level expression inside a statement: a row the guard can pair on its
/// own, instead of the whole line standing as one.
#[cfg(feature = "event-split")]
#[derive(Debug)]
pub struct Segment {
    /// The names an assignment binds; empty for an expression with no binder.
    pub target: Vec<String>,
    /// The flow form, spelled the way `mix` writes it.
    pub value: String,
    pub line: u32,
    pub marker: Marker,
}

#[cfg(feature = "event-split")]
struct Piece {
    start: usize,
    end: usize,
    target: Option<String>,
    forced: Option<Marker>,
}

#[cfg(feature = "event-split")]
fn one_byte(src: &[u8], token: &Token) -> u8 {
    if token.kind == Kind::Op && token.end - token.start == 1 {
        src[token.start as usize]
    } else {
        0
    }
}

/// The token that closes the group opened at `open`.
#[cfg(feature = "event-split")]
fn matching(src: &[u8], toks: &[Token], open: usize) -> usize {
    let mut depth = 0i32;
    for index in open..toks.len() {
        match one_byte(src, &toks[index]) {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    toks.len() - 1
}

/// `for (const s of xs)`: the name the header binds and the slice that holds
/// the value it walks. Read as a whole group, the declaration and the iterable
/// merged into one tuple and the row count was right while the row was wrong.
#[cfg(feature = "event-split")]
fn for_parts(
    src: &[u8],
    lang: &'static Language,
    toks: &[Token],
    open: usize,
    close: usize,
) -> Option<(usize, usize, String)> {
    let mut name = None;
    let mut index = open + 1;
    while index + 1 < close {
        if toks[index].kind == Kind::Ident {
            let word = lexeme(src, &toks[index], lang);
            if DECL_WORDS.contains(&word.as_str()) && toks[index + 1].kind == Kind::Ident {
                name = Some(lexeme(src, &toks[index + 1], lang));
                index += 2;
                break;
            }
        }
        index += 1;
    }
    let name = name?;
    while index < close {
        if toks[index].kind == Kind::Ident {
            let word = lexeme(src, &toks[index], lang);
            if word == "of" || word == "in" {
                return Some((index + 1, close, name));
            }
        }
        index += 1;
    }
    None
}

/// The names written on the left of an assignment, as the source spells them.
/// A declaration keyword and a type annotation are part of the statement, not
/// part of the name: `const x: Fn = …` binds `x`.
#[cfg(feature = "event-split")]
fn target_text(src: &[u8], lang: &'static Language, toks: &[Token], start: usize, end: usize) -> String {
    let mut from = start;
    while from < end {
        let token = &toks[from];
        if token.kind == Kind::Ident && DECL_WORDS.contains(&lexeme(src, token, lang).as_str()) {
            from += 1;
            continue;
        }
        break;
    }
    let mut to = end;
    let mut depth = 0i32;
    for index in from..end {
        match one_byte(src, &toks[index]) {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b':' if depth == 0 => {
                to = index;
                break;
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for token in &toks[from..to] {
        out.push_str(&lexeme(src, token, lang));
    }
    out
}

/// A method definition read as one row: `{ set(v) { … } }` binds `set` and its
/// body is a constant, the way the CST writes it.
#[cfg(feature = "event-split")]
fn method_head(src: &[u8], lang: &'static Language, toks: &[Token]) -> Option<String> {
    if one_byte(src, toks.first()?) == b'{' && matching(src, toks, 0) + 1 == toks.len() {
        return method_head(src, lang, &toks[1..toks.len() - 1]);
    }
    let first = toks.first().filter(|token| token.kind == Kind::Ident)?;
    let word = lexeme(src, first, lang);
    if HEADER_WORDS.contains(&word.as_str()) || NOISE.contains(&word.as_str()) {
        return None;
    }
    let open = if one_byte(src, toks.get(1)?) == b'(' { 1 } else { 2 };
    if one_byte(src, toks.get(open)?) != b'(' {
        return None;
    }
    let close = matching(src, toks, open);
    if one_byte(src, toks.get(close + 1)?) != b'{' {
        return None;
    }
    if matching(src, toks, close + 1) + 1 != toks.len() {
        return None;
    }
    Some(word)
}

/// Splits one statement's tokens into its top-level expressions. Pure: it never
/// consults the CST, never re-tokenizes, and does not move statement boundaries.
///
/// The rules that keep it predictable:
/// - a call is atomic — `c(callee;[args])` and never also the callee alone;
/// - an `if`/`while`/`for` header is its own row, then the body expression;
/// - a ternary is one row carrying its branches, plus a row per branch;
/// - a chained assignment binds each name, leftmost first;
/// - two `;`-separated statements on one line are two rows.
#[cfg(feature = "event-split")]
pub fn split_events(
    src: &[u8],
    lang: &'static Language,
    toks: &[Token],
    receiver: &str,
    calls: &mut Vec<Call>,
) -> Vec<Segment> {
    let pieces = pieces_of(src, lang, toks);
    let mut rows: Vec<(usize, usize, Option<String>, Option<Marker>, bool)> = Vec::new();
    for piece in pieces {
        expand(src, lang, toks, piece, &mut rows);
    }
    let mut out = Vec::new();
    for (start, end, target, forced, constant) in rows {
        if start >= end || end > toks.len() {
            continue;
        }
        let value = if constant { "0".to_string() } else { form(src, lang, &toks[start..end], receiver, calls) };
        let marker = forced.unwrap_or_else(|| marker_of(src, lang, &toks[start..end], &value));
        out.push(Segment {
            target: target.into_iter().collect(),
            value,
            line: toks[start].line,
            marker,
        });
    }
    out
}

#[cfg(feature = "event-split")]
fn pieces_of(src: &[u8], lang: &'static Language, toks: &[Token]) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    let mut depth = 0i32;
    while index < toks.len() {
        let token = &toks[index];
        let byte = one_byte(src, token);
        if depth == 0 && token.kind == Kind::Ident {
            let word = lexeme(src, token, lang);
            if HEADER_WORDS.contains(&word.as_str())
                && index + 1 < toks.len()
                && one_byte(src, &toks[index + 1]) == b'('
            {
                if index > start {
                    pieces.push(Piece { start, end: index, target: None, forced: None });
                }
                let close = matching(src, toks, index + 1);
                let mut header = None;
                if word == "for" || word == "foreach" {
                    header = for_parts(src, lang, toks, index + 1, close);
                }
                match header {
                    Some((from, to, name)) => pieces.push(Piece {
                        start: from,
                        end: to,
                        target: Some(name),
                        forced: Some(Marker::Condition),
                    }),
                    None => pieces.push(Piece {
                        start: index + 1,
                        end: (close + 1).min(toks.len()),
                        target: None,
                        forced: Some(Marker::Condition),
                    }),
                }
                start = (close + 1).min(toks.len());
                index = start;
                depth = 0;
                continue;
            }
            if word == "else" {
                if index > start {
                    pieces.push(Piece { start, end: index, target: None, forced: None });
                }
                start = index;
            }
        }
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth == 0 => {
                if index > start {
                    pieces.push(Piece { start, end: index, target: None, forced: None });
                }
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    if start < toks.len() {
        pieces.push(Piece { start, end: toks.len(), target: None, forced: None });
    }
    pieces
}

#[cfg(feature = "event-split")]
fn expand(
    src: &[u8],
    lang: &'static Language,
    toks: &[Token],
    piece: Piece,
    rows: &mut Vec<(usize, usize, Option<String>, Option<Marker>, bool)>,
) {
    // A method definition is one row holding the method's name and a constant.
    if piece.forced.is_none() {
        if let Some(name) = method_head(src, lang, &toks[piece.start..piece.end]) {
            rows.push((piece.start, piece.end, Some(name), Some(Marker::Value), true));
            return;
        }
    }
    let equals = top_level_events(src, toks, piece.start, piece.end, b'=');
    if equals.is_empty() {
        rows.push((piece.start, piece.end, piece.target, piece.forced, false));
        return;
    }
    // A chained assignment binds each name, leftmost first: `a = b = c(d)` has
    // an intermediate binder the CST records and the whole-line row hid.
    for (order, &at) in equals.iter().enumerate() {
        let from = if order == 0 { piece.start } else { equals[order - 1] + 1 };
        let target = target_text(src, lang, toks, from, at);
        let body_start = at + 1;
        let body_end = equals.get(order + 1).copied().unwrap_or(piece.end);
        rows.push((body_start, body_end, Some(target), Some(Marker::Assign), false));
        if order + 1 == equals.len() {
            // A ternary's branches are rows of their own as well: the merged
            // form keeps them, and the guard needs each one to pair against.
            for (start, end) in ternary_branches(src, toks, body_start, body_end) {
                rows.push((start, end, None, Some(Marker::Call), false));
            }
        }
    }
}

/// Top-level occurrences of one single-byte operator, skipping its two-byte
/// relatives (`==`, `=>`, `+=`, …).
#[cfg(feature = "event-split")]
fn top_level_events(src: &[u8], toks: &[Token], start: usize, end: usize, want: u8) -> Vec<usize> {
    let mut found = Vec::new();
    let mut depth = 0i32;
    for index in start..end {
        let token = &toks[index];
        let byte = one_byte(src, token);
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ if depth == 0 && byte == want => {
                // `=>` and `==` arrive as one two-byte token, so a single-byte
                // token is the assignment; a `=` glued to `=` is not.
                let next = toks.get(index + 1).map(|token| one_byte(src, token));
                if want != b'=' || next != Some(b'=') {
                    found.push(index);
                }
            }
            _ => {}
        }
    }
    found
}

#[cfg(feature = "event-split")]
fn ternary_branches(src: &[u8], toks: &[Token], start: usize, end: usize) -> Vec<(usize, usize)> {
    let marks = top_level_events(src, toks, start, end, b'?');
    let Some(&question) = marks.first() else { return Vec::new() };
    let colons = top_level_events(src, toks, question + 1, end, b':');
    let Some(&colon) = colons.first() else { return Vec::new() };
    vec![(question + 1, colon), (colon + 1, end)]
}

#[cfg(feature = "event-split")]
fn form(src: &[u8], lang: &'static Language, toks: &[Token], receiver: &str, calls: &mut Vec<Call>) -> String {
    let mut out = String::new();
    let mut emit = Emit { src, lang, toks, pos: 0, calls, receiver };
    emit.mix(&mut out, 0);
    out
}

#[cfg(feature = "event-split")]
fn marker_of(src: &[u8], lang: &'static Language, toks: &[Token], value: &str) -> Marker {
    if let Some(first) = toks.first().filter(|token| token.kind == Kind::Ident) {
        match lexeme(src, first, lang).as_str() {
            "return" => return Marker::Return,
            "yield" => return Marker::Yield,
            "import" | "use" | "require" | "from" => return Marker::Import,
            _ => {}
        }
    }
    if value.starts_with("[\"c\",") {
        Marker::Call
    } else {
        Marker::Value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs::by_content;

    fn form(suffix: &str, body: &str) -> (String, Vec<String>) {
        let lang = by_content(suffix, body.as_bytes()).unwrap();
        let mut lexer = Lexer::new(body.as_bytes(), lang);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next();
            match token.kind {
                Kind::Eof => break,
                Kind::Newline => {}
                _ => tokens.push(token),
            }
        }
        let mut calls = Vec::new();
        let mut emit = Emit { src: body.as_bytes(), lang, toks: &tokens, pos: 0, calls: &mut calls, receiver: "" };
        let mut out = String::new();
        emit.mix(&mut out, 0);
        (out, calls.into_iter().map(|call| call.path).collect())
    }

    #[test]
    fn a_call_is_one_row_and_never_a_merge_of_its_own_callee() {
        // The failure this catches: the callee written twice, once as the call
        // and once as a name beside it, so one source call reads as a merge of
        // a call and a path. Counting the call rows against the call table is
        // the invariant — every shape below is one call per source call, nested
        // through the receiver, never emitted a second time as an operand.
        let shapes = [
            ("js", "a.b(c)"),
            ("js", "f()(g)"),
            ("js", "new Foo(x)"),
            ("js", "a.b().c(d)"),
            ("js", "console.log(a)(b)"),
            ("js", "f(a, g(b))"),
            ("js", "x.y.z(1)"),
            ("js", "a.b(c)(d)(e)"),
            ("py", "f(x).g(y)"),
            ("js", "f(...a)"),
            ("js", "f(function(){})"),
            ("rb", "system \"ls\""),
        ];
        for (suffix, body) in shapes {
            let (value, calls) = form(suffix, body);
            let rows = value.matches("[\"c\",").count();
            assert_eq!(
                rows,
                calls.len(),
                "{suffix} {body}: {rows} call rows for {} calls: {value}",
                calls.len()
            );
            for path in &calls {
                // A bare callee (no receiver) has no business also standing as
                // a value; a method's path stays in the call table.
                let as_name = format!("[\"n\",\"{path}\"]");
                assert!(
                    !path.contains('.') || !value.contains(&as_name),
                    "{suffix} {body}: callee {path} also rendered as a name: {value}"
                );
            }
        }
    }

    #[test]
    fn an_arrow_argument_stays_one_constant_and_a_match_arm_keeps_its_value() {
        // The c5e5996 rule, pinned so it cannot drift back: in the JS family an
        // arrow is one constant for the whole argument, because dropping only
        // the pattern left the parameter and the body standing as the argument.
        assert_eq!(form("js", "xs.map(s => s.name)").0, r#"["c",0,["n","xs"],[0]]"#);
        assert_eq!(form("js", "f(x => { return x; })").0, r#"["c",0,0,[0]]"#);
        assert_eq!(form("ts", "items.filter((x: T) => x.ok)").0, r#"["c",0,["n","items"],[0]]"#);
        // In Rust `=>` opens a match arm, not an arrow: the pattern is dropped
        // and the arm's expression is the value. Reading it as a constant cost
        // rust a hundred files when it was last tried.
        assert_eq!(
            form("rs", "match y { _ => a.b }").0,
            r#"["m",[["n","match"],["n","y"],["m",[["n","a.b"]]]]]"#
        );
    }

    #[cfg(feature = "event-split")]
    #[test]
    fn the_split_emits_rows_in_source_order_and_never_reorders_them() {
        let lang = crate::langs::by_content("js", b"").unwrap();
        let body = "a = f(1); b = g(2); c = h(3)";
        let mut lexer = Lexer::new(body.as_bytes(), lang);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next();
            match token.kind {
                Kind::Eof => break,
                Kind::Newline => {}
                _ => tokens.push(token),
            }
        }
        let mut calls = Vec::new();
        let segments = split_events(body.as_bytes(), lang, &tokens, "", &mut calls);
        assert_eq!(segments.len(), 3, "three statements, three rows: {segments:?}");
        // The statement order is the source order, one row per `;` segment.
        assert_eq!(segments.iter().map(|s| s.target.join("")).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(calls.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["f", "g", "h"]);
        // No row's line is behind the row before it, in a statement that
        // spans lines: an `if` header, its body, then the statement after it.
        let body = "if (ready) {\n  go(x);\n}\nafter(y);";
        let mut lexer = Lexer::new(body.as_bytes(), lang);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next();
            match token.kind {
                Kind::Eof => break,
                Kind::Newline => {}
                _ => tokens.push(token),
            }
        }
        let mut calls = Vec::new();
        let segments = split_events(body.as_bytes(), lang, &tokens, "", &mut calls);
        let lines: Vec<u32> = segments.iter().map(|s| s.line).collect();
        assert!(lines.windows(2).all(|pair| pair[0] <= pair[1]), "rows out of order: {lines:?}");
        assert_eq!(lines.first(), Some(&1));
    }

    #[test]
    fn an_object_method_is_a_constant() {
        // `{ set(v) { … } }`: the CST writes the method as one value, and the
        // lexer read the body into the object's merge.
        assert_eq!(form("js", "{ set(v) { v; } }").0, r#"["m",[0]]"#);
        assert_eq!(form("js", "{ get name() { return a; } }").0, r#"["m",[0]]"#);
    }

    #[test]
    fn a_function_expression_is_a_constant() {
        // `f(function (st) { st; })`: the CST records the function expression
        // as one value, and reading `function` as a name made it a call whose
        // arguments were the parameter list.
        let (value, calls) = form("js", "f(function (st) { st; })");
        assert_eq!(value, r#"["c",0,0,[0]]"#);
        assert_eq!(calls, ["f"]);
        let (value, _) = form("js", "f(function st(x) { g(x); })");
        assert_eq!(value, r#"["c",0,0,[0]]"#);
    }

    #[test]
    fn a_constant_is_zero_and_a_name_is_a_path() {
        assert_eq!(form("py", "'x'").0, "0");
        assert_eq!(form("py", "a.b[1].c").0, r#"["n","a.b.c"]"#);
    }

    #[test]
    fn operators_and_format_strings_combine_values() {
        // A string with nothing interpolated is the constant it is: it used to
        // vanish from the sum while a lone string was already 0.
        assert_eq!(form("py", "'s ' + a + b").0, r#"["m",[0,["n","a"],["n","b"]]]"#);
        assert_eq!(form("py", "f'{a} {b.c}'").0, r#"["m",[["n","a"],["n","b.c"]]]"#);
        assert_eq!(form("js", "`x ${a}`").0, r#"["n","a"]"#);
    }

    #[test]
    fn a_call_keeps_its_receiver_and_keywords() {
        let (out, calls) = form("py", "a.b(x, shell=True).c(y)");
        assert_eq!(calls, ["a.b", ".c"]);
        assert_eq!(out, r#"["c",1,["c",0,["n","a"],[["n","x"],["k","shell",["n","True"]]]],[["n","y"]]]"#);
    }

    #[test]
    fn a_tuple_stays_element_by_element() {
        assert_eq!(form("py", "(a, 'k', b)").0, r#"["t",[["n","a"],0,["n","b"]]]"#);
    }

    #[test]
    fn calls_without_parentheses_are_calls() {
        let (out, calls) = form("rb", "system \"ls #{a}\"");
        assert_eq!(calls, ["system"]);
        assert_eq!(out, r#"["c",0,0,[["n","a"]]]"#);
        assert_eq!(form("hs", "f x y").1, ["f"]);
    }
}

#[cfg(all(test, feature = "event-split"))]
mod segment_tests {
    use super::*;
    use crate::langs::by_content;

    /// The rows one statement decomposes into: (targets, form, marker).
    fn rows(suffix: &str, body: &str) -> Vec<(Vec<String>, String, Marker)> {
        let lang = by_content(suffix, body.as_bytes()).unwrap();
        let mut lexer = Lexer::new(body.as_bytes(), lang);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next();
            match token.kind {
                Kind::Eof => break,
                Kind::Newline => {}
                _ => tokens.push(token),
            }
        }
        let mut calls = Vec::new();
        split_events(body.as_bytes(), lang, &tokens, "", &mut calls)
            .into_iter()
            .map(|segment| (segment.target, segment.value, segment.marker))
            .collect()
    }

    #[test]
    fn an_if_header_and_its_body_are_two_rows() {
        let rows = rows("js", "if (a) b(c);");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].2, Marker::Condition);
        assert_eq!(rows[0].1, r#"["n","a"]"#);
        assert_eq!(rows[1].2, Marker::Call);
        assert_eq!(rows[1].1, r#"["c",0,0,[["n","c"]]]"#);
    }

    #[test]
    fn a_for_header_binds_its_variable() {
        let rows = rows("js", "for (const s of xs) collect(s);");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].0, ["s"]);
        assert_eq!(rows[0].2, Marker::Condition);
        assert_eq!(rows[0].1, r#"["n","xs"]"#);
        assert_eq!(rows[1].2, Marker::Call);
        assert_eq!(rows[1].1, r#"["c",0,0,[["n","s"]]]"#);
    }

    #[test]
    fn a_ternary_keeps_its_branches_as_rows() {
        let rows = rows("js", "x = a ? b(c) : d(e);");
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[0].0, ["x"]);
        assert_eq!(rows[0].2, Marker::Assign);
        assert_eq!(rows[0].1, r#"["m",[["n","a"],["c",0,0,[["n","c"]]],["c",1,0,[["n","e"]]]]]"#);
        assert_eq!(rows[1].1, r#"["c",2,0,[["n","c"]]]"#);
        assert_eq!(rows[2].1, r#"["c",3,0,[["n","e"]]]"#);
        assert_eq!(rows[1].2, Marker::Call);
        assert_eq!(rows[2].2, Marker::Call);
    }

    #[test]
    fn a_return_is_one_row() {
        let rows = rows("js", "return obj.m(arg);");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].2, Marker::Return);
        // The callee path lives in the call table, the form carries its id.
        assert_eq!(rows[0].1, r#"["c",0,["n","obj"],[["n","arg"]]]"#);
    }

    #[test]
    fn a_chained_assignment_binds_each_name() {
        let rows = rows("js", "a = b = c(d);");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].0, ["a"]);
        assert_eq!(rows[0].1, r#"["n","b"]"#);
        assert_eq!(rows[1].0, ["b"]);
        assert_eq!(rows[1].1, r#"["c",0,0,[["n","d"]]]"#);
        assert_eq!(rows[0].2, Marker::Assign);
        assert_eq!(rows[1].2, Marker::Assign);
    }

    #[test]
    fn a_nested_call_stays_one_atomic_row() {
        let rows = rows("js", "f(g(h(x)));");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].1, r#"["c",0,0,[["c",1,0,[["c",2,0,[["n","x"]]]]]]]"#);
        assert_eq!(rows[0].2, Marker::Call);
    }

    #[test]
    fn an_indexed_target_is_spelled_as_written() {
        let rows = rows("js", "a[i] = j(k);");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].0, ["a[i]"]);
        assert_eq!(rows[0].1, r#"["c",0,0,[["n","k"]]]"#);
        assert_eq!(rows[0].2, Marker::Assign);
    }

    #[test]
    fn a_while_header_and_its_body_are_two_rows() {
        let rows = rows("js", "while (f()) tick();");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].2, Marker::Condition);
        assert_eq!(rows[0].1, r#"["c",0,0,[]]"#);
        assert_eq!(rows[1].2, Marker::Call);
        assert_eq!(rows[1].1, r#"["c",1,0,[]]"#);
    }

    #[test]
    fn a_function_expression_is_one_constant_row() {
        let rows = rows("js", "x = function (p) { return p; };");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].0, ["x"]);
        assert_eq!(rows[0].1, "0");
        assert_eq!(rows[0].2, Marker::Assign);
    }

    #[test]
    fn an_object_method_is_one_named_row() {
        let rows = rows("js", "{ set(v) { this._v = v; } }");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].0, ["set"]);
        assert_eq!(rows[0].1, "0");
    }

    #[test]
    fn two_statements_on_one_line_are_two_rows() {
        // The class the sweep finds in 1,100 files: the one-pass lexer wrote one
        // row per statement, so a call after a condition had no row of its own.
        let rows = rows("js", "a = 1; b = 2;");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].0, ["a"]);
        assert_eq!(rows[1].0, ["b"]);
    }
}
