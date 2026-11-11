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

/// Words that say something about a value without being one.
const NOISE: [&str; 37] = [
    "await", "yield", "new", "not", "typeof", "delete", "void", "throw", "raise", "async", "lambda", "in", "is", "and",
    "or", "if", "else", "for", "of", "as", "then", "do", "instanceof", "sizeof", "return", "my", "local", "let", "var",
    "const", "final", "mut", "ref", "unsafe", "defer", "go", "our",
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
            if self.is_arrow(0) {
                // `pat => value`: the pattern is not part of the value.
                out.truncate(start);
                count = 0;
                self.pos += 1;
                continue;
            }
            let at = out.len();
            if count > 0 {
                out.push(',');
            }
            let body = out.len();
            match kind {
                Kind::Ident => self.ident(out, close),
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
                let mut lexer = Lexer::nested(slice, self.lang, line);
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
        let go_word = matches!(word.as_str(), "go" | "defer") && self.lang.tag != "GO";
        if NOISE.contains(&word.as_str()) && !go_word {
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
