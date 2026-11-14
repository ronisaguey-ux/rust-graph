//! One forward walk over a file's tokens. Tokens collect into a statement; a
//! statement either opens a block (a definition, a branch) or becomes flow
//! events on the function it sits in. The only state is the open-block stack.

use std::collections::VecDeque;
use crate::expr::{lexeme, Emit};
use crate::facts::{Facts, Import, Symbol, IMPORT_MODULE, IMPORT_STAR, KIND_CLASS, KIND_FUNCTION, KIND_MODULE};
use crate::json::push_string;
use crate::langs::{Blocks, Imports, Language};
use crate::lexer::{Kind, Lexer, Token};

const MODIFIERS: [&str; 34] = [
    "export", "default", "public", "private", "protected", "internal", "static", "final", "abstract", "async", "pub",
    "virtual", "override", "inline", "extern", "sealed", "open", "data", "partial", "local", "declare", "readonly",
    "unsafe", "pure", "external", "view", "payable", "friend", "rec", "impure", "elemental", "recursive", "shared", "module",
];
const CONTROL: [&str; 24] = [
    "if", "elif", "elsif", "elseif", "else", "for", "foreach", "while", "until", "unless", "switch", "match", "try",
    "catch", "except", "finally", "rescue", "ensure", "do", "loop", "when", "with", "select", "repeat",
];
/// A word that continues the branch just closed instead of following it.
const ARMS: [&str; 11] =
    ["else", "elif", "elsif", "elseif", "catch", "except", "finally", "rescue", "ensure", "when", "case"];
const NOT_A_NAME: [&str; 10] = ["return", "sizeof", "new", "function", "throw", "await", "yield", "assert", "print", "defined"];
/// A `{` straight after one of these is a value or a destructuring pattern.
const VALUE_FOLLOWS: [&str; 7] = ["return", "import", "export", "const", "let", "var", "type"];
/// Words after which a `{` opens statements even in expression position, where
/// the body reads exactly like a shorthand field list.
const BLOCK_WORDS: [&[u8]; 4] = [b"unsafe", b"async", b"move", b"try"];
/// Tokens a brace's body is read for before it is called a block. A field list
/// is short; a body past this is not a shape worth more lookahead.
const MAX_BRACE_SCAN: usize = 4096;
const SQL_DDL: &[&str] = &["create", "alter", "drop"];
const COLUMN_SKIP: [&str; 15] = [
    "module", "import", "data", "type", "newtype", "class", "instance", "where", "infixl", "infixr", "infix", "deriving",
    "open", "exposing", "port",
];
const COBOL_HEADINGS: [&str; 4] = ["division", "section", "id", "program-id"];

#[derive(PartialEq, Clone, Copy, Debug)]
enum FrameKind {
    Module,
    Class,
    Function,
    Control,
    Plain,
}

struct Frame {
    kind: FrameKind,
    symbol: usize,
    class: String,
    receiver: String,
    /// Column of the statement that opened the frame (indent blocks).
    col: u32,
    /// A statement interrupted by this block: `app.get('/x', (req) => {`.
    saved: Option<(Vec<Token>, Vec<u8>)>,
    /// The symbol this frame defines, whose end line it sets on closing.
    defines: Option<usize>,
    /// The body of a type declaration: `interface { ... }`, `enum { ... }`.
    /// Its statements are type members and write no fact.
    type_only: bool,
}

#[derive(Default)]
struct Head {
    name: String,
    owner: String,
    params: Vec<String>,
    receiver: String,
    /// Tokens after this index are the body (`def f(x) = x + 1`).
    body: Option<usize>,
}

pub struct Parser<'a> {
    src: &'a [u8],
    lang: &'static Language,
    lexer: Lexer<'a>,
    stmt: Vec<Token>,
    brackets: Vec<u8>,
    frames: Vec<Frame>,
    facts: Facts,
    /// A branch whose block just closed; the next word decides if it goes on.
    closed_branch: bool,
    /// The next `{` belongs to an arm of the open branch.
    rearmed: bool,
    /// Pascal: a header waiting for its `begin`.
    pending: Option<Head>,
    last_line: u32,
    /// Tokens read past the cursor to decide what a `{` opens, in stream order.
    ahead: VecDeque<Token>,
    /// Inside a `var`/`let`/`const` declarator list: every later top-level
    /// comma ends a declaration too, because the CST writes one event per
    /// declarator even when the list is broken over lines.
    declaring: bool,
}

pub fn parse(src: &[u8], lang: &'static Language) -> Facts {
    let parser = Parser {
        src,
        lang,
        lexer: Lexer::new(src, lang),
        stmt: Vec::with_capacity(64),
        brackets: Vec::with_capacity(8),
        frames: Vec::with_capacity(8),
        facts: Facts { tag: lang.tag.to_string(), ..Facts::default() },
        closed_branch: false,
        rearmed: false,
        pending: None,
        last_line: 1,
        ahead: VecDeque::new(),
        declaring: false,
    };
    parser.run()
}

impl<'a> Parser<'a> {
    fn text(&self, token: &Token) -> String {
        lexeme(self.src, token, self.lang)
    }

