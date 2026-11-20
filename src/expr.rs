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
/// Keywords the lexer skips in the JavaScript family. `NOISE` below is the
/// cross-language list, and most of its entries (`lambda`, `is`, `and`, `my`,
/// `pub`) are ordinary identifiers in JavaScript: dropping them lost names the
/// source really read.
const JS_NOISE: [&str; 28] = [
    "await", "yield", "new", "typeof", "delete", "void", "throw", "async", "in", "of", "if", "else", "for", "do",
    "instanceof", "return", "let", "var", "const", "function", "class", "switch", "case", "try", "catch", "finally",
    "while", "as",
];
const PY_NOISE: [&str; 16] = [
    "await", "yield", "not", "raise", "async", "lambda", "in", "is", "and", "or", "if", "else", "for", "as", "return",
    "from",
];
const NOISE: [&str; 38] = [
    "await", "yield", "new", "not", "typeof", "delete", "void", "throw", "raise", "async", "lambda", "in", "is", "and",
    "or", "if", "else", "for", "of", "as", "then", "do", "instanceof", "sizeof", "return", "my", "local", "let", "var",
    "const", "final", "mut", "ref", "unsafe", "defer", "go", "our", "pub",
];
/// Words the Rust reader must skip. `NOISE` above is a mix of JavaScript,
/// Python and Perl operators, and only a handful of them are Rust keywords;
/// skipping the rest lost real names (`self.line(lambda)` read as a call with
/// the constant 0, because `lambda` was treated as Python's lambda keyword).
const RS_NOISE: [&str; 20] = [
    "await", "yield", "async", "in", "as", "if", "else", "for", "while", "loop", "return", "let", "const", "mut",
    "ref", "unsafe", "pub", "move", "try", "use",
];
const NAMED_ARGS_WITH_COLON: [&str; 5] = ["RB", "CR", "SWIFT", "DART", "EX"];
/// Words [`NOISE`] lists for another language that are ordinary Kotlin names.
/// `my` is Perl's declarator and `let` is a stdlib function Kotlin calls:
/// skipping them lost the first segment of `package my.qa.noname` (`qa.noname`
/// against `my.qa.noname`) and the callee of `text.let { … }`.
const KT_NAMES: [&str; 4] = ["my", "local", "our", "let"];
/// Kotlin keywords that take a parenthesised header without being a call.
/// `when` is a keyword in Kotlin and a name nowhere, so `when (x) { … }` read as
/// the call `when(x)` in every file that used it as a value (`val p = when (x) …`),
/// which is the lexer saying a call the CST never writes.
const KT_NOISE: [&str; 1] = ["when"];
/// Words that describe a C declaration instead of naming a value. `const` and
/// `void` are already in [`NOISE`]; the rest are the storage classes and
/// type specifiers the one-pass reader emitted as names, so `static int h(int
/// x);` carried `n:static` and `extern hidden void g(int);` carried `n:extern`
/// where the CST writes the type and the declared name alone.
pub(crate) const C_SPECIFIERS: [&str; 20] = [
    "static", "extern", "register", "inline", "auto", "typedef", "_Noreturn", "thread_local", "_Thread_local",
    "__inline", "__inline__", "volatile", "restrict", "__restrict", "__restrict__", "signed", "unsigned", "short",
    "long", "struct",
];
/// The C words a declaration specifier can precede. A word in front of one of
/// these is an annotation macro (`LUAI_FUNC const TValue *f(..)`,
/// `hidden size_t f(..)`), not a value: no expression writes a name and then a
/// type, and every one of them was a name the source never read.
const C_TYPE_WORDS: [&str; 15] = [
    "void", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "_Bool", "_Complex", "const",
    "volatile", "restrict", "struct",
];

