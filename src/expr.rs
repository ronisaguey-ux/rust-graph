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
            // `isNaN(it = +it)`: in JavaScript an `=` in an argument is an
            // assignment expression, not a keyword argument — the CST writes
            // the merge `m:[n:it,m:[n:it]]`, and `k(it)` was a shape the source
            // never had.
            let assigned = self.op_byte(1) == b'=' && !matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX");
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
        while let Some(token) = self.tok(0) {
            let kind = token.kind;
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
                Kind::Str => self.string(out),
                Kind::Op => {
                    let byte = self.op_byte(0);
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
                            && self.toks.get(self.pos + 1).is_some_and(|token| token.kind == Kind::Ident)
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
                            if self.op_byte(0) == b'(' {
                                self.pos += 1;
                                let mut params = String::new();
                                self.list(&mut params, b')', false);
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
                if count == 1 {
                    first_end = out.len();
                }
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
                            return self.toks.get(at + 1).is_some_and(|next| next.kind == Kind::Op && self.raw_is(next, b"("));
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
        // `new Fab;` with no parentheses is still a constructor call: the
        // grammar writes the call, the lexer dropped `new` as layout and left
        // the callee standing as a name. `new.target` is not a call, and a
        // dotted or parenthesised callee keeps the old path.
        if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && word == "new"
            && self.op_byte(1) != b'.'
            && self.op_byte(1) != b'('
            && self.word(1).is_some()
            && self.op_byte(2) != b'.'
            && self.op_byte(2) != b'('
            && self.op_byte(2) != b'['
        {
            let name = self.word(1).unwrap_or_default();
            let target = self.toks[self.pos + 1].line;
            self.pos += 2;
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
            }
            out.push('0');
            return;
        }
        let go_word = matches!(word.as_str(), "go" | "defer") && self.lang.tag != "GO";        // `new` is a keyword in the JavaScript family and an ordinary name in
        // Rust: reading it as noise left `new.is_none()` calling nothing, and
        // the record differed by a constant that was never there.
        let js_family = matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX");
        let noise = if js_family {
            JS_NOISE.contains(&word.as_str())
        } else if self.lang.tag == "PY" {
            PY_NOISE.contains(&word.as_str())
        } else {
            NOISE.contains(&word.as_str())
        } && !(self.lang.tag == "RS" && word == "new");
        if noise && !go_word {
            self.pos += 1;
            return;
        }
        self.pos += 1;
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
        // `++i` and `i++`: an increment is not a read of the variable, and the
        // CST writes the whole update expression as a constant. Reading the
        // name here made the file lose a value the source never produced.
        if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && (self.increment_token(-2) || self.increment_token(0))
        {
            out.push('0');
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
            // `x!.length`: the non-null assertion is a type marker between the
            // value and its member, and the CST keeps the chain whole.
            if matches!(self.lang.tag, "TS" | "TSX") && self.is_op(0, b"!") {
                self.pos += 1;
                continue;
            }
            let separator = self.is_op(0, b".")
                || self.is_op(0, b"->")
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