    fn raw<'t>(&self, token: &'t Token) -> &'a [u8] {
        &self.src[token.start as usize..token.end as usize]
    }

    fn is_op(&self, token: &Token, op: &[u8]) -> bool {
        token.kind == Kind::Op && self.raw(token) == op
    }

    fn word_at(&self, toks: &[Token], index: usize) -> Option<String> {
        toks.get(index).filter(|token| token.kind == Kind::Ident).map(|token| self.text(token))
    }

    /// Whether the statement being read is a `var` / `let` / `const` declaration,
    /// possibly behind `export`, `declare` or `default`. Only those statements
    /// keep several bindings behind top-level commas; anywhere else a comma is
    /// the comma operator and belongs to the expression.
    fn declarator_list(&self) -> bool {
        for token in &self.stmt {
            if token.kind == Kind::Ident {
                return matches!(self.raw(token), b"var" | b"let" | b"const");
            }
            match self.raw(token) {
                b"export" | b"declare" | b"default" => continue,
                _ => return false,
            }
        }
        false
    }

    /// Whether the statement so far is inside a `<...>` type-argument list.
    /// `const cfg = x satisfies Record<string, number>` has a comma at bracket
    /// depth zero - `<` is not a bracket - but it is a type, not a declarator
    /// separator, so the declarator rule must not split there.
    fn inside_type_arguments(&self) -> bool {
        let mut depth = 0i32;
        for token in &self.stmt {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"<" | b"<=" => depth += 1,
                b">" | b">=" | b"->" | b"=>" => depth -= 1,
                _ => {}
            }
        }
        depth > 0
    }

    fn top(&self) -> &Frame {
        self.frames.last().expect("the module frame is never popped")
    }

    /// The next token, from the lookahead buffer before the lexer: a `{` whose
    /// meaning was decided by reading past it must not re-read those tokens
    /// from the lexer, which has already moved on.
    fn next_token(&mut self) -> Token {
        self.ahead.pop_front().unwrap_or_else(|| self.lexer.next())
    }

    /// The n-th token past the cursor, filling the buffer from the lexer. `Eof`
    /// repeats, so a peek past the end is the end rather than a longer buffer.
    fn peek_token(&mut self, n: usize) -> Token {
        while self.ahead.len() <= n {
            if self.ahead.back().is_some_and(|token| token.kind == Kind::Eof) {
                return Token { kind: Kind::Eof, start: 0, end: 0, line: self.last_line, col: 0, interp: None };
            }
            let token = self.lexer.next();
            self.ahead.push_back(token);
        }
        self.ahead[n].clone()
    }

    /// The n-th token past the cursor that is not a line break. A struct
    /// literal written over several lines puts a break between its brace and
    /// its first field, so a field key is only visible past them.
    fn peek_word(&mut self, n: usize) -> Token {
        let mut seen = 0;
        let mut at = 0;
        loop {
            let token = self.peek_token(at);
            if token.kind == Kind::Eof {
                return token;
            }
            if token.kind != Kind::Newline {
                if seen == n {
                    return token;
                }
                seen += 1;
            }
            at += 1;
        }
    }

    fn run(mut self) -> Facts {
        self.facts.symbols.push(Symbol { key: "<module>".into(), kind: KIND_MODULE, line: 1, ..Symbol::default() });
        self.frames.push(Frame {
            kind: FrameKind::Module,
            symbol: 0,
            class: String::new(),
            receiver: String::new(),
            col: 0,
            saved: None,
            defines: None,
        type_only: false,
        });
        loop {
            let token = self.next_token();
            self.last_line = token.line;
            match token.kind {
                Kind::Eof => break,
                Kind::Newline => self.on_newline(),
                Kind::Directive => self.directive(&token),
                Kind::Op => self.on_op(token),
                _ => {
                    self.settle_branch(Some(&token));
                    self.stmt.push(token);
                }
            }
        }
        self.end_statement();
        self.settle_branch(None);
        while self.frames.len() > 1 {
            self.pop_frame();
        }
        self.facts.symbols[0].end_line = self.last_line;
        self.facts.lines = self.last_line;
        self.facts
    }

    fn uses_braces(&self) -> bool {
        self.lang.blocks == Blocks::Brace
    }

    /// Whether the next bytes after `end` are a member access that begins a
    /// new line, ignoring spaces and tabs: `.name` or `?.name`.
    fn next_is_member_access(&self, end: usize) -> bool {
        let mut at = end;
        while at < self.src.len() && matches!(self.src[at], b' ' | b'\t' | b'\r') {
            at += 1;
        }
        if at >= self.src.len() || self.src[at] != b'\n' {
            return false;
        }
        at += 1;
        while at < self.src.len() && matches!(self.src[at], b' ' | b'\t' | b'\r' | b'\n') {
            at += 1;
        }
        matches!(self.src.get(at), Some(b'.')) || self.src.get(at..at + 2) == Some(b"?.")
    }

    /// Whether the next line begins with `?` or `:` — the continuation of a
    /// ternary, never the start of a statement.
    fn next_starts_with_ternary(&self, end: usize) -> bool {
        let mut at = end;
        while at < self.src.len() && matches!(self.src[at], b' ' | b'\t' | b'\r') {
            at += 1;
        }
        if self.src.get(at) != Some(&b'\n') {
            return false;
        }
        at += 1;
        while at < self.src.len() && matches!(self.src[at], b' ' | b'\t' | b'\r' | b'\n') {
            at += 1;
        }
        matches!(self.src.get(at), Some(b'?') | Some(b':'))
    }

    /// Whether the next line begins with `{`, the body of a function expression
    /// whose head was written on the line above.
    fn next_starts_with_brace(&self, end: usize) -> bool {
        let mut at = end;
        while at < self.src.len() && matches!(self.src[at], b' ' | b'\t' | b'\r') {
            at += 1;
        }
        if self.src.get(at) != Some(&b'\n') {
            return false;
        }
        at += 1;
        while at < self.src.len() && matches!(self.src[at], b' ' | b'\t' | b'\r' | b'\n') {
            at += 1;
        }
        self.src.get(at) == Some(&b'{')
    }

    fn on_newline(&mut self) {
        if !self.brackets.is_empty() || self.stmt.is_empty() {
            return;
        }
        if self.lang.tag == "OBJC" && self.is_op(&self.stmt[0], b"@") {
            self.stmt.clear();
            return;
        }
        if self.lang.semicolons || self.lang.period_only {
            return;
        }
        // `String(name)` ends a line and `.replace(…)` begins the next: a
        // statement cannot start with a member access, so the newline is
        // inside the expression. Splitting there gave the continuation its own
        // event, which the CST (rightly) writes once on the line the statement
        // starts, and the guard could not pair them — 160 JS files.
        if let Some(last) = self.stmt.last() {
            if self.next_is_member_access(last.end as usize) {
                return;
            }
            // A ternary written over three lines puts `?` and `:` at the start
            // of the continuation lines. A statement cannot begin with either,
            // so the newline is inside the expression: the CST writes one value
            // on the line the statement starts and the split lost it.
            if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                && self.next_starts_with_ternary(last.end as usize)
            {
                return;
            }
            // `var result,` newline `index = -1,`: a declarator list broken over
            // lines is one declaration to the CST only up to each comma — it
            // writes one event per declarator, so the lexer's single statement
            // blamed the first line for the names of the next.
            // `this.x = function()` newline `{ ... }`: the body of a function
            // expression written with the brace on the next line belongs to the
            // assignment. Ending the statement here wrote `a self.x 0` and left
            // the body in the enclosing symbol, which the CST never does.
            if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                && self.is_op(&last, b")")
                && self.stmt.iter().any(|token| {
                    token.kind == Kind::Ident && self.raw(token) == b"function"
                })
                && self.next_starts_with_brace(last.end as usize)
            {
                return;
            }
        }
        if let Some(last) = self.stmt.last() {
            let continues = self.lang.operator_continues
                && last.kind == Kind::Op
                && !matches!(self.raw(last), b")" | b"]" | b"}" | b":" | b"++" | b"--" | b"=" | b"!" | b"?");
            let trailing_equal = last.kind == Kind::Op && self.raw(last) == b"=" && self.lang.blocks != Blocks::Indent;
            if continues || trailing_equal {
                return;
            }
        }
        self.end_statement();
    }

    fn on_op(&mut self, token: Token) {
        let raw = self.raw(&token);
        if raw.len() != 1 {
            self.stmt.push(token);
            return;
        }
        match raw[0] {
            b'(' | b'[' => {
                self.brackets.push(raw[0]);
                self.stmt.push(token);
            }
            b')' | b']' => {
                let open = if raw[0] == b')' { b'(' } else { b'[' };
                if let Some(at) = self.brackets.iter().rposition(|byte| *byte == open) {
                    self.brackets.truncate(at);
                }
                self.stmt.push(token);
            }
            b'{' => self.on_open_brace(token),
            b'}' => self.on_close_brace(token),
            b';' if self.brackets.is_empty() => {
                self.declaring = false;
                self.end_statement();
            }
            // A match arm ends at its comma, not at the closing brace: without
            // this the whole match is one statement and every arm's value is
            // folded into one merge. An arm whose body was a block already
            // ended at `}`, so its trailing comma is not the start of the next
            // arm.
            b',' if self.brackets.is_empty() && self.stmt.is_empty() => {}
            // A declarator list is several bindings, and the CST writes one
            // event per binding: `var x0 = 0,` newline `x1 = 1;` is two
            // declarations on two lines, not one tuple on the first. Ending the
            // statement at a top-level comma inside var/let/const makes the
            // reader write the same two.
            b',' if self.brackets.is_empty()
                && matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                && (self.declaring || self.declarator_list())
                && !self.inside_type_arguments() =>
            {
                // `var result,` newline `index = -1,` newline `length = a.length;`
                // is three declarations on three lines: the CST writes one event
                // per declarator, and the keyword is only on the first.
                self.declaring = true;
                self.end_statement();
            }
            b',' if self.brackets.is_empty() && self.stmt.iter().any(|token| self.is_op(token, b"=>")) => {
                self.stmt.push(token);
                self.end_statement();
            }
            b'.' if self.lang.period_ends && self.brackets.is_empty() && self.at_sentence_end(&token) => {
                self.stmt.push(token);
                self.end_statement();
            }
            _ => self.stmt.push(token),
        }
    }

    fn at_sentence_end(&self, token: &Token) -> bool {
        self.src.get(token.end as usize).is_none_or(u8::is_ascii_whitespace)
    }

    fn on_open_brace(&mut self, token: Token) {
        let opens_block = self.uses_braces() && self.is_block();
        if opens_block {
            self.settle_branch(None);
            // `interface User extends Base {` opens a body of type members:
            // `greet(who: string): string` is a signature, not a call, and the
            // CST writes nothing for any of it.
            let type_body = matches!(self.lang.tag, "TS" | "TSX") && self.type_declaration(&self.stmt);
            let before = self.frames.len();
            self.open_block();
            if type_body && self.frames.len() > before {
                self.frames[before].type_only = true;
            }
        } else {
            self.brackets.push(b'{');
            self.stmt.push(token);
        }
    }

    fn on_close_brace(&mut self, token: Token) {
        if self.brackets.last() == Some(&b'{') {
            self.brackets.pop();
            self.stmt.push(token);
            return;
        }
        if !self.uses_braces() {
            self.stmt.push(token);
            return;
        }
        self.end_statement();
        self.settle_branch(None);
        self.close_block();
    }

    /// Whether a `{` here starts statements rather than a value.
    fn is_block(&mut self) -> bool {
        // `if let Some(x) = e {`: the statement holds a `=` and the old test
        // read that as an assignment rather than a block opener, so the whole
        // body became one merged value.
        if self.lang.tag == "RS" {
            // `let digits = |c: &mut Cursor| {` and
            // `let render = |out: &mut dyn Write| -> io::Result<()> {`: a
            // closure's block is statements of its own, whether the header
            // ends on the closing bar or on a return type after `->`.
            let closure_header = self.stmt.last().is_some_and(|token| matches!(self.raw(token), b"|" | b"||"))
                || (self.stmt.iter().any(|token| matches!(self.raw(token), b"|" | b"||"))
                    && self.stmt.iter().any(|token| self.is_op(token, b"->")));
            if closure_header {
                return true;
            }
            if let Some(first) = self.stmt.iter().find(|token| token.kind == Kind::Ident) {
                if matches!(self.raw(first), b"if" | b"for" | b"while" | b"match" | b"loop" | b"else") {
                    return true;
                }
            }
        }
        // Cloned, not borrowed: deciding what the brace opens reads past the
        // cursor, and that needs the parser itself.
        let Some(last) = self.stmt.last().cloned() else { return true };
        if self.lang.tag == "TCL" && last.kind == Kind::Ident && self.has_def_word(&self.stmt) {
            return false;
        }
        if self.is_op(&last, b"=>") || (self.is_op(&last, b"->") && !self.brackets.is_empty()) {
            return true;
        }
        // `case 1: { ... }` and `default: { ... }`: the brace after a case label
        // opens a block, not an object. Read as an object the whole block became
        // one value on the case line, which the CST never writes - every case
        // with a block body lost its statements to a merge at the label's line.
        if self.is_op(&last, b":")
            && matches!(
                self.word_at(&self.stmt, 0).as_deref(),
                Some("case") | Some("default")
            )
        {
            return true;
        }
        if !self.brackets.is_empty() {
            return self.is_op(&last, b")") && self.function_word_in_tail();
        }
        if self.has_def_word(&self.stmt) {
            return true;
        }
        if last.kind == Kind::Ident && !BLOCK_WORDS.contains(&self.raw(&last)) && self.field_key_ahead() {
            return false;
        }
        if last.kind == Kind::Op {
            let raw = self.raw(&last);
            if raw != b")" && raw != b">" && raw != b"]" && raw != b"}" {
                return false;
            }
        }
        let last_word = self.word_at(&self.stmt, self.stmt.len() - 1);
        if last_word.is_some_and(|word| VALUE_FOLLOWS.contains(&word.as_str())) {
            return false;
        }
        !(self.top_level_assign(&self.stmt).is_some() && !self.is_op(&last, b")"))
    }

    /// Whether the brace just read opens a struct literal's field list rather
    /// than a block. Read as a block, `Kinds { definitions: A }` binds
    /// `definitions` as a statement of its own and the value the literal
    /// carries is lost.
    ///
    /// The shape is a key behind the brace (`definitions:`) or a shorthand
    /// field, which is a bare name (`Reader { bytes, pos: 0 }`). A path is the
    /// only thing that can name a struct, so the caller checks that first, and
    /// the definition keywords and the arm and control openers are decided
    /// above; what is left to rule out is a block in expression position, whose
    /// body can read as a shorthand list but carries a statement separator.
    fn field_key_ahead(&mut self) -> bool {
        if self.peek_word(0).kind != Kind::Ident {
            return false;
        }
        let after = self.peek_word(1);
        let field = self.is_op(&after, b":") || self.is_op(&after, b",") || self.is_op(&after, b"}");
        field && !self.statements_ahead()
    }

    /// Whether the brace just read holds statements before it closes. A field
    /// list is one expression, so a separator or a binding at the brace's own
    /// depth is the mark of a block the shapes above could not tell apart.
    fn statements_ahead(&mut self) -> bool {
        let mut depth = 0i32;
        for at in 0..MAX_BRACE_SCAN {
            let token = self.peek_token(at);
            match token.kind {
                Kind::Eof => return false,
                Kind::Op => match self.raw(&token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" => depth -= 1,
                    b"}" if depth == 0 => return false,
                    b"}" => depth -= 1,
                    b";" if depth == 0 => return true,
                    _ => {}
                },
                Kind::Ident if depth == 0 && matches!(self.raw(&token), b"let") => return true,
                _ => {}
            }
        }
        // A body longer than the scan is a block: a field list that long is not
        // a shape this reader has to win.
        true
    }

    fn function_word_in_tail(&self) -> bool {
        self.stmt.iter().rev().take(32).any(|token| {
            token.kind == Kind::Ident && self.lang.defs.iter().any(|word| word.as_bytes() == self.raw(token))
        })
    }

    fn has_def_word(&self, toks: &[Token]) -> bool {
        let mut index = 0;
        while index < toks.len().min(8) {
            match self.word_at(toks, index) {
                Some(word) if self.lang.defs.contains(&word.as_str()) => return true,
                Some(word) if MODIFIERS.contains(&word.as_str()) => index += 1,
                _ => return false,
            }
        }
        false
    }

    fn open_block(&mut self) {
        let mut stmt = std::mem::take(&mut self.stmt);
        let in_expression = !self.brackets.is_empty();
        let rearmed = std::mem::take(&mut self.rearmed);
        let start_col = stmt.first().map_or(0, |token| token.col);
        // An arrow written at the start of a statement has no open bracket to
        // mark it as an expression (`return (rng, ...) => {`), but it is still
        // a function of its own: the CST makes it a symbol and files its body
        // there, and keeping the body in the enclosing symbol made every line
        // inside look lost.
        // Only where `=>` is an arrow. In Rust it is a match arm and in Elixir
        // it is a case clause: both are statements of the enclosing block, and
        // reading them as functions moved every following line into a symbol
        // the CST never made (rust 345 -> 417).
        // `type Fn = (x: number) => void` has an arrow and is still a type: the
        // statement is skipped as a declaration, and opening a function here
        // left `Fn` with a return event the CST never writes.
        let type_declaration = matches!(self.lang.tag, "TS" | "TSX") && self.type_declaration(&stmt);
        let arrow = matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && !type_declaration
            && stmt.last().is_some_and(|token| self.is_op(token, b"=>"))
            // `const variance = (values) => {` is named by the assignment, and
            // taking it here made an `<anonymous>` twin the CST never has. A
            // member target (`module.exports = (a) => {`) is the case the
            // assignment cannot name, so that one stays anonymous.
            && self.top_level_assign(&stmt).map_or(true, |equal| {
                stmt[..equal].iter().any(|token| self.is_op(token, b".") || self.is_op(token, b"["))
            });
        if in_expression || arrow {
            let head = self.lambda_head(&stmt);
            let brackets = std::mem::take(&mut self.brackets);
            self.push_function(head, start_col, Some((stmt, brackets)));
            return;
        }
        if rearmed {
            self.final_arm(&stmt);
            self.control_condition(&stmt);
            self.stmt.clear();
            return;
        }
        // `case 1:` and `default:` label the block that follows: the CST writes
        // no event for the label, and a label is not a value.
        let label_only = matches!(
            self.word_at(&stmt, 0).as_deref(),
            Some("case") | Some("default")
        );
        if !label_only {
            self.import_of(&stmt);
        }
        if label_only {
            let symbol = self.top().symbol;
            let class = self.top().class.clone();
            let receiver = self.top().receiver.clone();
            self.frames.push(Frame { kind: FrameKind::Plain, symbol, class, receiver, col: start_col, saved: None, defines: None, type_only: false });
        } else if let Some((name, bases)) = self.class_head(&stmt) {
            self.push_class(name, bases, start_col);
        } else if let Some(head) = self.function_head(&stmt, true) {
            self.push_function(head, start_col, None);
        } else if self.head_word(&stmt).is_some_and(|word| CONTROL.contains(&word.as_str())) {
            self.open_control(&stmt, start_col);
        } else {
            self.statement(&stmt);
            let symbol = self.top().symbol;
            let class = self.top().class.clone();
            let receiver = self.top().receiver.clone();
            self.frames.push(Frame { kind: FrameKind::Plain, symbol, class, receiver, col: start_col, saved: None, defines: None, type_only: false });
        }
        stmt.clear();
        self.stmt = stmt;
    }

    fn open_control(&mut self, stmt: &[Token], col: u32) {
        self.control_condition_before(stmt);
        self.event("[\"b\"]");
        self.loop_binding(stmt);
        let symbol = self.top().symbol;
        let class = self.top().class.clone();
        let receiver = self.top().receiver.clone();
        self.frames.push(Frame { kind: FrameKind::Control, symbol, class, receiver, col, saved: None, defines: None, type_only: false });
    }

    /// `["f"]`: this arm has no condition, so the arms together cover every case.
    fn final_arm(&mut self, header: &[Token]) {
        if self.word_at(header, 0).as_deref() == Some("else") && header.len() == 1 {
            self.event("[\"f\"]");
        }
    }

    fn control_condition(&mut self, stmt: &[Token]) {
        self.control_condition_before(stmt);
        self.loop_binding(stmt);
    }

    fn binding_index(&self, stmt: &[Token]) -> Option<usize> {
        let mut depth = 0;
        for (index, token) in stmt.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    _ => {}
                }
            }
            if depth <= 1 && token.kind == Kind::Ident && index > 0 && matches!(self.raw(token), b"in" | b"of" | b"as") {
                return Some(index);
            }
        }
        None
    }

    fn control_condition_before(&mut self, stmt: &[Token]) {
        // Python's `match subject:` is a switch and `case pattern:` is a label:
        // neither writes a value the CST records, and the guard read both
        // events as lost the moment the CST (rightly) wrote nothing.
        if self.lang.tag == "PY" && matches!(self.raw(&stmt[0]), b"match" | b"case") {
            return;
        }
        if self.binding_index(stmt).is_none() && stmt.len() > 1 {
            self.expression_event("x", &stmt[1..]);
        }
    }

    /// `for a, b in c`, `for (const a of b)`, `with c as a`.
    fn loop_binding(&mut self, stmt: &[Token]) {
        let Some(at) = self.binding_index(stmt) else { return };
        let is_with = self.raw(&stmt[at]) == b"as";
        let (targets, value) = if is_with { (&stmt[at + 1..], &stmt[1..at]) } else { (&stmt[1..at], &stmt[at + 1..]) };
        self.assignment(targets, value, stmt[0].line, false);
    }

    fn head_word(&self, toks: &[Token]) -> Option<String> {
        toks.iter().find(|token| token.kind == Kind::Ident).map(|token| self.text(token)).filter(|_| toks[0].kind == Kind::Ident)
    }

    fn add_symbol(&mut self, name: String, owner: String, kind: u8, line: u32, params: Vec<String>) -> usize {
        let base = if owner.is_empty() { name.clone() } else { format!("{owner}.{name}") };
        let taken = self.facts.symbols.iter().any(|symbol| symbol.key == base);
        let key = if taken { format!("{base}@{line}") } else { base };
        self.facts.symbols.push(Symbol { key, name, owner, kind, line, end_line: line, params, ..Symbol::default() });
        self.facts.symbols.len() - 1
    }

    fn push_class(&mut self, name: String, bases: Vec<String>, col: u32) {
        let line = self.last_line;
        let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
        self.facts.symbols[index].bases = bases;
        let symbol = self.top().symbol;
        self.frames.push(Frame {
            kind: FrameKind::Class,
            symbol,
            class: name,
            receiver: String::new(),
            col,
            saved: None,
            defines: Some(index),
        type_only: false,
        });
    }

    fn push_function(&mut self, head: Head, col: u32, saved: Option<(Vec<Token>, Vec<u8>)>) {
        let index = self.define(head.name, head.owner, head.params, self.last_line);
        let class = self.facts.symbols[index].owner.clone();
        self.frames.push(Frame {
            kind: FrameKind::Function,
            symbol: index,
            class,
            receiver: head.receiver,
            col,
            saved,
            defines: Some(index),
        type_only: false,
        });
    }

    fn define(&mut self, name: String, owner: String, mut params: Vec<String>, line: u32) -> usize {
        let owner = if owner.is_empty() && self.top().kind == FrameKind::Class { self.top().class.clone() } else { owner };
        if params.first().is_some_and(|first| matches!(first.as_str(), "self" | "cls" | "this")) {
            params.remove(0);
        }
        if params.is_empty() {
            params = self.lang.implicit_params.iter().map(|param| param.to_string()).collect();
        }
        let name = if name.is_empty() { format!("<anonymous>@{line}") } else { name };
        self.add_symbol(name, owner, KIND_FUNCTION, line, params)
    }

    fn pop_frame(&mut self) -> Frame {
        let frame = self.frames.pop().expect("a frame to pop");
        if let Some(index) = frame.defines {
            self.facts.symbols[index].end_line = self.last_line;
        }
        frame
    }

    fn close_block(&mut self) {
        if self.frames.len() <= 1 {
            return;
        }
        if self.top().kind == FrameKind::Control {
            self.closed_branch = true;
            return;
        }
        let frame = self.pop_frame();
        if let Some((stmt, brackets)) = frame.saved {
            self.stmt = stmt;
            self.brackets = brackets;
            self.stmt.push(Token { kind: Kind::Lambda, start: 0, end: 0, line: self.last_line, col: 0, interp: None });
        }
    }

    /// After a closed branch: an arm word reopens it, anything else ends it.
    fn settle_branch(&mut self, next: Option<&Token>) {
        if !self.closed_branch {
            return;
        }
        self.closed_branch = false;
        let is_arm = next.is_some_and(|token| token.kind == Kind::Ident && ARMS.contains(&self.text(token).as_str()));
        if is_arm {
            self.event("[\"e\"]");
            self.rearmed = true;
        } else {
            self.event("[\"j\"]");
            self.pop_frame();
        }
    }

    fn event(&mut self, text: &str) {
        let symbol = self.top().symbol;
        let events = &mut self.facts.symbols[symbol].events;
        if !events.is_empty() {
            events.push(',');
        }
        events.push_str(text);
    }

    fn with_emit(&mut self, toks: &[Token], write: impl FnOnce(&mut Emit, &mut String)) -> String {
        let symbol = self.top().symbol;
        let receiver = self.top().receiver.clone();
        let mut calls = std::mem::take(&mut self.facts.symbols[symbol].calls);
        let mut out = String::new();
        let mut emit = Emit { src: self.src, lang: self.lang, toks, pos: 0, calls: &mut calls, receiver: &receiver };
        write(&mut emit, &mut out);
        self.facts.symbols[symbol].calls = calls;
        out
    }

    fn expression_event(&mut self, form: &str, toks: &[Token]) {
        let Some(first) = toks.first() else { return };
        let line = first.line;
        let value = self.values(toks);
        if value != "0" || form == "r" {
            self.event(&format!("[\"{form}\",{line},{value}]"));
        }
    }

    /// A whole expression; several top-level values become one tuple.
    fn values(&mut self, toks: &[Token]) -> String {
        let (items, count) = {
            let mut count = 0;
            let items = self.with_emit(toks, |emit, out| count = emit.list(out, 0, false));
            (items, count)
        };
        match count {
            0 => "0".to_string(),
            // A count with no text behind it means the reader stopped before it
            // wrote anything: an empty value is not a value, and left in the
            // event it produces `[,0]`, which is not JSON.
            1 if items.is_empty() => "0".to_string(),
            1 => items,
            _ => format!("[\"t\",[{items}]]"),
        }
    }

    fn end_statement(&mut self) {
        if self.stmt.is_empty() {
            self.brackets.clear();
            return;
        }
        // `@dec` and `@Component({...})` written on their own line decorate the
        // declaration that follows: a decorator is not a statement, so it writes
        // no event. A decorator sharing its line with a declaration is not
        // caught here: that statement carries the class/function keyword and is
        // read normally.
        if matches!(self.lang.tag, "TS" | "TSX")
            && self.stmt.len() > 1
            && self.is_op(&self.stmt[0], b"@")
            && !self.stmt.iter().any(|token| {
                token.kind == Kind::Ident
                    && matches!(
                        self.raw(token),
                        b"class" | b"function" | b"interface" | b"enum" | b"namespace"
                            | b"module" | b"let" | b"const" | b"var" | b"export" | b"declare"
                            | b"abstract" | b"async"
                    )
            })
        {
            self.brackets.clear();
            self.stmt.clear();
            return;
        }
        // `pub(super) fn parse_with(..) -> (Workspace, Outcome)` is a
        // declaration header, not a statement: the block it opens is the same
        // symbol, and the header carries no value the CST records.
        if self.lang.tag == "RS"
            && self.stmt.iter().any(|token| token.kind == Kind::Ident && self.raw(token) == b"fn")
            && !self.stmt.iter().any(|token| self.raw(token) == b"=")
            && !self.stmt.iter().any(|token| self.raw(token) == b"||")
        {
            self.brackets.clear();
            self.stmt.clear();
            return;
        }
        let stmt = std::mem::take(&mut self.stmt);
        self.brackets.clear();
        match self.lang.blocks {
            Blocks::Brace => self.brace_statement(&stmt),
            Blocks::Indent => self.indent_statement(&stmt),
            Blocks::End => self.end_family_statement(&stmt),
            Blocks::BeginEnd => self.pascal_statement(&stmt),
            Blocks::NamedEnd => self.named_end_statement(&stmt),
            Blocks::Column => self.column_statement(&stmt),
        }
        let mut stmt = stmt;
        stmt.clear();
        self.stmt = stmt;
    }

    fn brace_statement(&mut self, stmt: &[Token]) {
        let rearmed = std::mem::take(&mut self.rearmed);
        let word = self.head_word(stmt);
        if matches!(word.as_deref(), Some("case" | "default")) {
            if self.top().kind == FrameKind::Control {
                self.event("[\"e\"]");
            }
            if let Some(colon) = stmt.iter().position(|token| self.is_op(token, b":")) {
                self.simple_statement(&stmt[colon + 1..]);
            }
            return;
        }
        if rearmed {
            // `else x = y;`: the arm is this one statement.
            self.final_arm(&stmt[..1]);
            self.simple_statement(&stmt[1..]);
            self.event("[\"j\"]");
            self.pop_frame();
            return;
        }
        if let Some(word) = word.filter(|word| matches!(word.as_str(), "if" | "for" | "while" | "foreach")) {
            let _ = word;
            if let Some(end) = self.matching_close(stmt, 1) {
                if end + 1 < stmt.len() {
                    self.expression_event("x", &stmt[2..end]);
                    self.event("[\"b\"]");
                    self.simple_statement(&stmt[end + 1..]);
                    self.event("[\"j\"]");
                    return;
                }
            }
        }
        if self.import_of(stmt) {
            return;
        }
        if self.function_head(stmt, false).is_some_and(|head| head.body.is_some()) {
            let head = self.function_head(stmt, false).unwrap_or_default();
            self.inline_function(head, stmt);
            return;
        }
        self.simple_statement(stmt);
    }

    fn matching_close(&self, stmt: &[Token], open: usize) -> Option<usize> {
        if !stmt.get(open).is_some_and(|token| self.is_op(token, b"(")) {
            return None;
        }
        let mut depth = 0;
        for (index, token) in stmt.iter().enumerate().skip(open) {
            if self.is_op(token, b"(") {
                depth += 1;
            } else if self.is_op(token, b")") {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
        None
    }

    /// A definition whose body is the rest of its statement.
    fn inline_function(&mut self, head: Head, stmt: &[Token]) {
        let body = head.body.unwrap_or(stmt.len());
        self.push_function(head, stmt[0].col, None);
        self.expression_event("r", &stmt[body..]);
        self.last_line = stmt[stmt.len() - 1].line;
        self.pop_frame();
    }

    fn indent_statement(&mut self, stmt: &[Token]) {
        let col = stmt[0].col;
        let word = self.head_word(stmt);
        let arm = word.as_deref().is_some_and(|word| ARMS.contains(&word));
        while self.frames.len() > 1 && self.top().col >= col {
            if self.top().col == col && arm && self.top().kind == FrameKind::Control {
                self.event("[\"e\"]");
                self.after_header(stmt, None);
                return;
            }
            if self.top().kind == FrameKind::Control {
                self.event("[\"j\"]");
            }
            self.pop_frame();
        }
        if self.import_of(stmt) {
            return;
        }
        let colon = self.suite_colon(stmt);
        let header = &stmt[..colon.unwrap_or(stmt.len())];
        // A Python `case pattern:` is a branch label, not a statement: it opens
        // the arm's suite and writes no value. Falling through to
        // simple_statement wrote an `x` event per case, and the CST writes none.
        if self.lang.tag == "PY" && word.as_deref() == Some("case") && colon.is_some() {
            if self.top().kind == FrameKind::Control {
                self.event("[\"e\"]");
            }
            self.after_header(stmt, colon);
            return;
        }
        if let Some((name, bases)) = self.class_head(header) {
            self.push_class(name, bases, col);
        } else if let Some(mut head) = self.function_head(header, true) {
            head.body = None;
            self.push_function(head, col, None);
        } else if word.as_deref().is_some_and(|word| CONTROL.contains(&word)) && colon.is_some() {
            self.open_control(header, col);
        } else {
            self.simple_statement(stmt);
            return;
        }
        self.after_header(stmt, colon);
    }

    /// The statement on the same line as a `:`.
    fn after_header(&mut self, stmt: &[Token], colon: Option<usize>) {
        let colon = colon.or_else(|| self.suite_colon(stmt));
        if let Some(at) = colon {
            if colon.is_some() && self.top().kind == FrameKind::Control && self.is_arm_header(stmt) {
                self.final_arm(&stmt[..at]);
                self.control_condition(&stmt[..at]);
            }
            if at + 1 < stmt.len() {
                self.simple_statement(&stmt[at + 1..]);
            }
        }
    }

    fn is_arm_header(&self, stmt: &[Token]) -> bool {
        self.head_word(stmt).is_some_and(|word| ARMS.contains(&word.as_str()))
    }

    /// The `:` that ends a block header, or a Nim `=`.
    fn suite_colon(&self, stmt: &[Token]) -> Option<usize> {
        let mut depth = 0;
        let mut lambda = 0;
        for (index, token) in stmt.iter().enumerate() {
            if token.kind == Kind::Ident && self.raw(token) == b"lambda" {
                lambda += 1;
            }
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" => depth += 1,
                b")" | b"]" | b"}" => depth -= 1,
                b":" if depth == 0 && lambda > 0 => lambda -= 1,
                b":" if depth == 0 && self.lang.tag != "NIM" => {
                    let first = self.head_word(stmt)?;
                    let opens = CONTROL.contains(&first.as_str())
                        || self.lang.defs.contains(&first.as_str())
                        || self.lang.classes.contains(&first.as_str())
                        || first == "async"
                        // Python's `case pattern:` opens the arm's suite like a
                        // header, but it is not a control word: without this the
                        // colon was not a suite colon and the line fell through
                        // to the expression path as a value named `case`.
                        || (self.lang.tag == "PY" && first == "case");
                    return opens.then_some(index);
                }
                b":" if depth == 0 && index + 1 == stmt.len() => return Some(index),
                b"=" if depth == 0 && self.lang.tag == "NIM" && self.has_def_word(stmt) => return Some(index),
                _ => {}
            }
        }
        None
    }

    fn end_family_statement(&mut self, stmt: &[Token]) {
        let Some(word) = self.head_word(stmt) else {
            self.simple_statement(stmt);
            return;
        };
        if self.lang.closers.contains(&word.as_str()) {
            self.close_end_block();
            return;
        }
        if ARMS.contains(&word.as_str()) && self.top().kind == FrameKind::Control {
            self.event("[\"e\"]");
            self.final_arm(stmt);
            // Python's `case pattern:` is a label, not a value: the CST writes
            // no event for it, so neither does the lexer.
            if !(self.lang.tag == "PY" && word == "case") {
                self.expression_event("x", &stmt[1..]);
            }
            return;
        }
        if self.import_of(stmt) {
            return;
        }
        let one_liner = self.word_at(stmt, stmt.len() - 1).is_some_and(|last| self.lang.closers.contains(&last.as_str()));
        let col = stmt[0].col;
        if let Some((name, bases)) = self.class_head(stmt) {
            if !one_liner {
                self.push_class(name, bases, col);
            }
            return;
        }
        if let Some(head) = self.function_head(stmt, true) {
            if self.lang.tag == "M" && self.top().kind == FrameKind::Function {
                self.pop_frame();
            }
            let inline_body = stmt.windows(2).position(|pair| {
                self.raw(&pair[0]) == b"do" && self.is_op(&pair[1], b":")
            });
            if let Some(at) = inline_body {
                let mut head = head;
                head.body = Some(at + 2);
                self.inline_function(head, stmt);
            } else if one_liner {
                self.push_function(head, col, None);
                self.simple_statement(&stmt[..stmt.len() - 1]);
                self.pop_frame();
            } else {
                self.push_function(head, col, None);
            }
            return;
        }
        if self.lang.openers.contains(&word.as_str()) {
            if one_liner {
                self.simple_statement(stmt);
            } else {
                self.open_control(stmt, col);
            }
            return;
        }
        let opener = stmt.iter().position(|token| {
            token.kind == Kind::Ident && self.lang.inline_openers.iter().any(|word| word.as_bytes() == self.raw(token))
        });
        if let Some(at) = opener {
            self.simple_statement(&stmt[..at]);
            let symbol = self.top().symbol;
            let class = self.top().class.clone();
            let receiver = self.top().receiver.clone();
            self.frames.push(Frame { kind: FrameKind::Plain, symbol, class, receiver, col, saved: None, defines: None, type_only: false });
            return;
        }
        self.simple_statement(stmt);
    }

    fn close_end_block(&mut self) {
        if self.frames.len() <= 1 {
            return;
        }
        if self.top().kind == FrameKind::Control {
            self.event("[\"j\"]");
        }
        self.pop_frame();
    }

    fn pascal_statement(&mut self, stmt: &[Token]) {
        let Some(word) = self.head_word(stmt) else {
            self.simple_statement(stmt);
            return;
        };
        if self.import_of(stmt) {
            return;
        }
        if self.lang.defs.contains(&word.as_str()) || self.has_def_word(stmt) {
            self.pending = self.function_head(stmt, true);
            return;
        }
        // `begin`, `end` and the block words arrive as the first word of what
        // may be a longer statement, because they carry no `;` of their own.
        let mut start = 0;
        while let Some(word) = self.word_at(stmt, start) {
            if word == "begin" || word == "try" || word == "repeat" {
                match self.pending.take() {
                    Some(head) => self.push_function(head, 0, None),
                    None => {
                        let symbol = self.top().symbol;
                        let class = self.top().class.clone();
                        self.frames.push(Frame {
                            kind: FrameKind::Plain,
                            symbol,
                            class,
                            receiver: String::new(),
                            col: 0,
                            saved: None,
                            defines: None,
                        type_only: false,
                        });
                    }
                }
                start += 1;
            } else if self.lang.closers.contains(&word.as_str()) {
                self.close_end_block();
                start += 1;
            } else {
                break;
            }
        }
        if start < stmt.len() {
            self.simple_statement(&stmt[start..]);
        }
    }

    fn named_end_statement(&mut self, stmt: &[Token]) {
        let Some(word) = self.head_word(stmt) else {
            self.simple_statement(stmt);
            return;
        };
        if word == "end" {
            let next = self.word_at(stmt, 1);
            if next.is_none() || next.is_some_and(|next| self.lang.defs.contains(&next.as_str())) {
                if self.top().kind == FrameKind::Function {
                    self.pop_frame();
                }
            }
            return;
        }
        if self.import_of(stmt) {
            return;
        }
        if let Some(head) = self.function_head(stmt, true) {
            self.push_function(head, stmt[0].col, None);
            return;
        }
        self.simple_statement(stmt);
    }

    fn column_statement(&mut self, stmt: &[Token]) {
        let limit = if self.lang.tag == "COBOL" { 7 } else { 0 };
        // A SQL body is free to sit in column one; only DDL ends a routine.
        let sql_body = self.lang.tag == "SQL" && !self.word_at(stmt, 0).is_some_and(|word| SQL_DDL.contains(&word.as_str()));
        let starts_a_definition = stmt[0].col <= limit && !sql_body;
        if starts_a_definition {
            while self.frames.len() > 1 {
                self.pop_frame();
            }
            if self.import_of(stmt) {
                return;
            }
            if let Some(head) = self.column_head(stmt) {
                let body = head.body.unwrap_or(stmt.len());
                self.push_function(head, 0, None);
                if body < stmt.len() {
                    self.expression_event("r", &stmt[body..]);
                }
                return;
            }
            if self.function_head(stmt, true).is_some() {
                let mut head = self.function_head(stmt, true).unwrap_or_default();
                let body = head.body.take().unwrap_or(stmt.len());
                self.push_function(head, 0, None);
                if body < stmt.len() {
                    self.expression_event("r", &stmt[body..]);
                }
                return;
            }
        }
        self.simple_statement(stmt);
    }

    fn column_head(&self, stmt: &[Token]) -> Option<Head> {
        let first = self.word_at(stmt, 0);
        if self.lang.tag == "CLJ" {
            let word = self.word_at(stmt, 1)?;
            if !self.lang.defs.contains(&word.as_str()) {
                return None;
            }
            let name = self.word_at(stmt, 2)?;
            let open = stmt.iter().position(|token| self.is_op(token, b"["))?;
            let close = stmt.iter().position(|token| self.is_op(token, b"]"))?;
            let params = stmt[open + 1..close].iter().filter(|t| t.kind == Kind::Ident).map(|t| self.text(t)).collect();
            return Some(Head { name, params, body: Some(close + 1), ..Head::default() });
        }
        if self.lang.tag == "SQL" {
            let keyword = (0..stmt.len().min(5)).find(|at| self.word_at(stmt, *at).is_some_and(|word| self.lang.defs.contains(&word.as_str())))?;
            let (name, owner, open) = self.name_chain(stmt, keyword + 1);
            let params = self.group_at(stmt, open).map_or_else(Vec::new, |(first, close)| self.params(&stmt[first..close]));
            return Some(Head { name, owner, params, ..Head::default() });
        }
        if self.lang.tag == "COBOL" {
            let name = first?;
            let plain = stmt.len() == 2 && self.is_op(&stmt[1], b".");
            return (plain && !COBOL_HEADINGS.iter().any(|word| name.ends_with(word))).then(|| Head { name, ..Head::default() });
        }
        if self.lang.tag == "ERL" {
            let name = first?;
            let close = self.matching_close(stmt, 1)?;
            let params = self.params(&stmt[2..close]);
            let arrow = stmt.iter().position(|token| self.is_op(token, b"->"))?;
            return Some(Head { name, params, body: Some(arrow + 1), ..Head::default() });
        }
        let mut index = 0;
        if !self.lang.defs.is_empty() {
            if !self.word_at(stmt, 0).is_some_and(|word| self.lang.defs.contains(&word.as_str())) {
                return None;
            }
            index = 1;
            while self.word_at(stmt, index).is_some_and(|word| MODIFIERS.contains(&word.as_str())) {
                index += 1;
            }
        }
        let name = self.word_at(stmt, index)?;
        if COLUMN_SKIP.contains(&name.as_str()) || stmt.get(index + 1).is_some_and(|token| self.is_op(token, b"::")) {
            return None;
        }
        let equal = stmt.iter().position(|token| self.is_op(token, b"="))?;
        let params = stmt[index + 1..equal].iter().filter(|t| t.kind == Kind::Ident).map(|t| self.text(t)).collect();
        Some(Head { name, params, body: Some(equal + 1), ..Head::default() })
    }

    fn directive(&mut self, token: &Token) {
        let line = self.raw(token);
        let text = String::from_utf8_lossy(line);
        let rest = text.trim_start_matches('#').trim_start();
        let is_include = rest.starts_with("include") || rest.starts_with("import");
        if !is_include {
            return;
        }
        if let (Some(open), Some(close)) = (rest.find('"'), rest.rfind('"')) {
            if open < close {
                self.facts.imports.push(Import {
                    line: token.line,
                    module: rest[open + 1..close].to_string(),
                    kind: IMPORT_STAR,
                    ..Import::default()
                });
            }
        }
    }

    fn string_value(&self, token: &Token) -> String {
        let text = String::from_utf8_lossy(self.raw(token));
        text.trim_matches(|ch| matches!(ch, '"' | '\'' | '`')).to_string()
    }

    /// Records an import if this statement is one; true if nothing else is left in it.
    fn import_of(&mut self, stmt: &[Token]) -> bool {
        if stmt.is_empty() {
            return false;
        }
        match self.lang.imports {
            Imports::Python => self.python_import(stmt),
            Imports::Js => self.js_import(stmt),
            Imports::Path => self.path_import(stmt),
            Imports::Include | Imports::None => false,
        }
    }

    fn dotted(&self, toks: &[Token]) -> (String, usize) {
        let mut text = String::new();
        let mut index = 0;
        while let Some(token) = toks.get(index) {
            let raw = self.raw(token);
            let joins = token.kind == Kind::Op && raw.iter().all(|byte| *byte == b'.');
            if token.kind == Kind::Ident && !matches!(raw, b"import" | b"as") {
                if text.ends_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
                    break;
                }
            } else if !joins {
                break;
            }
            text.push_str(&String::from_utf8_lossy(raw));
            index += 1;
        }
        (text, index)
    }

    fn python_import(&mut self, stmt: &[Token]) -> bool {
        let line = stmt[0].line;
        match self.word_at(stmt, 0).as_deref() {
            Some("import") => {
                let parts: Vec<&[Token]> = stmt[1..].split(|token| self.is_op(token, b",")).collect();
                for part in parts {
                    let (module, used) = self.dotted(part);
                    let alias = if self.word_at(part, used).as_deref() == Some("as") {
                        self.word_at(part, used + 1).unwrap_or_default()
                    } else {
                        module.clone()
                    };
                    self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
                }
                true
            }
            Some("from") => {
                let (module, used) = self.dotted(&stmt[1..]);
                let names = &stmt[(used + 2).min(stmt.len())..];
                let parts: Vec<&[Token]> = names.split(|token| token.kind == Kind::Op && matches!(self.raw(token), b"," | b"(" | b")")).collect();
                for part in parts {
                    let Some(first) = part.first() else { continue };
                    if self.is_op(first, b"*") {
                        self.facts.imports.push(Import { line, module: module.clone(), kind: IMPORT_STAR, ..Import::default() });
                        continue;
                    }
                    let name = self.text(first);
                    let alias = if part.len() >= 3 { self.text(&part[2]) } else { name.clone() };
                    self.facts.imports.push(Import { line, module: module.clone(), name, alias, kind: IMPORT_MODULE });
                }
                true
            }
            _ => false,
        }
    }

    fn js_import(&mut self, stmt: &[Token]) -> bool {
        let Some(line) = stmt.first().map(|token| token.line) else {
            return false;
        };
        let word = self.word_at(stmt, 0);
        let from = stmt.iter().position(|token| token.kind == Kind::Ident && self.raw(token) == b"from");
        let is_static = matches!(word.as_deref(), Some("import" | "export")) && !stmt.get(1).is_some_and(|t| self.is_op(t, b"("));
        if is_static {
            let module_at = match from {
                Some(at) => at + 1,
                None if word.as_deref() == Some("import") => 1,
                None => return false,
            };
            let Some(module) = stmt.get(module_at).filter(|token| token.kind == Kind::Str).map(|token| self.string_value(token)) else {
                return false;
            };
            self.js_bindings(&stmt[1..module_at.saturating_sub(1).max(1)], &module, line);
            return true;
        }
        let Some(at) = stmt.iter().position(|token| token.kind == Kind::Ident && self.raw(token) == b"require") else {
            return false;
        };
        let Some(module) = stmt[at..].iter().take(3).find(|token| token.kind == Kind::Str).map(|token| self.string_value(token)) else {
            return false;
        };
        let Some(equal) = self.top_level_assign(stmt) else {
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
            return true;
        };
        let targets = &stmt[..equal];
        if let Some(open) = targets.iter().position(|token| self.is_op(token, b"{")) {
            self.js_named(&targets[open + 1..], &module, line, b":");
        } else if let Some(name) = targets.iter().rev().find(|token| token.kind == Kind::Ident) {
            let alias = self.text(name);
            self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
        }
        true
    }

    fn js_bindings(&mut self, toks: &[Token], module: &str, line: u32) {
        let mut index = 0;
        while index < toks.len() {
            let token = &toks[index];
            if self.is_op(token, b"{") {
                self.js_named(&toks[index + 1..], module, line, b"as");
                return;
            }
            if self.is_op(token, b"*") {
                let alias = if self.word_at(toks, index + 1).as_deref() == Some("as") {
                    self.word_at(toks, index + 2).unwrap_or_default()
                } else {
                    String::new()
                };
                let kind = if alias.is_empty() { IMPORT_STAR } else { IMPORT_MODULE };
                self.facts.imports.push(Import { line, module: module.into(), alias, kind, ..Import::default() });
                return;
            }
            if token.kind == Kind::Ident && self.raw(token) != b"type" {
                let alias = self.text(token);
                self.facts.imports.push(Import { line, module: module.into(), alias, kind: IMPORT_MODULE, ..Import::default() });
            }
            index += 1;
        }
    }

    fn js_named(&mut self, toks: &[Token], module: &str, line: u32, rename: &[u8]) {
        let close = toks.iter().position(|token| self.is_op(token, b"}")).unwrap_or(toks.len());
        let parts: Vec<&[Token]> = toks[..close].split(|token| self.is_op(token, b",")).collect();
        for part in parts {
            let Some(first) = part.iter().find(|token| token.kind == Kind::Ident) else { continue };
            let name = self.text(first);
            let renamed = part.iter().position(|token| self.raw(token) == rename);
            let alias = renamed.and_then(|at| part.get(at + 1)).map_or(name.clone(), |token| self.text(token));
            self.facts.imports.push(Import { line, module: module.into(), name, alias, kind: IMPORT_MODULE });
        }
    }

    fn path_import(&mut self, stmt: &[Token]) -> bool {
        let mut start = 0;
        while self.word_at(stmt, start).as_deref().is_some_and(|word| MODIFIERS.contains(&word)) {
            start += 1;
            if stmt.get(start).is_some_and(|t| self.is_op(t, b"(")) {
                if let Some((_, close)) = self.group_at(stmt, start) {
                    start = close + 1;
                }
            }
        }
        let local_require = self.top_level_assign(stmt).filter(|equal| {
            self.word_at(stmt, equal + 1).is_some_and(|word| self.lang.import_words.contains(&word.as_str()))
        });
        if let Some(equal) = local_require {
            let module = stmt[equal..].iter().find(|token| token.kind == Kind::Str).map(|token| self.string_value(token));
            let alias = stmt[..equal].iter().rev().find(|token| token.kind == Kind::Ident).map(|token| self.text(token));
            if let (Some(module), Some(alias)) = (module, alias) {
                self.facts.imports.push(Import { line: stmt[0].line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
                return true;
            }
            return false;
        }
        let Some(word) = self.word_at(stmt, start) else { return false };
        if !self.lang.import_words.contains(&word.as_str()) {
            return false;
        }
        let line = stmt[0].line;
        let rest = &stmt[start + 1..];
        let strings: Vec<&Token> = rest.iter().filter(|token| token.kind == Kind::Str).collect();
        if !strings.is_empty() {
            for (index, token) in rest.iter().enumerate() {
                if token.kind != Kind::Str {
                    continue;
                }
                let module = self.string_value(token);
                let named = index > 0 && rest[index - 1].kind == Kind::Ident;
                let last = module.rsplit('/').next().unwrap_or("").to_string();
                let alias = if named { self.text(&rest[index - 1]) } else { last };
                let kind = if self.lang.tag == "GO" || named { IMPORT_MODULE } else { IMPORT_STAR };
                self.facts.imports.push(Import { line, module, alias, kind, ..Import::default() });
            }
            return true;
        }
        let mut module = String::new();
        let mut alias = String::new();
        let mut index = 0;
        while let Some(token) = rest.get(index) {
            match token.kind {
                Kind::Ident if self.raw(token) == b"as" => {
                    alias = self.word_at(rest, index + 1).unwrap_or_default();
                    index += 1;
                }
                Kind::Ident => module.push_str(&self.text(token)),
                Kind::Op if self.is_op(token, b"{") => {
                    let prefix = module.trim_end_matches('.').to_string();
                    let before = self.facts.imports.len();
                    self.path_use_list(&rest[index + 1..], &prefix, line);
                    return self.facts.imports.len() > before || !prefix.is_empty();
                }
                Kind::Op if matches!(self.raw(token), b"." | b"::" | b"\\" | b"/" | b":") => module.push('.'),
                Kind::Op if self.is_op(token, b"*") => {
                    self.facts.imports.push(Import {
                        line,
                        module: module.trim_end_matches('.').to_string(),
                        kind: IMPORT_STAR,
                        ..Import::default()
                    });
                    return true;
                }
                _ => break,
            }
            index += 1;
        }
        let module = module.trim_end_matches('.').to_string();
        if module.is_empty() {
            return false;
        }
        let alias = if alias.is_empty() { module.rsplit('.').next().unwrap_or("").to_string() } else { alias };
        self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
        true
    }

    fn path_use_list(&mut self, toks: &[Token], base_module: &str, line: u32) {
        let mut depth = 0usize;
        let mut close = toks.len();
        for (i, token) in toks.iter().enumerate() {
            if self.is_op(token, b"{") {
                depth += 1;
            } else if self.is_op(token, b"}") {
                if depth == 0 {
                    close = i;
                    break;
                }
                depth -= 1;
            }
        }
        let mut start = 0usize;
        let mut nest = 0usize;
        for i in 0..=close {
            let at_end = i == close;
            let is_top_comma = !at_end && {
                let token = &toks[i];
                if self.is_op(token, b"{") {
                    nest += 1;
                    false
                } else if self.is_op(token, b"}") {
                    nest = nest.saturating_sub(1);
                    false
                } else {
                    nest == 0 && self.is_op(token, b",")
                }
            };
            if at_end || is_top_comma {
                let part = &toks[start..i];
                start = i + 1;
                self.path_use_item(part, base_module, line);
            }
        }
    }

    fn path_use_item(&mut self, part: &[Token], base_module: &str, line: u32) {
        if let Some(brace_at) = part.iter().position(|t| self.is_op(t, b"{")) {
            let sub: Vec<String> = part[..brace_at]
                .iter()
                .filter(|t| t.kind == Kind::Ident)
                .map(|t| self.text(t))
                .collect();
            let sub_str = sub.join(".");
            let next_module = match (base_module.is_empty(), sub_str.is_empty()) {
                (true, _) => sub_str,
                (_, true) => base_module.to_string(),
                _ => format!("{base_module}.{sub_str}"),
            };
            self.path_use_list(&part[brace_at + 1..], &next_module, line);
            return;
        }
        if let Some(star_at) = part.iter().position(|t| self.is_op(t, b"*")) {
            let sub: Vec<String> = part[..star_at]
                .iter()
                .filter(|t| t.kind == Kind::Ident)
                .map(|t| self.text(t))
                .collect();
            let sub_str = sub.join(".");
            let star_module = match (base_module.is_empty(), sub_str.is_empty()) {
                (true, _) => sub_str,
                (_, true) => base_module.to_string(),
                _ => format!("{base_module}.{sub_str}"),
            };
            self.facts.imports.push(Import {
                line,
                module: star_module,
                kind: IMPORT_STAR,
                ..Import::default()
            });
            return;
        }
        let rename_at = part
            .iter()
            .position(|t| (t.kind == Kind::Ident && self.raw(t) == b"as") || self.is_op(t, b"=>"));
        let path_slice = rename_at.map_or(part, |at| &part[..at]);
        let mut segs: Vec<String> = path_slice
            .iter()
            .filter(|t| t.kind == Kind::Ident)
            .map(|t| self.text(t))
            .collect();
        let Some(name) = segs.pop() else { return };
        let sub_str = segs.join(".");
        let item_module = match (base_module.is_empty(), sub_str.is_empty()) {
            (true, _) => sub_str,
            (_, true) => base_module.to_string(),
            _ => format!("{base_module}.{sub_str}"),
        };
        let alias = rename_at
            .and_then(|at| part.get(at + 1))
            .filter(|t| t.kind == Kind::Ident)
            .map_or_else(|| name.clone(), |t| self.text(t));
        self.facts.imports.push(Import {
            line,
            module: item_module,
            name,
            alias,
            kind: IMPORT_MODULE,
        });
    }

    fn top_level_assign(&self, toks: &[Token]) -> Option<usize> {
        let mut depth = 0;
        for (index, token) in toks.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" => depth += 1,
                b")" | b"]" | b"}" => depth -= 1,
                b"=" | b":=" | b"+=" | b"-=" | b".=" | b"|=" | b"||=" | b"&&=" | b"??=" | b"<-" | b"*=" | b"/=" | b"%="
                | b"&=" | b"^=" | b"<<=" | b">>="
                    if depth == 0 =>
                {
                    return Some(index)
                }
                _ => {}
            }
        }
        None
    }

    fn class_head(&self, toks: &[Token]) -> Option<(String, Vec<String>)> {
        let mut index = 0;
        while let Some(word) = self.word_at(toks, index) {
            if self.lang.classes.contains(&word.as_str()) {
                break;
            }
            if !MODIFIERS.contains(&word.as_str()) {
                return None;
            }
            index += 1;
        }
        let keyword = self.word_at(toks, index)?;
        if !self.lang.classes.contains(&keyword.as_str()) {
            return None;
        }
        let mut at = index + 1;
        if keyword == "impl" {
            // `impl<T> Trait for Type`: the type owns the methods.
            if let Some(after) = toks.iter().position(|t| t.kind == Kind::Ident && self.raw(t) == b"for") {
                at = after + 1;
            } else if toks.get(at).is_some_and(|t| self.is_op(t, b"<")) {
                at = toks.iter().position(|t| self.is_op(t, b">"))? + 1;
            }
        }
        let name = self.word_at(toks, at)?;
        let mut bases = Vec::new();
        let mut inherits = false;
        for (offset, token) in toks[at + 1..].iter().enumerate() {
            if token.kind == Kind::Op && matches!(self.raw(token), b"(" | b":" | b"<") {
                inherits = true;
            } else if token.kind == Kind::Ident {
                let word = self.text(token);
                if matches!(word.as_str(), "extends" | "implements" | "with") {
                    inherits = true;
                } else if inherits && !toks.get(at + 2 + offset).is_some_and(|t| self.is_op(t, b".")) {
                    bases.push(word);
                }
            }
        }
        Some((name, bases))
    }

    /// The parameter list between two brackets.
    fn params(&self, toks: &[Token]) -> Vec<String> {
        let mut out = Vec::new();
        let mut depth = 0;
        let mut start = 0;
        for (index, token) in toks.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" | b"<" => depth += 1,
                    b")" | b"]" | b"}" | b">" => depth -= 1,
                    b"," if depth == 0 => {
                        out.extend(self.param(&toks[start..index]));
                        start = index + 1;
                    }
                    _ => {}
                }
            }
        }
        out.extend(self.param(&toks[start..]));
        out
    }

    fn param(&self, toks: &[Token]) -> Option<String> {
        let mut depth = 0;
        let mut end = toks.len();
        let mut typed = false;
        for (index, token) in toks.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"<" => depth += 1,
                b")" | b"]" | b">" => depth -= 1,
                b"=" if depth == 0 => {
                    end = index;
                    break;
                }
                b":" if depth == 0 => {
                    end = index;
                    typed = true;
                    break;
                }
                _ => {}
            }
        }
        let names = toks[..end].iter().filter(|token| token.kind == Kind::Ident);
        let word = if typed || self.lang.type_first { names.last() } else { names.into_iter().find(|token| !matches!(self.raw(token), b"mut" | b"ref" | b"var" | b"val" | b"in" | b"out")) };
        word.map(|token| self.text(token)).filter(|name| name != "void")
    }

    fn name_chain(&self, toks: &[Token], mut at: usize) -> (String, String, usize) {
        let mut parts = Vec::new();
        while let Some(word) = self.word_at(toks, at) {
            parts.push(word);
            at += 1;
            let joined = toks.get(at).is_some_and(|t| t.kind == Kind::Op && matches!(self.raw(t), b"." | b"::" | b":"));
            let next_is_word = self.word_at(toks, at + 1).is_some();
            if joined && next_is_word && self.lang.tag != "SCALA" {
                at += 1;
            } else {
                break;
            }
        }
        let name = parts.pop().unwrap_or_default();
        let owner = parts.into_iter().filter(|part| !matches!(part.as_str(), "exports" | "module" | "prototype" | "self")).last();
        (name, owner.unwrap_or_default(), at)
    }

    /// The bracketed group opening at `open`, as (first inner, closing index).
    fn group_at(&self, toks: &[Token], open: usize) -> Option<(usize, usize)> {
        let mut depth = 0;
        for (index, token) in toks.iter().enumerate().skip(open) {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" => depth += 1,
                b")" | b"]" | b"}" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((open + 1, index));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn skip_generics(&self, toks: &[Token], mut at: usize) -> usize {
        if toks.get(at).is_some_and(|token| self.is_op(token, b"<")) {
            let mut depth = 0;
            while let Some(token) = toks.get(at) {
                if self.is_op(token, b"<") {
                    depth += 1;
                } else if self.is_op(token, b">") {
                    depth -= 1;
                    if depth == 0 {
                        return at + 1;
                    }
                }
                at += 1;
            }
        }
        at
    }

    fn keyword_head(&self, toks: &[Token], keyword: usize) -> Option<Head> {
        let mut head = Head::default();
        let mut at = keyword + 1;
        if toks.get(at).is_some_and(|token| self.is_op(token, b"*")) {
            at += 1;
        }
        // Go: `func (s *Server) Handle(x string)`.
        if self.lang.tag == "GO" && toks.get(at).is_some_and(|token| self.is_op(token, b"(")) {
            let (inner, close) = self.group_at(toks, at)?;
            let names: Vec<String> = toks[inner..close].iter().filter(|t| t.kind == Kind::Ident).map(|t| self.text(t)).collect();
            if let (Some(first), Some(last)) = (names.first(), names.last()) {
                head.receiver = first.clone();
                head.owner = last.clone();
            }
            at = close + 1;
        }
        // MATLAB: `function [a, b] = f(c)`.
        if self.lang.tag == "M" {
            if let Some(equal) = toks.iter().position(|token| self.is_op(token, b"=")) {
                at = equal + 1;
            }
        }
        if toks.get(at).is_some_and(|token| token.kind == Kind::Ident) {
            let (name, owner, next) = self.name_chain(toks, at);
            head.name = name;
            if !owner.is_empty() {
                head.owner = owner;
            }
            at = next;
        }
        at = self.skip_generics(toks, at);
        if toks.get(at).is_some_and(|token| self.is_op(token, b"(") || (self.lang.tag == "TCL" && self.is_op(token, b"{"))) {
            let (inner, close) = self.group_at(toks, at)?;
            head.params = self.params(&toks[inner..close]);
            at = close + 1;
        }
        if let Some(equal) = toks[at.min(toks.len())..].iter().position(|token| self.is_op(token, b"=")) {
            head.body = Some(at + equal + 1);
        }
        Some(head)
    }

    /// `class` as the value of an assignment: the name it spells, if any,
    /// and its bases. `= class extends B {` is anonymous — the CST gives that
    /// class no name of its own, so neither does the lexer.
    fn assigned_class_head(&self, toks: &[Token]) -> Option<(String, Vec<String>)> {
        // The class must open the value: `const x = { class: 1 }` has the word
        // inside an object literal and names no class.
        let at = toks.iter().position(|t| t.kind == Kind::Ident && self.raw(t) == b"class")?;
        if at != 0 {
            return None;
        }
        let mut name = String::new();
        let mut bases = Vec::new();
        let mut next = at + 1;
        match self.word_at(toks, next) {
            Some(word) if word != "extends" => {
                name = word;
                next += 1;
            }
            _ => {}
        }
        if self.word_at(toks, next).as_deref() == Some("extends") {
            let (base, _, _) = self.name_chain(toks, next + 1);
            if !base.is_empty() {
                bases.push(base);
            }
        }
        Some((name, bases))
    }

    /// `name = function (a) {`, `name = (a) => ...`, `name <- function(a)`.
    fn assigned_head(&self, toks: &[Token]) -> Option<Head> {
        // `type Fn = (x: number) => void` is an assignment shape with a type on
        // the right, not a function: the CST writes no symbol for it.
        if matches!(self.lang.tag, "TS" | "TSX") && self.type_declaration(toks) {
            return None;
        }
        let equal = self.top_level_assign(toks)?;
        // Only the chain touching the `=`: `export const f`, `const f: T` name `f`.
        let mut start = equal.checked_sub(1).filter(|at| toks[*at].kind == Kind::Ident)?;
        while start >= 2
            && toks[start - 2].kind == Kind::Ident
            && toks[start - 1].kind == Kind::Op
            && matches!(self.raw(&toks[start - 1]), b"." | b":")
        {
            start -= 2;
        }
        let annotated = self.lang.tag != "LUA" && toks[start..equal].iter().any(|token| self.is_op(token, b":"));
        let end = if annotated { start + 1 } else { equal };
        let (name, owner, _) = self.name_chain(&toks[..end], start);
        let rhs = &toks[equal + 1..];
        let mut skip = 0;
        while self.word_at(rhs, skip).is_some_and(|word| word == "async" || word == "local") {
            skip += 1;
        }
        let first = self.word_at(rhs, skip);
        if first.as_deref().is_some_and(|word| self.lang.defs.contains(&word) || word == "lambda") {
            let mut head = if first.as_deref() == Some("lambda") {
                let colon = rhs.iter().position(|token| self.is_op(token, b":"))?;
                let params = self.params(&rhs[skip + 1..colon]);
                Head { params, body: Some(equal + 1 + colon + 1), ..Head::default() }
            } else {
                let mut head = self.keyword_head(rhs, skip)?;
                head.body = head.body.map(|body| body + equal + 1);
                head
            };
            // `module.exports = function callBindBasic(args) {`: the function
            // names itself, and the CST keeps that name, so overriding it with
            // the assignment target made the two records disagree about a
            // symbol both of them saw. The target's name is only a fallback for
            // a function expression that has none.
            if head.name.is_empty() {
                head.name = name;
                head.owner = owner;
            }
            return Some(head);
        }
        let arrow = rhs.iter().position(|token| self.is_op(token, b"=>"))?;
        let mut depth = 0;
        for token in &rhs[..arrow] {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    _ => {}
                }
            }
        }
        if depth != 0 {
            return None;
        }
        let params = self.arrow_params(&rhs[skip..arrow]);
        let body = equal + 1 + arrow + 1;
        Some(Head { name, owner, params, body: Some(body), ..Head::default() })
    }

    fn arrow_params(&self, toks: &[Token]) -> Vec<String> {
        match toks.first() {
            Some(token) if self.is_op(token, b"(") => {
                let Some((inner, close)) = self.group_at(toks, 0) else { return Vec::new() };
                self.params(&toks[inner..close])
            }
            Some(token) if token.kind == Kind::Ident => vec![self.text(token)],
            _ => Vec::new(),
        }
    }

    fn lambda_head(&self, stmt: &[Token]) -> Head {
        let last = stmt.len() - 1;
        if self.is_op(&stmt[last], b"=>") || self.is_op(&stmt[last], b"->") {
            let mut start = last;
            if last > 0 && self.is_op(&stmt[last - 1], b")") {
                let mut depth = 0;
                for index in (0..last).rev() {
                    if self.is_op(&stmt[index], b")") {
                        depth += 1;
                    } else if self.is_op(&stmt[index], b"(") {
                        depth -= 1;
                        if depth == 0 {
                            start = index;
                            break;
                        }
                    }
                }
            } else if last > 0 {
                start = last - 1;
            }
            return Head { params: self.arrow_params(&stmt[start..last]), ..Head::default() };
        }
        let keyword = stmt.iter().rposition(|token| {
            token.kind == Kind::Ident && self.lang.defs.iter().any(|word| word.as_bytes() == self.raw(token))
        });
        keyword.and_then(|at| self.keyword_head(stmt, at)).unwrap_or_default()
    }

    /// `ret name(args) ... {` where no keyword says it is a function.
    fn c_style_head(&self, toks: &[Token]) -> Option<Head> {
        if self.top_level_assign(toks).is_some_and(|equal| !toks[..equal].iter().any(|t| self.is_op(t, b"("))) {
            return None;
        }
        // `- (void)f:(char *)c`.
        if self.lang.tag == "OBJC" && toks.first().is_some_and(|t| self.is_op(t, b"-") || self.is_op(t, b"+")) {
            let (_, close) = self.group_at(toks, 1)?;
            let name = self.word_at(toks, close + 1)?;
            let mut params = Vec::new();
            let mut at = close + 2;
            while at < toks.len() {
                if self.is_op(&toks[at], b":") {
                    if let Some((_, end)) = self.group_at(toks, at + 1) {
                        params.extend(self.word_at(toks, end + 1));
                        at = end + 1;
                    }
                }
                at += 1;
            }
            return Some(Head { name, params, ..Head::default() });
        }
        let mut depth = 0;
        let mut groups: Vec<usize> = Vec::new();
        for (index, token) in toks.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" => {
                    if depth == 0 {
                        groups.push(index);
                    }
                    depth += 1;
                }
                b")" => depth -= 1,
                b":" if depth == 0 && !groups.is_empty() => break,
                _ => {}
            }
        }
        // `pub(super) fn parse_with(..)`: the visibility group is also a group
        // at depth zero, and taking it as the parameter list named the function
        // `pub` — or, once `pub` was refused, nothing at all. The head's group
        // is the first one that has a usable name in front of it.
        let open = *groups.iter().find(|&&open| {
            open > 0
                && self
                    .word_at(toks, open - 1)
                    .is_some_and(|name| !CONTROL.contains(&name.as_str()) && !NOT_A_NAME.contains(&name.as_str()))
        })?;
        let name_at = open - 1;
        let name = self.word_at(toks, name_at)?;
        if CONTROL.contains(&name.as_str()) || NOT_A_NAME.contains(&name.as_str()) {
            return None;
        }
        if self.head_word(toks).is_some_and(|word| CONTROL.contains(&word.as_str())) {
            return None;
        }
        let mut chain_start = name_at;
        while chain_start >= 2
            && toks[chain_start - 1].kind == Kind::Op
            && matches!(self.raw(&toks[chain_start - 1]), b"::" | b".")
            && toks[chain_start - 2].kind == Kind::Ident
        {
            chain_start -= 2;
        }
        let (_, owner, _) = self.name_chain(toks, chain_start);
        let (inner, close) = self.group_at(toks, open)?;
        let params = self.params(&toks[inner..close]);
        Some(Head { name, owner, params, ..Head::default() })
    }

    fn function_head(&self, toks: &[Token], at_block: bool) -> Option<Head> {
        let mut index = 0;
        while let Some(word) = self.word_at(toks, index) {
            if self.lang.defs.contains(&word.as_str()) {
                return self.keyword_head(toks, index);
            }
            if !MODIFIERS.contains(&word.as_str()) {
                break;
            }
            index += 1;
        }
        if let Some(head) = self.assigned_head(toks) {
            return Some(head);
        }
        (at_block && self.uses_braces()).then(|| self.c_style_head(toks)).flatten()
    }

    fn simple_statement(&mut self, stmt: &[Token]) {
        if stmt.is_empty() {
            return;
        }
        let Some(first) = stmt.first() else { return };
        let line = first.line;
        if let Some(head) = self.assigned_head(stmt).filter(|head| head.body.is_some()) {
            self.inline_function(head, stmt);
            return;
        }
        self.statement(stmt);
        let _ = line;
    }

    fn statement(&mut self, stmt: &[Token]) {
        let Some(first) = stmt.first() else { return };
        // `export { a }` and `export * from …` name what leaves the module and
        // carry no value: the lexer read `export` as a name and the braces as a
        // merge. `export default X` is the value of X, not a merge with the
        // keyword in front of it.
        if self.lang.imports == crate::langs::Imports::Js
            && self.word_at(stmt, 0).as_deref() == Some("export")
        {
            match stmt.get(1) {
                Some(token) if self.is_op(token, b"{") || self.is_op(token, b"*") => return,
                _ if self.word_at(stmt, 1).as_deref() == Some("default") => {
                    self.expression_event("x", &stmt[2..]);
                    return;
                }
                _ => {}
            }
        }
        if self.import_of(stmt) {
            return;
        }
        // TypeScript declares types, and a type declaration writes no runtime
        // fact: `type Id = string | number` is not an assignment of `Id`, an
        // `enum` header is not a value, and `namespace NS` names no variable.
        // The CST writes nothing for any of them, so reading them as statements
        // only invented facts the other reader never had.
        if matches!(self.lang.tag, "TS" | "TSX") && self.type_declaration(stmt) {
            return;
        }
        // A type body writes no facts: its statements are members, not values.
        if self.frames.iter().any(|frame| frame.type_only) {
            return;
        }
        // `class A { error; }` is a field declaration with no value, not an
        // expression: the CST writes no event for it, and the lexer's bare
        // `x n:error` had no counterpart in every SDK error class.
        if matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX")
            && self.top().kind == FrameKind::Class
            && stmt.len() == 1
            && stmt[0].kind == Kind::Ident
        {
            return;
        }
        // A class member declares its type after a colon, and the type is
        // erased at runtime: `readonly id: number = 1` binds `id`,
        // `name!: string` binds nothing, and `abstract area(): number` is a
        // signature. Read as one value, the annotation ended up in the merge
        // and the CST never wrote it.
        if matches!(self.lang.tag, "TS" | "TSX") && self.top().kind == FrameKind::Class {
            // `set x(v: T);` and `abstract area(): number` are member signatures
            // in a declaration: they bind nothing at runtime. Read as a value
            // the setter became a call of the property name, which the CST never
            // writes (21 losses in the .d.ts files). The colon cases already end
            // at the ), this catches the ones whose only colon is inside the
            // parameter list.
            if stmt.last().is_some_and(|token| self.is_op(token, b")"))
                && self.top_level_assign(stmt).is_none()
            {
                return;
            }
            if let Some(cut) = self.member_type_cut(stmt) {
                let head: Vec<Token> = stmt[..cut].to_vec();
                if head.last().is_some_and(|token| self.is_op(token, b")")) {
                    return;
                }
                let skip = head
                    .iter()
                    .take_while(|token| token.kind == Kind::Ident && MODIFIERS.contains(&self.text(token).as_str()))
                    .count();
                let Some(equal) = self.top_level_assign(stmt) else { return };
                let mut combined: Vec<Token> = head[skip..].to_vec();
                combined.extend_from_slice(&stmt[equal..]);
                let _ = cut;
                self.statement(&combined);
                return;
            }
        }
        let word = self.word_at(stmt, 0);
        if matches!(word.as_deref(), Some("return")) {
            self.expression_event("r", &stmt[1..]);
            return;
        }
        let line = first.line;
        // `X = class Y { … }` and `module.exports = class …`: a class
        // expression is the definition an assigned function is — the walk
        // opens the class body as its own symbol and the CST writes no event.
        // Read as a value, `class`, the class name and `extends B` became
        // three names the CST never wrote.
        if matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX") {
            if let Some(equal) = self.top_level_assign(stmt) {
                if let Some((name, bases)) = self.assigned_class_head(&stmt[equal + 1..]) {
                    self.last_line = stmt[stmt.len() - 1].line;
                    self.push_class(name, bases, first.col);
                    return;
                }
            }
        }
        if let Some(equal) = self.top_level_assign(stmt) {
            let augmented = self.raw(&stmt[equal]) != b"=" && !matches!(self.raw(&stmt[equal]), b":=" | b"<-");
            self.note_class(&stmt[..equal], &stmt[equal + 1..]);
            self.assignment(&stmt[..equal], &stmt[equal + 1..], line, augmented);
        } else {
            self.expression_event("x", stmt);
        }
    }

    /// `repo = new Repo()` and `repo = Repo()` say what `repo.find` means.
    fn note_class(&mut self, targets: &[Token], value: &[Token]) {
        let mut at = 0;
        let explicit = self.word_at(value, 0).as_deref() == Some("new");
        if explicit {
            at = 1;
        }
        let Some(class) = self.word_at(value, at) else { return };
        let (_, owner, next) = self.name_chain(value, at);
        let called = value.get(next).is_some_and(|token| self.is_op(token, b"("));
        let capital = class.starts_with(|ch: char| ch.is_uppercase());
        let name = if owner.is_empty() { class } else { self.name_chain(value, at).0 };
        if called && (explicit || capital) && name.starts_with(|ch: char| ch.is_uppercase()) {
            let target = self.target_path(targets);
            if let Some(target) = target {
                self.facts.vars.push((target, name));
            }
        }
    }

    /// `type Id = ...`, `interface User { ... }`, `namespace NS { ... }`,
    /// `enum E { ... }` and their `export` / `declare` / `abstract` / `const`
    /// prefixed spellings: a declaration of a type, not a binding.
    fn type_declaration(&self, stmt: &[Token]) -> bool {
        let mut at = 0;
        while let Some(word) = self.word_at(stmt, at) {
            match word.as_str() {
                "export" | "declare" | "default" | "abstract" => at += 1,
                "const" => {
                    // `const enum E {}` is a type; `const x = 1` is not.
                    if self.word_at(stmt, at + 1).as_deref() != Some("enum") {
                        break;
                    }
                    at += 1;
                }
                _ => break,
            }
        }
        matches!(
            self.word_at(stmt, at).as_deref(),
            Some("type") | Some("interface") | Some("namespace") | Some("module") | Some("enum")
        ) && self.word_at(stmt, at + 1).is_some()
    }

    /// Where a class member's type annotation starts, if it has one: the first
    /// colon outside brackets and before any `=`.
    fn member_type_cut(&self, stmt: &[Token]) -> Option<usize> {
        let mut depth = 0i32;
        for (index, token) in stmt.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                // `<` counts: `connect<TOpaque = null>(options: T): U` has an
                // `=` and a `:` inside its generic parameter list, and a member
                // signature is not an assignment of `TOpaque`.
                b"(" | b"[" | b"{" | b"<" => depth += 1,
                b")" | b"]" | b"}" | b">" => depth -= 1,
                b"=" | b"=>" if depth == 0 => return None,
                b":" if depth == 0 => return (index > 0).then_some(index),
                _ => {}
            }
        }
        None
    }

    fn target_path(&self, toks: &[Token]) -> Option<String> {
        let mut end = toks.len();
        let mut depth = 0;
        for (index, token) in toks.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"<" => depth += 1,
                    b")" | b"]" | b">" => depth -= 1,
                    b":" if depth == 0 => {
                        end = index;
                        break;
                    }
                    _ => {}
                }
            }
        }
        let toks = &toks[..end];
        let mut at = toks.len();
        if toks.last().is_some_and(|token| self.is_op(token, b"]")) {
            at = toks.iter().position(|token| self.is_op(token, b"["))?;
        }
        let mut start = at;
        while start > 0 {
            if self.is_op(&toks[start - 1], b")") && start < toks.len() && self.is_op(&toks[start], b".") {
                // `(object).__proto__ = v`: the receiver is parenthesised, so the
                // path continues inside the group. Stopping at the `)` named the
                // target `.__proto__` and the assignment lost its receiver.
                let mut depth = 0usize;
                let mut open = None;
                let mut index = start - 1;
                loop {
                    let token = &toks[index];
                    if self.is_op(token, b")") {
                        depth += 1;
                    } else if self.is_op(token, b"(") {
                        depth -= 1;
                        if depth == 0 {
                            open = Some(index);
                            break;
                        }
                    }
                    if index == 0 {
                        break;
                    }
                    index -= 1;
                }
                match open {
                    Some(index) => {
                        // Only a parenthesised primary is a receiver. When the
                        // group follows an identifier or another group it is a
                        // call's argument list, and stepping into it merged the
                        // argument into the path.
                        let is_call = index > 0
                            && (toks[index - 1].kind == Kind::Ident
                                || self.is_op(&toks[index - 1], b")")
                                || self.is_op(&toks[index - 1], b"]"));
                        if is_call {
                            break;
                        }
                        start = index;
                    }
                    None => break,
                }
                continue;
            }
            let token = &toks[start - 1];
            let joins = token.kind == Kind::Op && matches!(self.raw(token), b"." | b"->" | b"::");
            if token.kind == Kind::Ident && (start == at || self.raw(&toks[start]) != b".") && start != at {
                if !self.is_join(&toks[start]) {
                    break;
                }
            }
            if token.kind != Kind::Ident && !joins {
                break;
            }
            start -= 1;
        }
        let mut path = String::new();
        for token in &toks[start..at] {
            if token.kind == Kind::Ident {
                let word = self.text(token);
                let receiver = self.top().receiver.as_str();
                if word == "this" || (!receiver.is_empty() && word == receiver) {
                    path.push_str("self");
                } else {
                    path.push_str(&word);
                }
            } else if token.kind == Kind::Op && matches!(self.raw(token), b"(" | b")") {
                // A parenthesised receiver contributes its contents, not a dot.
            } else {
                path.push('.');
            }
        }
        (!path.is_empty()).then_some(path)
    }

    fn is_join(&self, token: &Token) -> bool {
        token.kind == Kind::Op && matches!(self.raw(token), b"." | b"->" | b"::")
    }

    fn unpack_targets(&self, toks: &[Token]) -> Option<Vec<String>> {
        let mut inner = toks;
        while let Some(word) = self.word_at(inner, 0) {
            if !matches!(word.as_str(), "const" | "let" | "var" | "my" | "our" | "local" | "val" | "mut") {
                break;
            }
            inner = &inner[1..];
        }
        let wrapped = inner.first().is_some_and(|t| self.is_op(t, b"(") || self.is_op(t, b"[") || self.is_op(t, b"{"));
        if wrapped {
            let (start, close) = self.group_at(inner, 0)?;
            if close + 1 == inner.len() {
                inner = &inner[start..close];
            } else {
                return None;
            }
        } else if !inner.iter().any(|token| self.is_op(token, b",")) {
            return None;
        }
        let mut targets = Vec::new();
        let mut depth = 0;
        let mut start = 0;
        for (index, token) in inner.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" => depth += 1,
                    b")" | b"]" => depth -= 1,
                    b"," if depth == 0 => {
                        targets.extend(self.unpack_one(&inner[start..index]));
                        start = index + 1;
                    }
                    _ => {}
                }
            }
        }
        targets.extend(self.unpack_one(&inner[start..]));
        Some(targets)
    }

    fn unpack_one(&self, part: &[Token]) -> Option<String> {
        // `{ a: b }` binds `b`; `a, *b` binds `b`.
        match part.iter().rposition(|token| self.is_op(token, b":")) {
            Some(colon) => self.target_path(&part[colon + 1..]),
            None => self.target_path(part),
        }
    }

    fn assignment(&mut self, targets: &[Token], value: &[Token], line: u32, augmented: bool) {
        let unpacked = self.unpack_targets(targets);
        let weak = augmented || targets.last().is_some_and(|token| self.is_op(token, b"]"));
        let names = match unpacked {
            Some(names) => names,
            None => self.target_path(targets).into_iter().collect(),
        };
        let expr = self.values(value);
        if names.is_empty() {
            if expr != "0" {
                self.event(&format!("[\"x\",{line},{expr}]"));
            }
            return;
        }
        let mut text = format!("[\"a\",{line},[");
        for (index, name) in names.iter().enumerate() {
            if index > 0 {
                text.push(',');
            }
            push_string(&mut text, name);
        }
        text.push_str(&format!("],{expr},{}]", u8::from(weak)));
        self.event(&text);
    }
}