/// Whether a word opens a C declaration: a storage class or a type
/// specifier. `int __cxa_atexit(void (*)(void *), void *);` and `static double
/// pzero(double), qzero(double);` open with one, and the one-pass reader wrote
/// the declarator as a call.
pub(crate) fn c_declaration_word(word: &str) -> bool {
    C_SPECIFIERS.contains(&word) || C_TYPE_WORDS.contains(&word) || matches!(word, "enum" | "union" | "_Atomic")
}

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

    /// `async (x) => …`: the parenthesised list is an arrow's parameter list,
    /// and `async` there names nothing. `async(callback)` is a plain call and is
    /// settled by the caller, not by the word alone. `open_at` is the offset of
    /// the `(` from the word under the cursor.
    fn async_arrow_head(&self, open_at: usize) -> bool {
        if !self.is_op(open_at, b"(") {
            return false;
        }
        let Some(after) = self.skip_balanced(self.pos + open_at, b"(", b")") else { return false };
        let Some(token) = self.toks.get(after) else { return false };
        token.kind == Kind::Op && &self.src[token.start as usize..token.end as usize] == b"=>"
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
        if self
            .toks
            .get(open)
            .is_some_and(|token| matches!(token.kind, Kind::Ident | Kind::Str | Kind::Num))
        {
            open += 1;
        }
        if matches!(self.lang.tag, "TS" | "TSX") && op_at(open, b"<") {
            open = self.skip_balanced(open, b"<", b">")?;
        }
        if !op_at(open, b"(") {
            return None;
        }
        let mut index = self.skip_balanced(open, b"(", b")")?;
        if matches!(self.lang.tag, "TS" | "TSX") && op_at(index, b":") {
            index = self.skip_return_type(index + 1)?;
        }
        if !op_at(index, b"{") {
            return None;
        }
        self.skip_balanced(index, b"{", b"}")
    }

    fn skip_balanced(&self, start: usize, open: &[u8], close: &[u8]) -> Option<usize> {
        let mut depth = 0usize;
        let mut index = start;
        loop {
            let token = self.toks.get(index)?;
            if token.kind == Kind::Op {
                let raw = &self.src[token.start as usize..token.end as usize];
                if raw == open {
                    depth += 1;
                } else if raw == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + 1);
                    }
                } else if close == b">" && matches!(raw, b">>" | b">>>") {
                    depth = depth.saturating_sub(raw.len());
                    if depth == 0 {
                        return Some(index + 1);
                    }
                }
            }
            index += 1;
        }
    }

    fn skip_return_type(&self, mut index: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut body = None;
        loop {
            let Some(token) = self.toks.get(index) else {
                return body;
            };
            if token.kind == Kind::Op {
                let raw = &self.src[token.start as usize..token.end as usize];
                match raw {
                    b"(" | b"[" | b"<" => depth += 1,
                    b")" | b"]" | b">" => {
                        if depth == 0 {
                            return body;
                        }
                        depth -= 1;
                    }
                    b">>" | b">>>" => {
                        if depth == 0 {
                            return body;
                        }
                        depth = depth.saturating_sub(raw.len());
                    }
                    b"{" if depth == 0 => {
                        body = Some(index);
                        // A braced return type (`: { x: number } | null { … }`) is
                        // followed by type continuations and the method body `{`; a
                        // method body is followed by `,` or `}`.
                        let Some(after) = self.skip_balanced(index, b"{", b"}") else {
                            return body;
                        };
                        let continues = self.toks.get(after).is_some_and(|next| {
                            let next_raw = &self.src[next.start as usize..next.end as usize];
                            (next.kind == Kind::Op
                                && matches!(next_raw, b"{" | b"|" | b"&" | b"[" | b"?" | b":"))
                                || (next.kind == Kind::Ident && next_raw == b"extends")
                        });
                        if !continues {
                            return body;
                        }
                        index = after;
                        continue;
                    }
                    b"{" => depth += 1,
                    b"}" if depth > 0 => depth -= 1,
                    b"," | b";" | b"=" | b"}" if depth == 0 => return body,
                    _ => {}
                }
            }
            index += 1;
        }
    }

    /// If `index` (right after `:`) starts a TypeScript arrow return type
    /// (`(params): ReturnType => ...`), return the token index of `=>`.
    fn ts_arrow_after_return_type(&self, mut index: usize) -> Option<usize> {
        let mut depth = 0usize;
        while let Some(token) = self.toks.get(index) {
            if token.kind == Kind::Op {
                let raw = &self.src[token.start as usize..token.end as usize];
                match raw {
                    b"(" | b"[" | b"{" | b"<" => depth += 1,
                    b")" | b"]" | b"}" | b">" => {
                        if depth == 0 {
                            return None;
                        }
                        depth -= 1;
                    }
                    b">>" | b">>>" => {
                        if depth == 0 {
                            return None;
                        }
                        depth = depth.saturating_sub(raw.len());
                    }
                    b"=>" if depth == 0 => return Some(index),
                    b"," | b";" | b"=" | b"&&" | b"||" | b"??" | b"==" | b"===" | b"!="
                    | b"!==" | b"<=" | b">=" | b"+=" | b"-=" | b"*=" | b"/=" | b"%="
                        if depth == 0 =>
                    {
                        return None;
                    }
                    _ => {}
                }
            }
            index += 1;
        }
        None
    }

    fn argument(&mut self, out: &mut String, close: u8) {
        if let Some(name) = self.word(0) {
            let colon = self.op_byte(1) == b':' && NAMED_ARGS_WITH_COLON.contains(&self.lang.tag);
            // `isNaN(it = +it)` in JS/TS and `g(x = 1)` / `cfg!(all(feature = "deep"))`
            // in Rust: `=` in an argument is an assignment expression or macro key-value
            // token, not a keyword argument — the CST writes a merge, never `k(...)`.
            let assigned =
                self.op_byte(1) == b'=' && !matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX" | "RS");
            if assigned || colon {
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
        // Calls written by the pattern: in Rust `Pat => value` drops everything
        // before the arrow, so a call the pattern spelled — `Some(q @ ..)` in
        // `Some(q @ (A | B)) => true` — has to leave the call table with its
        // text. The text was truncated and the record kept a call no CST writes.
        let calls_at_start = self.calls.len();
        let mut count = 0;
        // Where the first value of this item ends. An object property whose
        // value is an arrow keeps its key: `{ identify: v => … }` is one pair,
        // and the arrow rule below must truncate to the end of the key, not to
        // the start of the item (55 js losses: yaml tags, config tables).
        let mut first_end = 0usize;
        // The word that opened the first item. `delete: (r) => …` opens with a
        // keyword, so ident() never runs for the key and the first value came
        // from the arrow's parameter list: that is not a key/value pair.
        let mut first_word: Option<String> = None;
        // Where the item just written began. A conditional lifts its condition
        // out of the list, and the condition's own text holds commas, so the
        // boundary has to be remembered rather than searched for.
        let mut item_start = start;
        // Where the pattern ends. In `Pat if guard => value` the guard is a
        // value and its calls are real, so the arrow drops only the calls the
        // pattern wrote — those before the guard's `if`.
        let mut calls_after_pattern = calls_at_start;
        let mut guard_seen = false;
        let mut in_let = false;
        // `T { a, .. }`: the rest pattern names no value, and the CST writes
        // none. The generic empty-item rule at the end of this function pushes
        // a constant `0` for an item that produced nothing, which is right for
        // an absent operand and wrong for a rest — that `0` was the whole of
        // `assert_matches!(r, ActivityTask { variant: Some(x), .. })`.
        let mut rest_only = false;
        while let Some(token) = self.tok(0) {
            let kind = token.kind;
            if self.lang.tag == "RS" && kind == Kind::Ident && self.word(0).as_deref() == Some("if") {
                calls_after_pattern = self.calls.len();
                guard_seen = true;
            }
            if self.lang.tag == "RS" && kind == Kind::Ident && self.word(0).as_deref() == Some("let") {
                in_let = true;
            }
            // Inside a Kotlin `{ ... }` lambda body, `val x = rhs` / `var x = rhs`
            // and `prop = rhs` are declarations/assignments whose LHS names are
            // not part of the lambda's value; only the RHS expression is.
            if self.lang.tag == "KT" && close == b'}' && kind == Kind::Ident {
                if matches!(self.word(0).as_deref(), Some("val" | "var")) {
                    let mut at = 1usize;
                    let mut depth = 0i32;
                    let mut eq_at = None;
                    while let Some(next) = self.tok(at) {
                        if next.kind == Kind::Op {
                            if depth == 0 && self.raw_is(next, b"=") {
                                eq_at = Some(at);
                                break;
                            }
                            if depth == 0 && (self.raw_is(next, b";") || self.raw_is(next, b"}")) {
                                break;
                            }
                            match &self.src[next.start as usize..next.end as usize] {
                                b"(" | b"[" | b"<" | b"{" => depth += 1,
                                b")" | b"]" | b">" | b"}" if depth > 0 => depth -= 1,
                                _ => {}
                            }
                        }
                        at += 1;
                    }
                    if let Some(at) = eq_at {
                        self.pos += at + 1;
                        continue;
                    }
                } else if self.is_op(1, b"=") {
                    self.pos += 2;
                    continue;
                }
            }
            // `pub(crate) meter: u64` — a visibility restriction is not a value:
            // the CST writes the field's name and type only, and `crate` in the
            // value lost every pub(crate) field line.
            if self.lang.tag == "RS" && kind == Kind::Ident && self.word(0).as_deref() == Some("pub") && self.op_byte(1) == b'(' {
                let mut at = 2;
                while let Some(next) = self.tok(at) {
                    if self.raw_is(next, b")") {
                        break;
                    }
                    at += 1;
                }
                self.pos += at + 1;
                continue;
            }
            // `|&id| self.nodes[id as usize].start`: a closure's parameter list
            // is not a value. Read as a bit-or, the parameter merged with the
            // body and the CST, which writes the body alone, lost the line
            // (50 corpus files).
            if self.lang.tag == "RS"
                && self.op_byte(0) == b'|'
                && self.op_byte(1) == b'&'
                && out.len() == start
            {
                let opening = self.toks[self.pos].line;
                let mut at = 1;
                let mut close_bar = None;
                while let Some(next) = self.tok(at) {
                    // A parameter list sits on one line: `a | b` with another
                    // `|` further down the block is a bit-or, not a closure.
                    if next.line != opening {
                        break;
                    }
                    if self.raw_is(next, b"|") {
                        close_bar = Some(at);
                        break;
                    }
                    let shape_ok = matches!(next.kind, Kind::Ident)
                        || ["&", ",", ":", "<", ">", "'", "(", ")", "[", "]"].iter().any(|op| self.raw_is(next, op.as_bytes()));
                    if !shape_ok {
                        break;
                    }
                    at += 1;
                }
                if let Some(close_bar) = close_bar {
                    self.pos += close_bar + 1;
                    continue;
                }
            }
            // `"abc" "def"`: Python and friends concatenate adjacent string
            // literals into one value. Counting two constants against the CST's
            // one node is a difference in how the text is spelled, not a fact.
            if kind == Kind::Str && self.pos > 0 && self.toks[self.pos - 1].kind == Kind::Str {
                self.pos += 1;
                // `'a, ' 'b {}'.format(x)`: the two literals are one value and
                // the dot binds to it, so the call is the value, not a second
                // item beside the literal. Only this shape reaches here — a
                // general dot continuation also swallowed JS index chains and
                // wrote a call with an empty receiver.
                if count == 1 && self.op_byte(0) == b'.' && self.word(1).is_some() {
                    let value = out.split_off(start);
                    let line = self.toks[self.pos].line;
                    self.chain(out, value, String::new(), line);
                }
                continue;
            }
            if self.is_arrow(0) || (self.lang.tag == "KT" && self.is_op(0, b"->")) {
                // In Rust `=>` opens a match arm: the pattern is not part of the
                // value, so it is dropped and the arm's expression stands (the
                // constant form below cost rust 100 files there).
                // In Kotlin `->` opens a `when` arm or lambda body: the
                // condition or parameter list before `->` is not part of the
                // body's value, though any call in a `when` condition is kept.
                if !matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") {
                    out.truncate(start);
                    if self.lang.tag != "KT" {
                        if guard_seen {
                            // The pattern's calls sit between the item's start and the
                            // guard's `if`; the guard's own calls follow it and are real.
                            let end = calls_after_pattern.min(self.calls.len());
                            self.calls.drain(calls_at_start.min(end)..end);
                        } else {
                            self.calls.truncate(calls_at_start);
                        }
                    }
                    count = 0;
                    self.pos += 1;
                    continue;
                }
                // `map(s => s.name)`: the CST writes one constant for the whole
                // arrow. Dropping only the pattern left the parameter and the
                // body standing as the argument.
                // `{ key: v => … }` keeps the key and makes the value a
                // constant: truncate to the end of the key and write the second
                // item, so the pair wraps exactly as `key: value` does.
                // One value already written is the key: `createNode: () => new
                // Scalar(null)` spends its parameter list inside group() and
                // leaves count at 1, so count >= 2 was too strict.
                let key = first_word.as_deref().map(|w| format!("[\"n\",{w:?}]"));
                let paired = close == b'}' && count >= 1 && first_end > start
                    && key.is_some_and(|k| out[start..first_end] == k[..]);
                if paired {
                    out.truncate(first_end);
                    out.push_str(",0");
                } else {
                    out.truncate(start);
                    out.push('0');
                }
                count = if paired { 2 } else { 1 };
                self.pos += 1;
                let mut depth = 0usize;
                while let Some(token) = self.tok(0) {
                    let byte = self.op_byte(0);
                    if token.kind == Kind::Op {
                        if depth == 0 && (byte == b',' || (close != 0 && byte == close)) {
                            break;
                        }
                        match byte {
                            b'(' | b'[' | b'{' => depth += 1,
                            b')' | b']' | b'}' if depth > 0 => depth -= 1,
                            _ => {}
                        }
                    }
                    self.pos += 1;
                }
                break;
            }
            if count == 0 {
                first_word = self.word(0).map(|w| w.to_string());
            }
            let at = out.len();
            if count > 0 {
                out.push(',');
            }
            let body = out.len();
            match kind {
                Kind::Ident => {
                    // Swift `@Sendable (…) -> Void` and `@escaping (…) -> Void`: the word behind the
                    // `@` is an attribute on a closure type, never a call. Read as a call it invented
                    // `call Sendable` / `call escaping` on every closure-typed parameter and alias.
                    if self.lang.tag == "SWIFT" {
                        let start = self.toks[self.pos].start as usize;
                        let after_at = start > 0 && self.src[start - 1] == b'@';
                        if after_at {
                            self.pos += 1;
                            if self.op_byte(0) == b'(' {
                                let mut skipped = String::new();
                                self.list(&mut skipped, close, false);
                            }
                            if count > 0 {
                                out.truncate(at);
                            }
                            break;
                        }
                    }
                    // `x as T` and `x satisfies T`: everything after the keyword
                    // is a type, erased at runtime, so the value ends there.
                    // Left as a name it became a merge item beside the value and
                    // the CST never wrote it.
                    if matches!(self.lang.tag, "TS" | "TSX")
                        && count > 0
                        && matches!(self.word(0).as_deref(), Some("as") | Some("satisfies"))
                    {
                        // The separator for this item is already written; drop it
                        // with the item, or the value ends in a stray comma and
                        // the event JSON stops parsing.
                        out.truncate(at);
                        break;
                    }
                    // `{ set(v) { … } }`: a method definition is one value to
                    // the CST, and the lexer read its body as part of the
                    // object. The key is dropped with it, exactly as the CST
                    // drops it, so the merge lists the same items.
                    let js_family = matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX");
                    if js_family && close == b'}' && self.op_byte(1) == b':' {
                        // `{ delete: 1 }` and `{ q: 1 }`: a key whose value is
                        // a constant is one pair to the CST. The keyword key is
                        // dropped by ident() as noise, and a number writes no
                        // item of its own, so the pair vanished.
                        let word = self.word(0).unwrap_or_default();
                        let constant = self.toks.get(self.pos + 2).is_some_and(|token| token.kind == Kind::Num);
                        if constant {
                            self.pos += 3;
                            out.push_str("[\"m\",[[\"n\",");
                            push_string(out, &word);
                            out.push_str("],0]]");
                        } else if NOISE.contains(&word.as_str()) && !self.lang.defs.contains(&word.as_str()) {
                            self.pos += 1;
                            out.push_str("[\"n\",");
                            push_string(out, &word);
                            out.push(']');
                        } else {
                            self.ident(out, close);
                        }
                    } else if close == b'}' {
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
                Kind::Str => {
                    if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") && close == b'}' {
                        if let Some(end) = self.method_end(self.pos) {
                            self.pos = end;
                            out.push('0');
                        } else {
                            self.string(out);
                        }
                    } else {
                        self.string(out);
                    }
                }
                Kind::Num if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") && close == b'}' => {
                    if let Some(end) = self.method_end(self.pos) {
                        self.pos = end;
                        out.push('0');
                    } else {
                        self.pos += 1;
                    }
                }
                // `3_usize.saturating_sub(n)`: a numeric literal followed by
                // `.method` starts a receiver chain, just like a string literal.
                Kind::Num if self.op_byte(1) == b'.' && self.word(2).is_some() => {
                    let line = self.toks[self.pos].line;
                    self.pos += 1;
                    self.chain(out, "0".to_string(), String::new(), line);
                }
                Kind::Op => {
                    let byte = self.op_byte(0);
                    // `drain(..label)` / `[..n]`: a prefix range with no start is
                    // one constant to the CST, and its bound names a place rather
                    // than a value the call carries. Only a range that opens an
                    // item is rewritten — `0..n` keeps its shape, where the CST
                    // writes the start and the bound as separate items.
                    let opens_item = self.pos == 0
                        || matches!(self.op_byte_at(self.pos - 1), b'(' | b'[' | b'{' | b',');
                    // A brace group is a struct literal, where `..base` is an
                    // update that names a value the CST keeps; only a call's or a
                    // subscript's prefix range is the constant.
                    if close != b'}' && opens_item && count == 0 && byte == b'.' && self.is_op(1, b".") {
                        out.truncate(at);
                        out.push('0');
                        count += 1;
                        self.pos += 2;
                        if self.is_op(0, b"=") {
                            self.pos += 1;
                        }
                        while let Some(next) = self.tok(0) {
                            if matches!(next.kind, Kind::Ident | Kind::Num) {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                        continue;
                    }
                    if byte == b',' || (close != 0 && byte == close) {
                        out.truncate(at);
                        break;
                    }
                    match byte {
                        // `xs.map(f => …).join('…')`: a dot after a finished
                        // item continues that item's chain. Left to the default
                        // arm the dot was dropped and the member read as a call
                        // with no target, which the CST never writes.
                        b'.' if count > 0 && self.word(1).is_some() && out.ends_with(']') => {
                            let value = out.split_off(body);
                            let line = self.toks[self.pos].line;
                            self.chain(out, value, String::new(), line);
                        }
                        b'(' => {
                            // `(function (g, f) { … })(this, fn)`: a
                            // parenthesised function expression followed by a
                            // parameter list is a call of it, and the CST
                            // writes the call. Left alone the two groups read as
                            // a pair and the value the CST has was lost.
                            let line = self.toks[self.pos].line;
                            let prev_end = out.trim_end_matches(',').len();
                            let prev_start = out[..prev_end].rfind(',').map_or(0, |at| at + 1);
                            // Only a value that ended a block or a group is a
                            // callee here: `…}(args)` and `…)(args)` are
                            // calls, while `if (…)` and a TS cast followed by
                            // `(` are not.
                            // A function expression leaves an empty-span Lambda
                            // token behind it; a block or a group ends with
                            // `}` or `)`. Anything else in that position is
                            // not a callee.
                            let ends_block = self.pos.checked_sub(1).and_then(|at| self.toks.get(at)).is_some_and(|token| {
                                token.kind == Kind::Lambda
                                    || (token.kind == Kind::Op
                                        && matches!(&self.src[token.start as usize..token.end as usize], b"}" | b")"))
                            });
                            let call_of_previous = count > 0 && ends_block && &out[prev_start..prev_end] == "0";
                            if call_of_previous {
                                // `(function (g) { … }(this, fn))`: the call is
                                // written inside the parentheses, so the value
                                // just read is the callee and this parenthesis
                                // is its argument list.
                                self.pos += 1;
                                let mut args = String::new();
                                self.list(&mut args, b')', false);
                                let id = self.new_call(String::new(), line);
                                out.truncate(prev_start);
                                out.push_str(&format!("[\"c\",{id},0,[{args}]]"));
                                continue;
                            }
                            let before = out.len();
                            self.group(out);
                            if matches!(self.lang.tag, "TS" | "TSX") && self.op_byte(0) == b':' {
                                if let Some(arrow) = self.ts_arrow_after_return_type(self.pos + 1) {
                                    self.pos = arrow;
                                    continue;
                                }
                            }
                            if self.op_byte(0) == b'(' && &out[before..] == "0" {
                                self.pos += 1;
                                let mut args = String::new();
                                self.list(&mut args, b')', false);
                                let id = self.new_call(String::new(), line);
                                out.truncate(before);
                                out.push_str(&format!("[\"c\",{id},0,[{args}]]"));
                            }
                        }
                        // `{ *gen() { … } }`: a generator method is one
                        // constant to the CST. Read as a call it was a call the
                        // source never made.
                        b'*' if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                            && close == b'}'
                            && self.toks.get(self.pos + 1).is_some_and(|token| {
                                matches!(token.kind, Kind::Ident | Kind::Str | Kind::Num)
                            })
                            && self.op_byte(2) == b'(' =>
                        {
                            self.pos += 2;
                            let mut params = String::new();
                            self.list(&mut params, b')', false);
                            self.skip_block();
                            out.push('0');
                        }
                        b'[' if close == b'}' && self.display_is_method() => {
                            let key_path = self.computed_key_path();
                            // `{ [Symbol.iterator]() { … } }`: a computed-key
                            // method is one constant to the CST. Read as a
                            // display it became a pair with the method's body
                            // hanging off it.
                            self.display(out, b']', "t");
                            out.truncate(at);
                            if count > 0 {
                                out.push(',');
                            }
                            if matches!(self.lang.tag, "TS" | "TSX")
                                && self.toks.get(self.pos).is_some_and(|t| t.kind == Kind::Op && self.raw_is(t, b"<"))
                            {
                                if let Some(next) = self.skip_balanced(self.pos, b"<", b">") {
                                    self.pos = next;
                                }
                            }
                            if self.op_byte(0) == b'(' {
                                self.pos += 1;
                                let mut params = String::new();
                                self.list(&mut params, b')', false);
                            }
                            if matches!(self.lang.tag, "TS" | "TSX") && self.op_byte(0) == b':' {
                                if let Some(next) = self.skip_return_type(self.pos + 1) {
                                    self.pos = next;
                                }
                            }
                            self.skip_block();
                            // `[Hash.symbol]() { … }` keeps its path: the CST
                            // writes the name when the key is a member
                            // expression, and the constant 0 when it is a
                            // single word.
                            if key_path.contains('.') {
                                out.push_str("[\"n\",");
                                push_string(out, &key_path);
                                out.push(']');
                            } else {
                                out.push('0');
                            }
                        }
                        b'[' => {
                            let mut key = String::new();
                            self.display(&mut key, b']', "t");
                            // `{ [key]: 1 }`: a constant value is written with
                            // its key as one pair, as the CST has it.
                            if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                                && close == b'}'
                                && self.op_byte(0) == b':'
                                && self.toks.get(self.pos + 1).is_some_and(|token| token.kind == Kind::Num)
                            {
                                self.pos += 2;
                                out.push_str("[\"m\",[");
                                out.push_str(&key);
                                out.push_str(",0]]");
                            } else {
                                out.push_str(&key);
                            }
                        }
                        b'{' => self.display(out, b'}', "m"),
                        // `a ? b : c ? b : a`: the whole conditional is one
                        // merge to the CST, and a conditional inside a branch
                        // is a merge inside it. Dropped as layout, the `?` and
                        // `:` left every name in one flat merge.
                        b'?' if count > 0
                            && !self.toks.get(self.pos).is_some_and(|token| {
                                let before = token.start.checked_sub(1).map(|at| self.src[at as usize]);
                                let after = self.src.get(token.end as usize).copied();
                                let nullish = after == Some(b'?')
                                    || before == Some(b'?')
                                    || after == Some(b'.')
                                    || after == Some(b'=');
                                nullish
                            })
                            && matches!(self.lang.tag, "JS" | "JSX")
                            && matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") =>
                        {
                            // The condition is the item just written, not the
                            // `?` itself: lift it (and its separator) out and
                            // let the conditional be the one item.
                            let condition = out[item_start..at].to_string();
                            if condition.is_empty() {
                                // A constant condition writes no item of its
                                // own, so there is nothing to lift: only the
                                // separator this arm had already pushed.
                                out.truncate(at);
                            } else {
                                out.truncate(item_start);
                                count -= 1;
                            }
                            self.pos += 1;
                            let mut consequent = String::new();
                            self.mix(&mut consequent, b':');
                            if self.op_byte(0) == b':' {
                                self.pos += 1;
                            }
                            let mut alternate = String::new();
                            self.mix(&mut alternate, close);
                            out.push_str("[\"m\",[");
                            out.push_str(&condition);
                            out.push(',');
                            out.push_str(if consequent.is_empty() { "0" } else { &consequent });
                            out.push(',');
                            out.push_str(if alternate.is_empty() { "0" } else { &alternate });
                            out.push_str("]]");
                        }
                        b'@' if matches!(self.lang.tag, "RB" | "CR") && self.word(1).is_some() => {
                            let name = self.word(1).unwrap_or_default();
                            let line = self.toks[self.pos].line;
                            self.pos += 2;
                            self.chain(out, String::new(), format!("self.{name}"), line);
                        }
                        // `let resets: Vec<String> = …`: inside an expression
                        // block (e.g. a match arm), the type annotation on a
                        // `let` binding is not a value; the CST skips it.
                        b':' if in_let && self.is_op(0, b":") => {
                            out.truncate(at);
                            in_let = false;
                            self.pos += 1;
                            self.skip_type();
                            continue;
                        }
                        b'=' | b';' if in_let => {
                            in_let = false;
                            self.pos += 1;
                        }
                        // A brace group's `..` is a rest pattern, never a
                        // prefix range: there is no bound to read and nothing
                        // for the value to hold.
                        b'.' if self.lang.tag == "RS"
                            && close == b'}'
                            && self.is_op(0, b".")
                            && self.is_op(1, b".")
                            && !matches!(
                                self.tok(2).map(|token| token.kind),
                                Some(Kind::Ident | Kind::Num)
                            ) =>
                        {
                            self.pos += 2;
                            rest_only = true;
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
                if count == 1 {
                    first_end = out.len();
                }
                item_start = body;
            }
        }
        match count {
            0 if rest_only => {}
            0 => out.push('0'),
            1 => {}
            _ => {
                out.insert_str(start, "[\"m\",[");
                out.push_str("]]");
            }
        }
    }

    /// Whether the `[` at the cursor opens a computed property name that is a
    /// method (`[key]() { … }`) rather than a plain computed key.
    fn display_is_method(&self) -> bool {
        let mut depth = 0usize;
        let mut at = self.pos;
        while let Some(token) = self.toks.get(at) {
            if token.kind == Kind::Op {
                match self.op_byte_at(at) {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            let mut next = at + 1;
                            if matches!(self.lang.tag, "TS" | "TSX")
                                && self.toks.get(next).is_some_and(|t| t.kind == Kind::Op && self.raw_is(t, b"<"))
                            {
                                let Some(after) = self.skip_balanced(next, b"<", b">") else {
                                    return false;
                                };
                                next = after;
                            }
                            return self.toks.get(next).is_some_and(|tok| tok.kind == Kind::Op && self.raw_is(tok, b"("));
                        }
                    }
                    _ => {}
                }
            }
            at += 1;
        }
        false
    }

    /// The path a computed key spells when it is a plain member chain
    /// (`Hash.symbol`), or empty when the key is anything else.
    fn computed_key_path(&self) -> String {
        let mut path = String::new();
        let mut depth = 0usize;
        let mut at = self.pos;
        let mut expect_dot = false;
        while let Some(token) = self.toks.get(at) {
            if token.kind == Kind::Op {
                match self.op_byte_at(at) {
                    b'[' => {
                        depth += 1;
                        if depth > 1 {
                            return String::new();
                        }
                    }
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return path;
                        }
                    }
                    b'.' => expect_dot = true,
                    _ => return String::new(),
                }
            } else if token.kind == Kind::Ident {
                if !path.is_empty() && !expect_dot {
                    return String::new();
                }
                if expect_dot {
                    path.push('.');
                    expect_dot = false;
                }
                path.push_str(&lexeme(self.src, token, self.lang));
            } else {
                return String::new();
            }
            at += 1;
        }
        String::new()
    }

    fn op_byte_at(&self, at: usize) -> u8 {
        self.toks.get(at).filter(|t| t.kind == Kind::Op).map_or(0, |t| self.src[t.start as usize])
    }

    fn raw_is(&self, token: &Token, byte: &[u8]) -> bool {
        &self.src[token.start as usize..token.end as usize] == byte
    }

    /// Skip a `{ … }` block at the cursor.
    fn skip_block(&mut self) {
        if self.op_byte(0) != b'{' {
            return;
        }
        let mut depth = 0usize;
        loop {
            let Some(kind) = self.tok(0).map(|token| token.kind) else { break };
            let byte = self.op_byte(0);
            self.pos += 1;
            if kind == Kind::Op {
                if byte == b'{' {
                    depth += 1;
                } else if byte == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
            }
        }
    }

    fn group(&mut self, out: &mut String) {
        let line = self.toks[self.pos].line;
        // `(*byte)(unsafe.Pointer(&b[0]))`, `(*T)(p)`: a Go conversion writes its
        // type in parentheses, and a type is not a value. Read as a name, it was
        // the only name the statement carried and no CST has it; the value of a
        // conversion is the argument group that follows the type.
        if self.lang.tag == "GO" && self.op_byte(1) == b'*' {
            if let Some(after) = self.skip_balanced(self.pos, b"(", b")") {
                if self.toks.get(after).is_some_and(|token| token.kind == Kind::Op && self.raw_is(token, b"(")) {
                    self.pos = after;
                    self.group(out);
                    return;
                }
            }
        }
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
        let (value, path) = match count {
            0 => (String::new(), String::new()),
            // `(operations[node.type])(node)`: in JS/TS, parenthesizing a bare
            // callee name or subscript does not turn it into an anonymous `.call`.
            1 if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") && self.op_byte(0) == b'(' => {
                match items.strip_prefix("[\"n\",\"").and_then(|rest| rest.strip_suffix("\"]")) {
                    Some(name) if !name.contains('\\') && !name.contains('.') => (String::new(), name.to_string()),
                    _ => (items, String::new()),
                }
            }
            1 => (items, String::new()),
            _ => (format!("[\"t\",[{items}]]"), String::new()),
        };
        self.chain(out, value, path, line);
    }

    fn display(&mut self, out: &mut String, close: u8, form: &str) {
        let line = self.toks[self.pos].line;
        self.pos += 1;
        let mut items = String::new();
        let count = self.list(&mut items, close, false);
        let value = if count == 0 { String::new() } else { format!("[\"{form}\",[{items}]]") };
        self.chain(out, value, String::new(), line);
    }

    fn string_value(&mut self) -> (u32, String) {
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
        (line, value)
    }

    fn string(&mut self, out: &mut String) {
        let (line, value) = self.string_value();
        self.chain(out, value, String::new(), line);
    }

    /// Whether the token `offset` places from the cursor is `++` or `--`.
    fn increment_token(&self, offset: i32) -> bool {
        let at = self.pos as i64 + offset as i64;
        let at = usize::try_from(at).ok();
        let Some(token) = at.and_then(|at| self.toks.get(at)) else { return false };
        if token.kind != Kind::Op {
            return false;
        }
        matches!(&self.src[token.start as usize..token.end as usize], b"++" | b"--")
    }

    /// Skip one type expression written after `as` (or `satisfies`): it runs to
    /// the first token that cannot continue a type at depth zero.
    fn skip_type(&mut self) {
        let mut depth = 0usize;
        while let Some(token) = self.tok(0) {
            let raw = &self.src[token.start as usize..token.end as usize];
            if token.kind == Kind::Op {
                match raw {
                    b"(" | b"[" | b"{" | b"<" => depth += 1,
                    b")" | b"]" | b"}" | b">" => {
                        if depth == 0 {
                            return;
                        }
                        depth -= 1;
                    }
                    b">>" => {
                        if depth >= 2 {
                            depth -= 2;
                        } else {
                            return;
                        }
                    }
                    b";" | b"," | b"=" if depth == 0 => return,
                    _ => {}
                }
            }
            self.pos += 1;
        }
    }

    /// Whether the word at the cursor describes the declaration that follows
    /// (`static`, `extern`, a calling-convention or annotation macro) rather
    /// than naming a value read by the statement.
    fn c_specifier(&self, word: &str) -> bool {
        if C_SPECIFIERS.contains(&word) {
            return true;
        }
        // An annotation macro sits exactly where a type does: `hidden size_t
        // __strftime_l(..)`, `LUAI_FUNC const TValue *luaH_getnum(..)`. No
        // expression writes a name and then a type, so the word before a type
        // is a specifier.
        if self.word(1).is_some_and(|next| C_TYPE_WORDS.contains(&next.as_str())) {
            return true;
        }
        // The same macro in front of a name that is not a keyword: `LUAI_FUNC
        // TString *luaS_newlstr(..)`, `hidden size_t __strftime_l(..)`. A
        // statement cannot open with two words unless the first one declares
        // the type, so the word that opens a statement and is followed by
        // another word is a specifier.
        self.pos == 0 && self.word(1).is_some()
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
        // Scala's `new Foo { ... }` and `new A with B { ... }` build an
        // anonymous class: a constructor call on the first type, not a run of
        // type words and template members. The lexer read the `with` chain and
        // the body as values (`m:[n:A,n:with,...]`), where the CST writes one
        // call with no arguments and records the type as a call.
        if word == "new" && self.lang.tag == "SCALA" {
            if let Some(name) = self.word(1) {
                let line = self.toks[self.pos + 1].line;
                let mut at = 2usize;
                let mut anonymous = false;
                loop {
                    while self.op_byte(at) == b'.' {
                        match self.word(at + 1) {
                            Some(_) => at += 2,
                            None => break,
                        }
                    }
                    if self.word(at).as_deref() == Some("with") && self.word(at + 1).is_some() {
                        anonymous = true;
                        at += 2;
                    } else {
                        break;
                    }
                }
                let mut end = self.pos + at;
                if self.op_byte(at) == b'{' {
                    if let Some(close) = self.skip_balanced(self.pos + at, b"{", b"}") {
                        anonymous = true;
                        end = close;
                    }
                }
                if anonymous {
                    self.pos = end;
                    let id = self.new_call(name, line);
                    out.push_str(&format!("[\"c\",{id},0,[]]"));
                    return;
                }
            }
        }
        // `new ArrayList<>()`, `new HashMap<K, V>()`: Java's diamond and a
        // constructor's type arguments sit between the class name and its
        // argument list. The one-pass reader dropped `new` as noise and then
        // stopped the chain at `<`, so the value was the class name where the
        // CST writes a call (java, the largest cluster in the shape table).
        // C# is the same shape and the same fix makes it *worse* (csharp
        // 14,643 -> 15,031 losing): its CST writes `new List<int>()` as a call
        // with no callee and no argument, and a call the lexer names then has
        // no atom in that value to be covered by, where the merge of names it
        // replaced was excused. That is the C# reader's gap, not this one.
        if word == "new" && matches!(self.lang.tag, "JAVA" | "KT") {
            let mut at = 1usize;
            let mut name = String::new();
            if let Some(first) = self.word(at) {
                name.push_str(&first);
                at += 1;
                while self.op_byte(at) == b'.' {
                    match self.word(at + 1) {
                        Some(segment) => {
                            name.push('.');
                            name.push_str(&segment);
                            at += 2;
                        }
                        None => break,
                    }
                }
            }
            if !name.is_empty() && self.op_byte(at) == b'<' {
                let after = self.skip_balanced(self.pos + at, b"<", b">");
                if let Some(after) = after.filter(|&after| self.op_byte_at(after) == b'(') {
                    // The call is stamped where the class name is, the way the
                    // CST writes it: a type argument list may run over several
                    // lines and its own line is not the statement's.
                    let line = self.toks[self.pos + 1].line;
                    self.pos = after;
                    self.chain(out, String::new(), name, line);
                    return;
                }
            }
        }
        // Scala's `new ConcurrentHashMap[String, X]` with no argument list is a
        // constructor call: the type arguments sit between the class name and
        // the call. The CST writes an anonymous call there, where the lexer read
        // the bare class name, which names the class and not the instance built.
        // Only when the constructor ends its statement: `new ArrayBuffer[T] ++=
        // xs` continues with an operator, and a class body or mixin is not this.
        if word == "new" && self.lang.tag == "SCALA" && self.word(1).is_some() && self.op_byte(2) == b'[' {
            if let Some(after) = self.skip_balanced(self.pos + 2, b"[", b"]") {
                let line = self.toks[self.pos + 1].line;
                let ends = self.toks.get(after).map_or(true, |t| t.line != line)
                    || matches!(self.op_byte_at(after), b';' | b')' | b'}' | b',');
                let mixin = matches!(self.word(after - self.pos).as_deref(), Some("with" | "extends"));
                if ends && !mixin {
                    let name = self.word(1).unwrap_or_default();
                    self.pos = after;
                    let id = self.new_call(name, line);
                    out.push_str(&format!("[\"c\",{id},0,[]]"));
                    return;
                }
            }
        }
        // `new Fab;` with no parentheses is still a constructor call: the
        // grammar writes the call, the lexer dropped `new` as layout and left
        // the callee standing as a name. `new.target` is not a call, and a
        // dotted or parenthesised callee keeps the old path.
        // `<` is the fourth way a constructor has something after its callee:
        // `new Map<keyof Events, Array<Handler<any>>>()` writes type arguments,
        // which are erased at runtime. Read as the no-parenthesis form, the
        // call was emitted with no arguments and the type argument list was
        // read as a value of its own, so the statement carried a merge the CST
        // never writes.
        if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && word == "new"
            && self.op_byte(1) != b'.'
            && self.op_byte(1) != b'('
            && self.word(1).is_some()
            && self.op_byte(2) != b'.'
            && self.op_byte(2) != b'('
            && self.op_byte(2) != b'['
            && self.op_byte(2) != b'<'
        {
            let name = self.word(1).unwrap_or_default();
            let target = self.toks[self.pos + 1].line;
            self.pos += 2;
            // `new Map<keyof Events | '*', Array<Handler<any>>>()`: TypeScript
            // type arguments sit between the callee and its argument list, and
            // read as a value they put the type's words (`keyof`, `Events`) in
            // the record where the CST has one call and nothing else. The
            // group after them is that call's arguments, so it is read here.
            if matches!(self.lang.tag, "TS" | "TSX") && self.is_op(0, b"<") {
                if let Some(after) = self.skip_ts_type_args(self.pos) {
                    // The call sits on the callee's line, the way the CST
                    // writes it: the type list may run over several lines and
                    // the argument list opens on another one, and a call
                    // stamped with the argument's line is a call the CST does
                    // not have.
                    self.pos = after + 1;
                    let id = self.new_call(name.clone(), target);
                    let mut args = String::new();
                    self.list(&mut args, b')', true);
                    let value = format!("[\"c\",{id},0,[{args}]]");
                    self.chain(out, value, String::new(), target);
                    return;
                }
            }
            let id = self.new_call(name, target);
            out.push_str(&format!("[\"c\",{id},0,[]]"));
            return;
        }
        // `f(function (st) { ... })`: a function expression is a value, and the
        // CST records it as one constant. Read as a name, `function` became a
        // call whose arguments were its parameter list, and the statement was
        // blamed for a call the source never made.
        if word == "function" && self.lang.defs.contains(&word.as_str()) {
            self.pos += 1;
            if self.op_byte(0) == b'*' {
                self.pos += 1;
            }
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
            } else if self.tok(0).is_some_and(|t| t.kind == Kind::Lambda) {
                self.pos += 1;
            }
            self.chain(out, "0".to_string(), String::new(), line);
            return;
        }
        // `{} as Record<string, number>`: a type assertion's type is not a
        // value. `as` is already noise, but the type that followed it was read
        // like an expression, so the lexer's value carried the type arguments
        // while the CST writes only the value being cast.
        if matches!(self.lang.tag, "TS" | "TSX") && word == "as" {
            self.pos += 1;
            self.skip_type();
            return;
        }
        // `b'A'` / `b"…"` (and raw `br"…"`): a byte literal is one constant.
        // The lexer read the prefix as a name and merged it with the literal,
        // so the value was `m:[n:b,0]` where the CST writes the constant 0.
        if self.lang.tag == "RS" && matches!(word.as_str(), "b" | "br") {
            if let (Some(current), Some(next)) = (self.tok(0), self.tok(1)) {
                if next.kind == Kind::Str && current.end == next.start {
                    self.pos += 1;
                    return;
                }
            }
        }
        // `pub(crate) meter: T`: the visibility is not part of the field. `pub`
        // is already noise, but the parenthesised group survived it and its
        // constant became a member of the merge - the CST writes neither.
        if self.lang.tag == "RS" && word == "pub" && self.is_op(1, b"(") {
            self.pos += 1;
            let mut depth = 0usize;
            loop {
                let Some(token) = self.tok(0) else { break };
                let (kind, start, end) = (token.kind, token.start as usize, token.end as usize);
                self.pos += 1;
                if kind == Kind::Op {
                    match &self.src[start..end] {
                        b"(" => depth += 1,
                        b")" => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            return;
        }
        // `override fun visitMethod(...): MethodVisitor = ...`, `fun(x: Int) = x`:
        // in Kotlin `fun` introduces a function declaration or anonymous
        // function, never a call. Left to `chain()`, the declared name was
        // read as a callee (`call visitMethod` inside `object : ... { ... }`).
        // Checked before `noise` because `"fun"` is in `NOISE`.
        if self.lang.tag == "KT" && word == "fun" {
            self.pos += 1;
            self.skip_kt_fun_header();
            return;
        }
        // Swift's `defer { … }` is a statement keyword, never a name: the CST
        // writes the block's calls and nothing for the word itself.
        let go_word = match word.as_str() {
            "go" => self.lang.tag != "GO",
            "defer" => !matches!(self.lang.tag, "GO" | "SWIFT"),
            _ => false,
        };
        // `new` is a keyword in the JavaScript family and an ordinary name in
        // Rust: reading it as noise left `new.is_none()` calling nothing, and
        // the record differed by a constant that was never there.
        let js_family = matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX");
        let noise = if js_family {
            // `async(callback)(null, x)`: `async` is an ordinary name being
            // called, and reading it as noise left the call with no callee
            // (the first argument was recorded instead). Only an arrow head
            // (`async (x) => …`) is a keyword that names nothing.
            JS_NOISE.contains(&word.as_str())
                && !(word == "async" && self.op_byte(1) == b'(' && !self.async_arrow_head(1))
        } else if self.lang.tag == "PY" {
            PY_NOISE.contains(&word.as_str())
        } else if self.lang.tag == "KT" {
            (NOISE.contains(&word.as_str()) && !KT_NAMES.contains(&word.as_str()))
                || KT_NOISE.contains(&word.as_str())
        } else if self.lang.tag == "RS" {
            // `async move { … }` / `move ||`: `move` names a closure, not a
            // binding; the CST writes no value for it.
            RS_NOISE.contains(&word.as_str())
        } else {
            // Swift's `try` marks a throwing call and names no value: the CST
            // writes `try build()` as the call alone, so the word is noise here.
            NOISE.contains(&word.as_str()) || (self.lang.tag == "SWIFT" && word == "try")
        };
        if noise && !go_word {
            self.pos += 1;
            return;
        }
        // Swift's `guard` is a keyword that opens a condition, never a name: the
        // CST writes the condition's values and nothing for the word itself, so
        // `guard a <= b else` must not file `n:guard` with them.
        if self.lang.tag == "SWIFT" && word == "guard" {
            self.pos += 1;
            return;
        }
        // `private nonisolated(unsafe) var value`: `nonisolated` is a Swift
        // declaration modifier and `(unsafe)` its argument, so neither is a
        // call. The CST still writes a call here, a separate grammar gap.
        if self.lang.tag == "SWIFT" && word == "nonisolated" && self.op_byte(1) == b'(' && self.word(2).is_some() && self.op_byte(3) == b')' {
            self.pos += 4;
            return;
        }
        // A C declaration's specifiers are not values. `static int h(int x);`
        // read `static` as a name, `LUAI_FUNC const TValue *luaH_getnum(..)`
        // read the annotation macro, and `extern hidden void j(int);` read
        // both; the CST writes the type and the declared name and nothing else,
        // so each was a name the source never had as a value (c corpus).
        if matches!(self.lang.tag, "C" | "CPP") && self.c_specifier(&word) {
            self.pos += 1;
            return;
        }
        self.pos += 1;
        // `strings.FieldsFunc(line, func(r rune) bool { … })`: in Go `func`
        // introduces a function literal or type, never a call. Left to
        // `chain()` it read the parameter list as arguments to a callee named
        // `func`, adding a call atom the CST never writes.
        if self.lang.tag == "GO" && word == "func" && self.op_byte(0) == b'(' {
            return;
        }
        // `typealias H = suspend (Int) -> Unit`, `val w: suspend C.() -> Unit`:
        // a suspend function *type*. The group is its parameter list and the
        // type after `->` is its result, so the construct names no value at all;
        // read as a call it invented a callee called `suspend` in every file
        // that wrote one.
        if self.lang.tag == "KT" && word == "suspend" && self.op_byte(0) == b'(' {
            self.pos += 1;
            let mut params = String::new();
            self.list(&mut params, b')', false);
            if self.is_op(0, b"->") {
                self.pos += 1;
                while let Some(token) = self.tok(0) {
                    let op = if token.kind == Kind::Op { self.src[token.start as usize..token.end as usize].to_vec() } else { Vec::new() };
                    let is_type = token.kind == Kind::Ident || matches!(op.as_slice(), b"?" | b"." | b"::" | b"<");
                    if !is_type {
                        break;
                    }
                    self.pos += 1;
                }
            }
            return;
        }
        // `pick: fn(&Corners) -> f64`: a function *type*. `fn` is a keyword that
        // names no value, its parentheses are a parameter list, and the type
        // after `->` is a type. Read as a value chain the two words `fn` and
        // `f64` reached the record as a name, and the CST — which writes the
        // parameter's type name and nothing else — was blamed for them.
        if self.lang.tag == "RS" && word == "fn" && self.op_byte(0) == b'(' {
            self.pos += 1;
            let mut params = String::new();
            self.list(&mut params, b')', false);
            if self.is_op(0, b"->") {
                self.pos += 2;
                let mut depth = 0usize;
                while let Some(token) = self.tok(0) {
                    if token.kind == Kind::Op {
                        let raw = &self.src[token.start as usize..token.end as usize];
                        match raw {
                            b"(" | b"[" | b"<" => depth += 1,
                            b">" => {
                                if depth == 0 {
                                    break;
                                }
                                depth -= 1;
                            }
                            b"{" | b")" | b"]" | b"}" | b"," | b";" | b"=" | b"|" if depth == 0 => break,
                            _ => {}
                        }
                    }
                    self.pos += 1;
                }
            }
            return;
        }
        // `await import('./x.js')`: a dynamic import is a module load, not a
        // call. The CST records no call for it, and the lexer's `import` call
        // lost 12 js files; the value it produces is its argument group.
        if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") && word == "import" && self.op_byte(0) == b'(' {
            let mut value = String::new();
            self.group(&mut value);
            out.push_str(&value);
            return;
        }
        if self.lang.arg_words.contains(&word.as_str()) {
            out.push_str("[\"n\",\"_\"]");
            return;
        }
        // Swift's `true`, `false` and `nil` are literals, not names: the CST
        // writes each as the constant 0, so the one-pass reader must not file
        // `n:true` for `isFixedLengthCollection: true`, or `n:nil` for `(nil)`.
        if self.lang.tag == "SWIFT" && matches!(word.as_str(), "true" | "false" | "nil") && self.op_byte(0) != b'.' {
            out.push('0');
            return;
        }
        // `++i` and `i++`: an increment is not a read of the variable, and the
        // CST writes the whole update expression as a constant. Reading the
        // name here made the file lose a value the source never produced.
        // `++this.counter` increments a member, not a name: the CST reads the
        // path, so only a bare operand is written as the constant.
        if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX" | "KT")
            && self.op_byte(0) != b'.'
            && (self.increment_token(-2) || self.increment_token(0))
        {
            out.push('0');
            return;
        }
        // Swift's `@convention(c)` and `@convention(block)` are type attributes:
        // the parentheses name the convention, not a call. The CST writes none
        // (`_type_attribute: "@" _name`), so the one-pass reader must not file
        // a `convention` call for every such typealias.
        if self.lang.tag == "SWIFT"
            && word == "convention"
            && self.op_byte(0) == b'('
            && self.pos >= 2
            && self.raw_is(&self.toks[self.pos - 2], b"@")
        {
            if let Some(after) = self.skip_balanced(self.pos, b"(", b")") {
                // The function type's parameter list follows the attribute:
                // `(UnsafeMutablePointer<T>) -> Int`. It is a type, not the
                // arguments of a call on the attribute's value.
                self.pos = after;
                if self.op_byte(0) == b'(' {
                    if let Some(params) = self.skip_balanced(self.pos, b"(", b")") {
                        self.pos = params;
                    }
                }
                out.push('0');
                return;
            }
        }
        let head = if word == "this" || word == "self" || (!self.receiver.is_empty() && word == self.receiver) {
            "self".to_string()
        } else {
            word
        };
        // `returns() implies (condition)`, `a shl (b)`: in Kotlin an infix
        // function whose right operand is parenthesised is an infix expression,
        // not a call to `implies(...)` or `shl(...)`. Leave the `(` for
        // `group()` so both readers record the name and the operand without a
        // spurious call.
        if self.lang.tag == "KT" && self.op_byte(0) == b'(' && self.kt_infix_before(self.pos - 1) {
            out.push_str("[\"n\",");
            push_string(out, &head);
            out.push(']');
            return;
        }
        self.chain(out, String::new(), head, line);
        if self.lang.command_calls || self.lang.apply_calls {
            self.juxtaposed(out, close, line);
        }
    }

    /// Skip a Kotlin `<...>` type-argument or type-parameter list at `self.pos`.
    fn skip_kt_angles(&mut self) {
        if !self.is_op(0, b"<") && !self.is_op(0, b"<<") {
            return;
        }
        let mut depth = 0i32;
        while let Some(token) = self.tok(0) {
            if token.kind == Kind::Op {
                match &self.src[token.start as usize..token.end as usize] {
                    b"<" => depth += 1,
                    b"<<" => depth += 2,
                    b">" => depth -= 1,
                    b">>" => depth -= 2,
                    _ => {}
                }
            }
            self.pos += 1;
            if depth <= 0 {
                break;
            }
        }
    }

    /// Skip a Kotlin `fun` header (type params, receiver/name, parameter list,
    /// and optional `: ReturnType`) so `object : ... { override fun f() = ... }`
    /// does not emit a call for `f` while still walking its body.
    fn skip_kt_fun_header(&mut self) {
        self.skip_kt_angles();
        while let Some(token) = self.tok(0) {
            if token.kind == Kind::Ident || self.is_op(0, b".") || self.is_op(0, b"?") {
                self.pos += 1;
                self.skip_kt_angles();
            } else {
                break;
            }
        }
        if self.op_byte(0) == b'(' {
            if let Some(after) = self.skip_balanced(self.pos, b"(", b")") {
                self.pos = after;
            }
        }
        if self.is_op(0, b":") {
            self.pos += 1;
            let mut depth = 0i32;
            while let Some(token) = self.tok(0) {
                if token.kind == Kind::Op {
                    match &self.src[token.start as usize..token.end as usize] {
                        b"(" | b"[" | b"<" => depth += 1,
                        b"<<" => depth += 2,
                        b")" | b"]" if depth > 0 => depth -= 1,
                        b">" if depth > 0 => depth -= 1,
                        b">>" if depth > 0 => depth -= 2,
                        b"{" | b"=" | b")" | b"]" | b"}" | b"," | b";" if depth <= 0 => break,
                        _ => {}
                    }
                }
                self.pos += 1;
            }
        }
    }

    /// Whether the Kotlin identifier at `ident_pos` sits immediately after a
    /// same-line expression, making `ident (...)` an infix call rather than a
    /// parenthesised function call.
    fn kt_infix_before(&self, ident_pos: usize) -> bool {
        let Some(prev_pos) = ident_pos.checked_sub(1) else {
            return false;
        };
        self.toks[prev_pos].line == self.toks[ident_pos].line && self.kt_is_expr_end(prev_pos)
    }

    /// Whether the token at `pos` ends a Kotlin expression (and is not itself
    /// an infix operator or keyword).
    fn kt_is_expr_end(&self, pos: usize) -> bool {
        let tok = &self.toks[pos];
        match tok.kind {
            Kind::Num | Kind::Str => true,
            Kind::Ident => {
                let word = lexeme(self.src, tok, self.lang);
                if matches!(
                    word.as_str(),
                    "return"
                        | "throw"
                        | "else"
                        | "in"
                        | "is"
                        | "as"
                        | "do"
                        | "by"
                        | "where"
                        | "if"
                        | "when"
                        | "try"
                        | "catch"
                        | "finally"
                        | "for"
                        | "while"
                        | "fun"
                        | "val"
                        | "var"
                        | "object"
                        | "class"
                        | "interface"
                        | "typealias"
                        | "import"
                        | "package"
                ) {
                    return false;
                }
                // Two adjacent identifiers on the same line alternate:
                // `expr infix_op expr ...`. If `pos - 1` ended an expression on
                // the same line, then `pos` is an infix operator, not an
                // expression end.
                !pos
                    .checked_sub(1)
                    .is_some_and(|prev| self.toks[prev].line == tok.line && self.kt_is_expr_end(prev))
            }
            Kind::Op => {
                let raw = &self.src[tok.start as usize..tok.end as usize];
                if matches!(raw, b"]" | b"!!") {
                    return true;
                }
                if raw != b")" {
                    return false;
                }
                let mut depth = 0usize;
                let mut j = pos;
                loop {
                    let t = &self.toks[j];
                    if t.kind == Kind::Op {
                        let r = &self.src[t.start as usize..t.end as usize];
                        if r == b")" {
                            depth += 1;
                        } else if r == b"(" {
                            depth -= 1;
                            if depth == 0 {
                                if let Some(before) = j.checked_sub(1) {
                                    let bt = &self.toks[before];
                                    if bt.kind == Kind::Ident {
                                        let bw = lexeme(self.src, bt, self.lang);
                                        if matches!(bw.as_str(), "if" | "for" | "while" | "catch" | "when") {
                                            return false;
                                        }
                                        if before > 0 && self.raw_is(&self.toks[before - 1], b"@") {
                                            return false;
                                        }
                                    }
                                }
                                return true;
                            }
                        }
                    }
                    if j == 0 {
                        return false;
                    }
                    j -= 1;
                }
            }
            _ => false,
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
        let noise_list: &[&str] = if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") {
            &JS_NOISE
        } else if self.lang.tag == "PY" {
            &PY_NOISE
        } else {
            &NOISE
        };
        if next.kind == Kind::Ident && self.word(0).is_some_and(|word| noise_list.contains(&word.as_str())) {
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
    /// Whether the `(` at the cursor closes a Rust type rather than a call:
    /// `dyn Fn(..)`, `: Fn(..)`, `impl Fn(..)`, `fn(..)`.
    fn rust_type_group(&self) -> bool {
        let Some(token) = self.pos.checked_sub(2).and_then(|at| self.toks.get(at)) else {
            return false;
        };
        // Only a word that can find nothing else: `:` and `<` also precede a
        // real call (`f: g(x)`, `a < b(1)`), and reading those as types broke
        // 133 corpus files.
        if self.raw_is(token, b"fn") {
            return true;
        }
        matches!(lexeme(self.src, token, self.lang).as_str(), "dyn" | "impl")
    }

    /// If `start` opens a TypeScript type-argument list (`<T, U>`) followed by
    /// `(`, return the token index of that `(`.
    fn skip_ts_type_args(&self, start: usize) -> Option<usize> {
        if !self.toks.get(start).is_some_and(|t| t.kind == Kind::Op && self.raw_is(t, b"<")) {
            return None;
        }
        let mut angle = 1usize;
        let mut paren = 0usize;
        let mut bracket = 0usize;
        let mut brace = 0usize;
        let mut index = start + 1;
        while let Some(token) = self.toks.get(index) {
            if token.kind == Kind::Op {
                let raw = &self.src[token.start as usize..token.end as usize];
                match raw {
                    b"(" => paren += 1,
                    b")" => paren = paren.checked_sub(1)?,
                    b"[" => bracket += 1,
                    b"]" => bracket = bracket.checked_sub(1)?,
                    b"{" => brace += 1,
                    b"}" => brace = brace.checked_sub(1)?,
                    b"<" if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
                    b">" | b">>" | b">>>" if paren == 0 && bracket == 0 && brace == 0 => {
                        angle = angle.checked_sub(raw.len())?;
                        if angle == 0 {
                            return self
                                .toks
                                .get(index + 1)
                                .is_some_and(|t| t.kind == Kind::Op && self.raw_is(t, b"("))
                                .then_some(index + 1);
                        }
                    }
                    b";" | b"&&" | b"||" | b"??" | b"==" | b"===" | b"!=" | b"!=="
                    | b"<=" | b">=" | b"+=" | b"-=" | b"*=" | b"/=" | b"%=" | b"="
                        if paren == 0 && bracket == 0 && brace == 0 =>
                    {
                        return None;
                    }
                    _ => {}
                }
            }
            index += 1;
        }
        None
    }

    /// The offset just past a Kotlin annotation at the cursor: `@Name`,
    /// `@get:Name`, `@a.b.Name("arg")`, or a bracketed list
    /// `@[Suppress("x") OptIn(Y::class)]`.
    fn kt_annotation_end(&self) -> Option<usize> {
        let text = |token: &Token| -> String {
            String::from_utf8_lossy(&self.src[token.start as usize..token.end as usize]).into_owned()
        };
        let mut index = self.pos + 1;
        let first = self.toks.get(index)?;
        if first.kind == Kind::Op && text(first) == "[" {
            let mut depth = 0usize;
            while let Some(token) = self.toks.get(index) {
                if token.kind == Kind::Op {
                    match text(token).as_str() {
                        "[" => depth += 1,
                        "]" => {
                            depth -= 1;
                            index += 1;
                            if depth == 0 {
                                return Some(index);
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                index += 1;
            }
            return None;
        }
        let mut named = false;
        while let Some(token) = self.toks.get(index) {
            if token.kind != Kind::Ident {
                break;
            }
            named = true;
            index += 1;
            match self.toks.get(index) {
                Some(op) if op.kind == Kind::Op && matches!(text(op).as_str(), "." | ":") => {
                    index += 1
                }
                _ => break,
            }
        }
        if !named {
            return None;
        }
        if self.toks.get(index).is_some_and(|token| token.kind == Kind::Op && text(token) == "(") {
            let mut depth = 0usize;
            while let Some(token) = self.toks.get(index) {
                if token.kind == Kind::Op {
                    match text(token).as_str() {
                        "(" => depth += 1,
                        ")" => depth -= 1,
                        _ => {}
                    }
                }
                index += 1;
                if depth == 0 {
                    break;
                }
            }
        }
        Some(index)
    }

    fn chain(&mut self, out: &mut String, mut value: String, mut path: String, line: u32) {
        let mut constructed = None;
        let mut type_arg_line = None;
        let mut generic_callee = false;
        loop {
            // `-> @Suppress("UNCHECKED_CAST") (ReverseOrderComparator as
            // Comparator<T>)`: in Kotlin an annotation decorates an expression
            // as well as a declaration. Read as a value its name became a call
            // the CST never writes (`call Suppress line 302 in reversed`,
            // stdlib Comparisons.kt); the annotated expression is the value and
            // the annotation names nothing. Only in front of a value: a label
            // (`return@f`) follows a word, not an operator.
            if self.lang.tag == "KT"
                && self.is_op(0, b"@")
                && (self.pos == 0 || self.toks[self.pos - 1].kind == Kind::Op)
            {
                if let Some(after) = self.kt_annotation_end() {
                    self.pos = after;
                    continue;
                }
            }
            // `.collect::<Vec<_>>()`: a turbofish names a type, not a value.
            // Reading its words as names put `Vec` and `_` in the record where
            // the CST has a call and nothing else.
            if self.is_op(0, b"::") && (self.is_op(1, b"<") || self.is_op(1, b"<<")) {
                let opened = self.toks[self.pos].line;
                self.pos += 1;
                // `None::<Vec<Entry<T>>>`: the tokenizer reads `>>` as one
                // operator, and here it closes two lists (`<<` opens two, in
                // `f::<<T as Tr>::Out>`). Counting only a lone `>` never closed
                // such a turbofish, so the skip ran to the end of the statement
                // and took every later argument and call with it.
                let mut depth = 0i32;
                while let Some(token) = self.tok(0) {
                    match &self.src[token.start as usize..token.end as usize] {
                        b"<" => depth += 1,
                        b"<<" => depth += 2,
                        b">" => depth -= 1,
                        b">>" => depth -= 2,
                        _ => {}
                    }
                    self.pos += 1;
                    if depth <= 0 {
                        break;
                    }
                }
                // `transmute::<` newline `A,` newline `>(x)`: the call is
                // written where its name is, which is the line the CST stamps;
                // the parenthesis can be any number of lines further down.
                if self.op_byte(0) == b'(' {
                    type_arg_line = Some(opened);
                }
                continue;
            }
            // `x!.length`: the non-null assertion is a type marker between the
            // value and its member, and the CST keeps the chain whole.
            if matches!(self.lang.tag, "TS" | "TSX") && self.is_op(0, b"!") {
                self.pos += 1;
                continue;
            }
            // `remainingPath!.count`: Swift's postfix force-unwrap is the same
            // kind of marker, and the CST names the chain `remainingPath.count`.
            // Ending the value at `!` split it into two merged names.
            if self.lang.tag == "SWIFT" && self.is_op(0, b"!") && self.is_op(1, b".") {
                self.pos += 1;
                continue;
            }
            // `z.string().brand<"username">()`: TypeScript/Kotlin type arguments before
            // `(` are types, not `< ... >` comparisons or runtime constants.
            if matches!(self.lang.tag, "TS" | "TSX" | "KT" | "SWIFT") && self.is_op(0, b"<") {
                if let Some(after) = self.skip_ts_type_args(self.pos) {
                    type_arg_line = Some(self.toks[self.pos].line);
                    generic_callee = self.lang.tag == "SWIFT";
                    self.pos = after;
                    continue;
                }
            }
            // `this@asList.isEmpty()`: a labelled receiver names the enclosing
            // scope, not a member. Left in the chain the label became part of
            // the callee (`asList.isEmpty`) where the CST names `self.isEmpty`.
            if self.lang.tag == "KT"
                && matches!(path.as_str(), "self" | "super")
                && self.is_op(0, b"@")
                && self.word(1).is_some()
            {
                self.pos += 2;
                continue;
            }
            // `client.borrow()?` newline `.core_client`: in Rust `?` is a
            // postfix operator between the call and its member, and the CST
            // keeps the chain whole. Ending the value there split one chain into
            // two merged items (46 files).
            if self.lang.tag == "RS" && self.is_op(0, b"?") {
                self.pos += 1;
                continue;
            }
            // `start.await.status`: `.await` is the other postfix operator, and
            // it is no more a member than `?` is. Kept as a segment the lexer
            // named `start.await.status` where the CST, which reads through an
            // `await_expression`, names `start.status`.
            if self.lang.tag == "RS" && self.is_op(0, b".") && self.word(1).as_deref() == Some("await") {
                self.pos += 2;
                continue;
            }
            let separator = self.is_op(0, b".")
                || (self.is_op(0, b"->") && self.lang.tag != "KT")
                || self.is_op(0, b"::")
                || self.is_op(0, b"?.")
                || (self.is_op(0, b":") && self.op_byte(2) == b'(' && self.lang.tag == "LUA");
            if separator {
                // `a?.()`: the optional chain is a call with no member name
                // between the separator and the parenthesis. Leaving it to be
                // rescanned as an operator made the lexer read the whole thing
                // as the name `a`, so the call the CST records was lost.
                if self.is_op(0, b"?.") && self.op_byte(1) == b'(' {
                    self.pos += 1;
                    continue;
                }
                // `a?.[1]`: the CST keeps the name and drops the optional
                // subscript entirely (`a?.[i].x` is `a.x`, with no `i`), while
                // the lexer read the bracket as a display and the name as a
                // value the source never had.
                if self.is_op(0, b"?.") && self.op_byte(1) == b'[' {
                    self.pos += 2;
                    let mut index = String::new();
                    self.list(&mut index, b']', false);
                    continue;
                }
                // `want.1.targets`: a tuple field's name is a number, and the
                // chain keeps it — the CST writes `want.1.targets` while the
                // lexer ended the name at `want` and left `1` as a constant of
                // its own (ten corpus files and this repository's own guard).
                if self.is_op(0, b".")
                    && self.tok(1).is_some_and(|token| token.kind == Kind::Num)
                    && (self.is_op(2, b".") || self.is_op(2, b"::"))
                {
                    let token = &self.toks[self.pos + 1];
                    let number = &self.src[token.start as usize..token.end as usize];
                    self.pos += 2;
                    path.push('.');
                    path.push_str(std::str::from_utf8(number).unwrap_or("0"));
                    continue;
                }
                if let Some(name) = self.word(1) {
                    self.pos += 2;
                    path.push('.');
                    // The CST names a private member without its hash; the
                    // hash is how the source spells the name, not part of it.
                    path.push_str(name.trim_start_matches('#'));
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
                // `f()()`, `Function(source)()`: the CST records the inner call
                // and nothing else — the outer call, its target and its
                // arguments never reach the record.
                if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                    && path.is_empty()
                    && value.starts_with("[\"c\",")
                {
                    self.pos += 1;
                    let mut args = String::new();
                    self.list(&mut args, b')', true);
                    continue;
                }
                // `dyn Fn(i32) -> Fut`, `where F: Fn(..) -> Self`, `Arc<dyn
                // Fn(..)>`: in Rust a group after a type context is a function
                // *type*. Read as a call it invented a callee called `Fn` on
                // every trait bound in the corpus.
                if self.lang.tag == "RS" && self.rust_type_group() {
                    self.pos += 1;
                    let mut args = String::new();
                    self.list(&mut args, b')', true);
                    continue;
                }
                // `Other(#[from] AnyhowError)`: a group whose first token is an
                // attribute declares the fields of an enum variant, it does not
                // call anything. Read as a call it invented a callee named after
                // the variant (7 corpus files).
                if self.lang.tag == "RS" && self.op_byte(1) == b'#' && self.op_byte(2) == b'[' {
                    self.pos += 1;
                    let mut args = String::new();
                    self.list(&mut args, b')', true);
                    continue;
                }
                // `Other(#[from] AnyhowError)`: a group whose first token is an
                // attribute declares the fields of an enum variant, it does not
                // call anything. Read as a call it invented a callee named after
                // the variant (7 corpus files).
                if self.lang.tag == "RS" && self.op_byte(1) == b'#' && self.op_byte(2) == b'[' {
                    self.pos += 1;
                    let mut args = String::new();
                    self.list(&mut args, b')', true);
                    continue;
                }
                let line = type_arg_line.take().unwrap_or(self.toks[self.pos].line);
                self.pos += 1;
                // `CFUNCTYPE(c_void_p, c_size_t)(addr)`: the value a
                // constructor built is applied, and an application has no
                // name, exactly like `f(x)(y)`. Glued to the class it was
                // `CFUNCTYPE().call` - a method no source spells, which the
                // CST never writes and the resolver would tie to any method
                // that happens to be named `call`.
                let applied = path.is_empty();
                let (receiver, mut target) = self.split_receiver(&value, &path);
                // `Foo<Int8>(x)`: the CST writes the type as the callee and an anonymous call,
                // so a bare name that carried type arguments names no call.
                if generic_callee && value.is_empty() && !path.is_empty() && !path.contains('.') {
                    target.clear();
                }
                if target.starts_with('.') {
                    if let Some(class) = constructed.take().filter(|_| !applied) {
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
            // `tag`...``: a tagged template literal in the JS/TS family is a
            // call to the tag with the template's interpolations as arguments.
            if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                && self.tok(0).is_some_and(|t| t.kind == Kind::Str && self.src[t.start as usize] == b'`')
            {
                let (t_line, inner) = self.string_value();
                let applied = path.is_empty();
                let (receiver, mut target) = self.split_receiver(&value, &path);
                if target.starts_with('.') {
                    if let Some(class) = constructed.take().filter(|_| !applied) {
                        target = format!("{class}(){target}");
                    }
                }
                constructed = capitalised(&target);
                let id = self.new_call(target, t_line);
                let args = if inner == "0" { String::new() } else { inner };
                value = format!("[\"c\",{id},{receiver},[{args}]]");
                path.clear();
                continue;
            }
            // `ServletHandler().apply { ... }`: in Kotlin, a trailing `{ ... }`
            // after a chained member call is a lambda argument of that call,
            // not a separate map/block literal merged beside the receiver.
            if self.lang.tag == "KT"
                && self.op_byte(0) == b'{'
                && !value.is_empty()
                && path.starts_with('.')
            {
                let line = type_arg_line.take().unwrap_or(self.toks[self.pos].line);
                let (receiver, mut target) = self.split_receiver(&value, &path);
                if target.starts_with('.') {
                    if let Some(class) = constructed.take() {
                        target = format!("{class}(){target}");
                    }
                }
                constructed = capitalised(&target);
                let id = self.new_call(target, line);
                let mut args = String::new();
                self.display(&mut args, b'}', "m");
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
    fn a_new_without_parentheses_is_still_a_call() {
        // `var fab = new Fab;` invokes a constructor with no arguments. The
        // grammar writes the call and the lexer dropped `new` as layout, so the
        // callee stood as a name the CST never wrote — 32 losses in the corpus.
        let (value, calls) = form("js", "var fab = new Fab;");
        assert!(value.contains(r#"["c""#), "`new Fab` is a constructor call: {value}");
        assert_eq!(calls, vec!["Fab".to_string()]);
        // `new.target` is a property, not a call, and a parenthesised callee is
        // read by the normal call path.
        assert_eq!(form("js", "new.target").1.len(), 0);
        assert_eq!(form("js", "new Foo()").1, vec!["Foo".to_string()]);
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


    #[test]
    fn an_object_method_is_a_constant() {
        // `{ set(v) { … } }`: the CST writes the method as one value, and the
        // lexer read the body into the object's merge.
        assert_eq!(form("js", "{ set(v) { v; } }").0, r#"["m",[0]]"#);
        assert_eq!(form("js", "{ get name() { return a; } }").0, r#"["m",[0]]"#);
        assert_eq!(form("js", r#"{ "a"() { return a; } }"#).0, r#"["m",[0]]"#);
        assert_eq!(form("js", r#"{ get "a"() { return a; } }"#).0, r#"["m",[0]]"#);
        assert_eq!(form("js", r#"{ *"a"() { return a; } }"#).0, r#"["m",[0]]"#);
        assert_eq!(form("js", "{ 1() { return a; } }").0, r#"["m",[0]]"#);
        assert_eq!(form("ts", "{ m<T extends R<A>>(x: T): R<A<B>, C<D>> { return g(x); } }").0, r#"["m",[0]]"#);
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
        let (value, calls) = form("js", "f(function () { this.sign(); }.bind(this))");
        assert_eq!(value, r#"["c",0,0,[["c",1,0,[["n","self"]]]]]"#);
        assert_eq!(calls, ["f", ".bind"]);
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

    #[test]
    fn ts_call_type_arguments_are_skipped() {
        let (out, calls) = form("ts", r#"z.string().brand<"username">()"#);
        assert_eq!(calls, ["z.string", ".brand"]);
        assert_eq!(out, r#"["c",1,["c",0,["n","z"],[]],[]]"#);
        let (out, calls) = form("ts", "foo<Array<number>>(x)");
        assert_eq!(calls, ["foo"]);
        assert_eq!(out, r#"["c",0,0,[["n","x"]]]"#);
        let (out, calls) = form("ts", "a < b && c > (d)");
        assert!(calls.is_empty());
        assert_eq!(out, r#"["m",[["n","a"],["n","b"],["n","c"],["n","d"]]]"#);
    }

    #[test]
    fn a_turbofish_closes_where_its_angle_brackets_do() {
        // `>>` is one token to the tokenizer and two closers to a turbofish.
        // The skip used to run past it to the end of the statement, so the
        // arguments after `None::<Vec<Entry<T>>>` left the record.
        let (out, calls) = form("rs", "f(a, None::<Vec<Entry<T>>>, g(b))");
        assert_eq!(calls, ["f", "g"]);
        assert_eq!(out, r#"["c",0,0,[["n","a"],["n","None"],["c",1,0,[["n","b"]]]]]"#);
        let (out, calls) = form("rs", "x.collect::<Vec<Vec<u8>>>().len()");
        assert_eq!(calls, ["x.collect", ".len"]);
        assert_eq!(out, r#"["c",1,["c",0,["n","x"],[]],[]]"#);
        // `<<` opens two lists the same way, and the call behind them is read.
        let (out, calls) = form("rs", "size_of::<<T as Tr>::Out>() + n");
        assert_eq!(calls, ["size_of"]);
        assert_eq!(out, r#"["m",[["c",0,0,[]],["n","n"]]]"#);
        // A lone `>` still closes one list, and the path goes on behind it.
        assert_eq!(form("rs", "Option::<T>::None").0, r#"["n","Option.None"]"#);
    }

    #[test]
    fn js_parenthesized_callee_keeps_its_name() {
        let (out, calls) = form("js", "(operations[node.type])(node, initialScope)");
        assert_eq!(calls, ["operations"]);
        assert_eq!(out, r#"["c",0,0,[["n","node"],["n","initialScope"]]]"#);
    }

    #[test]
    fn applying_a_constructed_value_is_an_anonymous_call() {
        // `CFUNCTYPE(c_void_p)(addr)` applies the value the constructor built.
        // No method is named, so the call is `.call` like `f(x)(y)`; a member
        // called on the same value still carries the class that built it.
        let (out, calls) = form("py", "CFUNCTYPE(c_void_p, c_size_t)(addr)");
        assert_eq!(calls, ["CFUNCTYPE", ".call"]);
        assert_eq!(out, r#"["c",1,["c",0,0,[["n","c_void_p"],["n","c_size_t"]]],[["n","addr"]]]"#);
        assert_eq!(form("py", "f(x)(y)").1, ["f", ".call"]);
        assert_eq!(form("py", "Repo(db).find(key)").1, ["Repo", "Repo().find"]);
        assert_eq!(form("py", "Repo(db).call(key)").1, ["Repo", "Repo().call"]);
        assert_eq!(form("py", "Repo(db)(key).find(x)").1, ["Repo", ".call", ".find"]);
    }
}
