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
/// The words Kotlin writes in front of a member. `lateinit`, `suspend`,
/// `vararg` and `reified` are not in [`MODIFIERS`], which is the C-family list.
const KOTLIN_MODIFIERS: [&str; 24] = [
    "public", "private", "protected", "internal", "abstract", "final", "open", "sealed", "data", "value", "inner",
    "override", "lateinit", "const", "suspend", "inline", "infix", "operator", "tailrec", "external", "annotation",
    "vararg", "expect", "actual",
];
/// A word Java writes in front of a member and binds nothing.
fn java_modifier(word: &str) -> bool {
    matches!(
        word,
        "public" | "private" | "protected" | "static" | "final" | "abstract" | "native" | "synchronized"
            | "transient" | "volatile" | "strictfp" | "default"
    )
}

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
/// Words that open a statement in Go. A brace whose first inner word is one of
/// these is a block: `if x == nil { return }`, `for { break }`.
const GO_STATEMENT_OPENERS: [&str; 16] = [
    "break", "case", "const", "continue", "default", "defer", "fallthrough", "for", "go", "goto", "if", "return",
    "select", "switch", "type", "var",
];

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

enum SavedStmt {
    Expr(Vec<Token>, Vec<u8>),
    /// `window.onerror = function(...) { ... }.bind()`: an assigned function
    /// expression whose enclosing statement may continue past `}`.
    Assigned(Vec<Token>, Vec<u8>),
}

struct Frame {
    kind: FrameKind,
    symbol: usize,
    class: String,
    receiver: String,
    /// Column of the statement that opened the frame (indent blocks).
    col: u32,
    /// A statement interrupted by this block: `app.get('/x', (req) => {`.
    saved: Option<SavedStmt>,
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
    /// Whether `name`/`owner` came from an assignment target rather than the
    /// function's own identifier.
    assigned_name: bool,
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
    /// The line of the block that just closed, so a bare `name { … }` opening
    /// a block on that same line continues the expression above it
    /// (`} builder {`).
    closed_line: u32,
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
    /// The `{` that just opened was a conditional type's object type
    /// (`typeof X extends { … } ? T : U`), so the `}` closes a type, and the
    /// `? T : U` that follows it is a type too.
    conditional_type_open: bool,
    /// The statement after such a `}` writes no fact: it is the type's tail.
    suppress_event: bool,
    /// The frame a `typedef struct { … } Name;` body opened, as the stack depth
    /// that frame has. The `}` closes the body and the name after it is the
    /// declarator the C++ grammar files inside the declaration on the header's
    /// line, so the tail writes no event of its own.
    typedef_block: Option<usize>,
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
        closed_line: 0,
        rearmed: false,
        pending: None,
        last_line: 1,
        ahead: VecDeque::new(),
        declaring: false,
        conditional_type_open: false,
        suppress_event: false,
        typedef_block: None,
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
        if !matches!(self.lang.tag, "TS" | "TSX") {
            return false;
        }
        let mut depth = 0i32;
        for token in &self.stmt {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"<" | b"<=" => depth += 1,
                // An arrow ends the angle context it is in: `(e: A) => B` is a
                // function type, and its `=>` never closes a type-argument list
                // opened earlier on the line. Counting it as a close made the
                // depth negative, so the `<` of a return type's own list looked
                // unmatched and a declarator split at its comma -
                // `const t = (f: (e: A) => B): C<D, E> => 1` became two
                // statements where the CST reads one arrow with an annotation.
                b"->" | b"=>" => depth = 0,
                b">" | b">=" => depth -= 1,
                _ => {}
            }
        }
        depth > 0
    }

    /// Whether the statement has an unmatched `<` that opens a type-argument
    /// list rather than a comparison. `getRandomValues<` holds the angle against
    /// the name; a less-than is written with space on both sides, and `<=` is
    /// arithmetic on any reading.
    fn open_type_arguments(&self) -> bool {
        if !matches!(self.lang.tag, "TS" | "TSX") {
            return false;
        }
        let mut depth = 0i32;
        let mut open: Option<usize> = None;
        for (index, token) in self.stmt.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"<" => {
                    if depth == 0 {
                        open = Some(index);
                    }
                    depth += 1;
                }
                b">" => depth -= 1,
                b">>" => depth -= 2,
                b">>>" => depth -= 3,
                _ => {}
            }
        }
        if depth <= 0 {
            return false;
        }
        let Some(at) = open else { return false };
        at > 0 && self.stmt[at - 1].kind == Kind::Ident && self.stmt[at - 1].end == self.stmt[at].start
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
        self.declaring = false;
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

    fn next_line_start(&self, end: usize) -> Option<usize> {
        let mut at = end;
        let mut saw_newline = false;
        let has_block = self.lang.block_comments.iter().any(|&(open, _)| open == "/*");
        loop {
            while at < self.src.len() && matches!(self.src[at], b' ' | b'\t' | b'\r') {
                at += 1;
            }
            if self.src.get(at) == Some(&b'\n') {
                saw_newline = true;
                at += 1;
                continue;
            }
            if self.src.get(at..at + 2) == Some(b"//") {
                while at < self.src.len() && self.src[at] != b'\n' {
                    at += 1;
                }
                continue;
            }
            if has_block && self.src.get(at..at + 2) == Some(b"/*") {
                at += 2;
                while at < self.src.len() && self.src.get(at..at + 2) != Some(b"*/") {
                    if self.src[at] == b'\n' {
                        saw_newline = true;
                    }
                    at += 1;
                }
                if self.src.get(at..at + 2) == Some(b"*/") {
                    at += 2;
                }
                continue;
            }
            break;
        }
        saw_newline.then_some(at)
    }

    /// Whether the next bytes after `end` are a member access that begins a
    /// new line, ignoring spaces and tabs: `.name` or `?.name`.
    fn next_is_member_access(&self, end: usize) -> bool {
        let Some(at) = self.next_line_start(end) else { return false };
        matches!(self.src.get(at), Some(b'.')) || self.src.get(at..at + 2) == Some(b"?.")
    }

    /// Whether the next line begins with `?`, `:` or a binary operator — the
    /// continuation of a multi-line expression, never the start of a statement.
    fn next_starts_with_ternary(&self, end: usize) -> bool {
        let Some(at) = self.next_line_start(end) else { return false };
        matches!(self.src.get(at), Some(b'?') | Some(b':') | Some(b'>'))
            || matches!(
                self.src.get(at..at + 2),
                Some(
                    b"||" | b"&&" | b"??" | b"==" | b"!=" | b"<=" | b"<<" | b"**"
                        | b"* " | b"+ " | b"- " | b"/ " | b"% " | b"| " | b"& " | b"^ " | b"< "
                )
            )
    }

    /// Whether the next line begins with `{`, the body of a function expression
    /// whose head was written on the line above.
    fn next_starts_with_brace(&self, end: usize) -> bool {
        let Some(at) = self.next_line_start(end) else { return false };
        self.src.get(at) == Some(&b'{')
    }

    /// Whether the next line begins with `word` as a standalone identifier.
    fn next_starts_with_word(&self, end: usize, word: &[u8]) -> bool {
        let Some(at) = self.next_line_start(end) else { return false };
        self.src.get(at..at + word.len()) == Some(word)
            && !self
                .src
                .get(at + word.len())
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    }

    /// How many `<` the open statement has not closed outside `(...)`/`[...]`/`{...}`.
    /// A type-parameter or type-argument list is the only reason TS leaves one
    /// open across a line; ignoring grouped `<`/`>` keeps default-parameter
    /// comparisons like `function f(x = a < b) {` from looking like a type list.
    fn angle_depth(&self) -> i32 {
        let mut depth = 0;
        let mut group = 0i32;
        for token in &self.stmt {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" => group += 1,
                b")" | b"]" | b"}" if group > 0 => group -= 1,
                b"<" if group == 0 => depth += 1,
                b">" if group == 0 => depth -= 1,
                b">>" if group == 0 => depth -= 2,
                _ => {}
            }
        }
        depth
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
            // `getRandomValues<` newline `T extends` newline `| Int8Array,`
            // newline `>(buffer: T): T;` is one member signature: the
            // type-parameter list spans the breaks and ends at its own `>`.
            // Ended at the first break, each line became an expression the CST
            // never writes - the first one an `x` merge of the name, `T` and
            // `extends`.
            if self.open_type_arguments() {
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
            // `typeof globalThis extends` newline `{ onmessage: any; … } ? T : U`:
            // the object type of a conditional type opens on the next line, and
            // ending the statement here made that brace a block whose members
            // became statements the CST never writes.
            if matches!(self.lang.tag, "TS" | "TSX")
                && last.kind == Kind::Ident
                && self.raw(last) == b"extends"
                && self.stmt.iter().any(|token| token.kind == Kind::Ident && self.raw(token) == b"typeof")
                && self.next_starts_with_brace(last.end as usize)
            {
                return;
            }
            if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                && self.is_op(&last, b")")
                && self.stmt.iter().any(|token| {
                    token.kind == Kind::Ident && self.raw(token) == b"function"
                })
                && self.next_starts_with_brace(last.end as usize)
            {
                return;
            }
            // `class X(...) :` newline `Super(...)` or `fun f():` newline `T`:
            // a heritage or return-type clause cannot end at the colon, so the
            // newline is inside the declaration. Also `fun f()` newline `= e`:
            // a leading `=` continues the declaration rather than starting a
            // statement.
            if self.lang.tag == "KT"
                && ((self.is_op(last, b":")
                    && self.stmt.iter().any(|token| {
                        token.kind == Kind::Ident
                            && matches!(self.raw(token), b"class" | b"interface" | b"fun" | b"val" | b"var")
                    }))
                    || self.next_line_start(last.end as usize).is_some_and(|at| {
                        self.src.get(at) == Some(&b'=') && self.src.get(at + 1) != Some(&b'=')
                    }))
            {
                return;
            }
            // `interface A extends B` newline `{` or `interface A<T>` newline
            // `extends B`: the header of a type declaration spans the line
            // break; ending here leaves `{` in `<module>` and turns its type
            // members into statements the CST never writes.
            if matches!(self.lang.tag, "TS" | "TSX")
                && self.type_declaration(&self.stmt)
                && (self.next_starts_with_brace(last.end as usize)
                    || self.next_starts_with_word(last.end as usize, b"extends")
                    // `export interface X<T> extends` ends the line with the
                    // keyword itself and the base type is on the next line. A
                    // heritage clause cannot end there, so the newline is
                    // inside the header: ended here, the continuation read as a
                    // value and the body it opens wrote type members as
                    // statements the CST never writes.
                    || (last.kind == Kind::Ident && self.raw(last) == b"extends")
                    // `export interface encodeKeys<` newline `  S extends Top & {…},`
                    // newline `> extends`: the type-parameter list of a type
                    // declaration spans lines, and a fragment of it is not a
                    // statement — read as one it wrote a value the CST never
                    // writes.
                    || self.angle_depth() > 0)
            {
                return;
            }
        }
        // `x = a ? // a comment` newline `b : c`: a trailing `?` in JS/TS
        // begins a ternary branch (or `??`), never ends a statement. Ending
        // here split the first branch out and orphaned the following `:`.
        if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && self.stmt.last().is_some_and(|last| self.is_op(last, b"?"))
        {
            return;
        }
        // `x = a ? b : // a comment` newline `c[0];`: the line ends with the
        // ternary's colon, so the next line continues the statement. Ending it
        // here gave the continuation its own event on the wrong line.
        if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && self.stmt.last().is_some_and(|last| self.is_op(last, b":"))
            && self.stmt.iter().any(|token| self.is_op(token, b"?"))
        {
            return;
        }
        if let Some(last) = self.stmt.last() {
            let continues = self.lang.operator_continues
                && last.kind == Kind::Op
                && !matches!(self.raw(last), b")" | b"]" | b"}" | b":" | b"++" | b"--" | b"=" | b"!" | b"?");
            let trailing_equal = last.kind == Kind::Op && self.raw(last) == b"=" && self.lang.blocks != Blocks::Indent;
            let trailing_nullish = self.stmt.len() >= 2
                && self.is_op(&self.stmt[self.stmt.len() - 1], b"?")
                && self.is_op(&self.stmt[self.stmt.len() - 2], b"?");
            let trailing_ternary = matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                && self.is_op(last, b"?");
            if continues || trailing_equal || trailing_nullish || trailing_ternary {
                return;
            }
        }
        self.declaring = false;
        self.end_statement();
    }

    /// Whether the statement being read is a Go control header, whose clauses
    /// a `;` separates: `for i := 0; i < n; i++ {`, `if err := f(); err != nil {`.
    fn go_header_open(&self) -> bool {
        self.lang.tag == "GO"
            && self
                .word_at(&self.stmt, 0)
                .as_deref()
                .is_some_and(|word| matches!(word, "for" | "if" | "switch" | "select"))
    }

    fn on_op(&mut self, token: Token) {
        let raw = self.raw(&token);
        if raw.len() != 1 {
            self.stmt.push(token);
            return;
        }
        // `NS_ASSUME_NONNULL_BEGIN` on its own line, then `@interface …`: an
        // Objective-C keyword never continues the statement before it, so the
        // newline in front of it ends that statement. Merged, the one-pass
        // reader wrote a single value for two lines - the macro and the whole
        // interface header - which no CST record can pair.
        if raw[0] == b'@'
            && self.lang.tag == "OBJC"
            && self.brackets.is_empty()
            && self.stmt.last().is_some_and(|last| last.line < token.line)
        {
            self.end_statement();
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
            // Go writes a control header without parentheses - `for i := 0;
            // i < n; i++ {`, `if err := f(); err != nil {` - so a `;` inside
            // one separates clauses and is not a statement end. Ending the
            // statement there wrote the init as an event of its own and left
            // the body brace to be read as a composite literal, which folded
            // every statement of the body into one value.
            b';' if self.brackets.is_empty() && !self.go_header_open() => {
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
            b',' if self.lang.tag == "RS"
                && self.brackets.is_empty()
                && self.stmt.iter().any(|token| self.is_op(token, b"=>")) =>
            {
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

    /// `macro_rules! name { \u2026 }`: the body is a matcher and a transcriber,
    /// not statements. The CST reader gets one opaque token tree here and writes
    /// no fact, so the token reader must not invent the bindings and calls inside
    /// it either — `line_first = Some(symbol);` in a macro body is not an
    /// assignment the source contains.
    fn macro_rules_head(&self) -> bool {
        self.lang.tag == "RS"
            && self.stmt.first().is_some_and(|token| self.text(token) == "macro_rules")
            && self.stmt.get(1).is_some_and(|token| self.is_op(token, b"!"))
    }

    /// `enum urlreject {` in C and C++: the body holds enumerators, which are
    /// declarations, and the CST writes one event for the whole enumerator
    /// list on the header line. Read as statements, every initialised
    /// enumerator (`REJECT_NADA = 2,`) became an assignment the CST never
    /// wrote, and the trailing comma swallowed the enumerators that follow it.
    fn c_enum_body(&self) -> bool {
        if !matches!(self.lang.tag, "C" | "CPP" | "OBJC") {
            return false;
        }
        let mut depth = 0i32;
        for (index, token) in self.stmt.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" => depth += 1,
                    b")" | b"]" => depth -= 1,
                    _ => {}
                }
            }
            if depth == 0 && self.word_at(&self.stmt, index).as_deref() == Some("enum") {
                return true;
            }
        }
        false
    }

    /// `type A interface {`, `type S struct {`: a Go type body holds member
    /// declarations, and `M()` there is a method specification, not a call.
    fn go_type_body(&self) -> bool {
        self.lang.tag == "GO"
            && self.word_at(&self.stmt, 0).as_deref() == Some("type")
            && self.stmt.iter().any(|token| {
                token.kind == Kind::Ident && matches!(self.raw(token), b"interface" | b"struct")
            })
    }

    /// `enum class Kind {` in Kotlin: the body holds entries, and an entry
    /// with a body (`A { … }`) states no value of its own.
    fn kotlin_enum_body(&self) -> bool {
        self.lang.tag == "KT"
            && self.stmt.iter().any(|token| token.kind == Kind::Ident && self.raw(token) == b"enum")
    }

    fn on_open_brace(&mut self, token: Token) {
        let opens_block = self.uses_braces() && self.is_block();
        if opens_block {
            self.settle_branch(None);
            // `interface User extends Base {` opens a body of type members:
            // `greet(who: string): string` is a signature, not a call, and the
            // CST writes nothing for any of it.
            let conditional = matches!(self.lang.tag, "TS" | "TSX") && self.conditional_type_head(&self.stmt);
            let type_body = (matches!(self.lang.tag, "TS" | "TSX")
                && (self.type_declaration(&self.stmt) || conditional || self.is_declare_function(&self.stmt)))
                || (self.lang.tag == "RS" && {
                    let index = self.skip_modifiers(&self.stmt);
                    self.word_at(&self.stmt, index).is_some_and(|w| matches!(w.as_str(), "struct" | "enum" | "union"))
                })
                || self.c_enum_body()
                || self.go_type_body()
                // `enum class Kind { A { … }, B { … } }`: an entry's body is a
                // type body too, and the entry itself (`A {`) is a member, not
                // a value. Read as a statement it wrote `x n:A`, which the CST
                // never writes (12 files of the kotlin dump).
                || self.kotlin_enum_body();
            // The header is a declaration and a type: `declare var M: typeof
            // globalThis extends`. Only the declarator is a name the CST writes;
            // `globalThis` and `extends` are type text here, and writing them
            // made the header line disagree on every conditional-type
            // declaration in the corpus. Truncate before the block saves it.
            if conditional {
                if let Some(colon) = self.stmt.iter().position(|token| self.is_op(token, b":")) {
                    // `declare var M:` — the declarator is the last name before
                    // the colon; the modifiers and the type are not values.
                    let name = self.stmt[..colon]
                        .iter()
                        .rposition(|token| token.kind == Kind::Ident)
                        .filter(|&name| name > 0);
                    if let Some(name) = name {
                        self.stmt = self.stmt[name..colon].to_vec();
                    } else {
                        self.stmt.truncate(colon);
                    }
                }
            }
            let macro_body = self.macro_rules_head();
            let before = self.frames.len();
            self.open_block();
            if (type_body || macro_body) && self.frames.len() > before {
                self.frames[before].type_only = true;
            }
            if conditional {
                self.conditional_type_open = true;
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
        self.declaring = false;
        self.end_statement();
        self.settle_branch(None);
        let conditional = std::mem::take(&mut self.conditional_type_open);
        let depth = self.frames.len();
        self.close_block();
        // `typedef struct art_s { … } art_t;`: the closing `}` ends the body and
        // `art_t` is the declarator, which the C++ grammar files inside the
        // declaration on the header's line. Written on the closing line it was
        // a name the CST never had there (roaring.h, art.h, art.c).
        if self.typedef_block == Some(depth) && self.frames.len() < depth {
            self.typedef_block = None;
            self.suppress_event = true;
        }
        // `… extends { onmessage: any } ? T : typeof undici.M;`: what follows
        // the object type is the rest of the type, and a type writes no fact.
        if conditional {
            self.suppress_event = true;
        }
    }

    /// Whether the brace just read closes a block that a postfix chain
    /// continues: `{ … }.await`, `{ … }?`, `{ … }()`.
    fn brace_continued(&mut self) -> bool {
        let mut depth = 0i32;
        for at in 0..MAX_BRACE_SCAN {
            let token = self.peek_token(at);
            if token.kind == Kind::Eof {
                return false;
            }
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(&token) {
                b"{" | b"(" | b"[" => depth += 1,
                b"}" | b")" | b"]" => {
                    depth -= 1;
                    if depth < 0 {
                        // The chain may start on the line after the brace
                        // (`}\n    .catch_unwind()`), so the line breaks between
                        // them are skipped before the operator is read.
                        let mut next_at = at + 1;
                        loop {
                            let next = self.peek_token(next_at);
                            if next.kind == Kind::Newline {
                                next_at += 1;
                                continue;
                            }
                            if next.kind == Kind::Eof {
                                return false;
                            }
                            return next.kind == Kind::Op && matches!(self.raw(&next), b"." | b"?");
                        }
                    }
                }
                _ => {}
            }
        }
        false
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
            let bars: Vec<usize> = (0..self.stmt.len())
                .filter(|at| matches!(self.raw(&self.stmt[*at]), b"|" | b"||"))
                .collect();
            let line_of = |at: usize| self.stmt.get(at).map(|token| token.line);
            let header_on_one_line = match (bars.first(), bars.last()) {
                (Some(first), Some(last)) => line_of(*first) == line_of(*last),
                _ => false,
            };
            let closure_header = self.stmt.last().is_some_and(|token| matches!(self.raw(token), b"|" | b"||"))
                || if header_on_one_line && bars.len() <= 2 {
                    // `let fold = |pick: fn(f64, f64) -> f64, start: f64| Rgba {`:
                    // the `->` is a parameter's own type, so the header closes at
                    // the bar and this brace opens the struct literal the closure
                    // returns (001452:386).
                    bars.last().is_some_and(|bar| self.stmt[bar + 1..].iter().any(|token| self.is_op(token, b"->")))
                } else {
                    // A statement whose bars are rows apart is a merge, and its
                    // arrows belong to other declarations: the older reading is
                    // the one that keeps them from reading as one value.
                    !bars.is_empty() && self.stmt.iter().any(|token| self.is_op(token, b"->"))
                };
            if closure_header {
                return true;
            }
            // Rust has no object literal, so a brace straight after `=` opens a
            // block of statements: `let stem = { use …; static …; … };`. Read as
            // a value, the block's statements merged into one event the CST
            // never writes (001556_build_phase.rs:880). A block the chain keeps
            // (`{ … }.catch_unwind().await`) is still a value: the method that
            // follows it is the statement's own, and cutting the statement at the
            // brace lost that call's line (001072_polling_tests.rs:469).
            if self.stmt.last().is_some_and(|token| self.is_op(token, b"=")) && !self.brace_continued() {
                return true;
            }
            // `… && match sha256_file(io, file) { … } && …`: a `match` inside a
            // binary chain is a value of that chain, not a block statement. Opened
            // as a block, its `}` ended the statement and every later link of the
            // chain was stamped on a line the CST never wrote one for
            // (001556_build_phase.rs:716).
            {
                let mut depth = 0i32;
                let mut chain = false;
                let mut keyword = false;
                for token in &self.stmt {
                    if token.kind == Kind::Op {
                        match self.raw(token) {
                            b"(" | b"[" | b"{" => depth += 1,
                            b")" | b"]" | b"}" => depth -= 1,
                            b"&&" | b"||" | b"=" if depth == 0 => chain = true,
                            _ => {}
                        }
                    } else if depth == 0 && token.kind == Kind::Ident && self.raw(token) == b"match" {
                        keyword = true;
                    }
                }
                let control = self
                    .stmt
                    .iter()
                    .find(|token| token.kind == Kind::Ident)
                    .is_some_and(|token| matches!(self.raw(token), b"if" | b"while" | b"else" | b"for" | b"loop"));
                if chain && keyword && !control {
                    return false;
                }
            }
            if let Some(first) = self.stmt.iter().find(|token| token.kind == Kind::Ident) {
                if matches!(self.raw(first), b"if" | b"for" | b"while" | b"match" | b"loop" | b"else") {
                    // `if let Self::Enabled {` newline `core,` newline `..` newline
                    // `} = self`: the brace lists the fields of a pattern, and the
                    // body opens at the brace after the binding `=`. Opened as the
                    // body here, the fields became statements and the condition
                    // lost its value to the line below them.
                    if matches!(self.raw(first), b"if" | b"while" | b"else") && self.rust_let_pattern_open() {
                        return false;
                    }
                    // `if let Some(v) = unsafe { … } && let Some(v) = unsafe { … } {`:
                    // a block expression inside a condition. Opened as the body,
                    // its `}` ended the statement, so the second link of the chain
                    // started a statement of its own and the body brace after it
                    // was read as a value: everything down to the end of the
                    // function merged into one event (000928_runtime.rs:255).
                    // The block's text is a value of the condition, not the body.
                    if self
                        .stmt
                        .last()
                        .is_some_and(|token| token.kind == Kind::Ident && matches!(self.raw(token), b"unsafe" | b"const"))
                    {
                        return false;
                    }
                    return true;
                }
            }
            // `let Some(x) = g(1) else { ... };`: the else's brace opens the
            // divergence block. Read as part of the value, the whole let-else
            // became one merged expression the CST never writes.
            if self.stmt.last().is_some_and(|token| self.raw(token) == b"else")
                && self.stmt.first().is_some_and(|token| self.raw(token) == b"let")
            {
                return true;
            }
        }
        // Cloned, not borrowed: deciding what the brace opens reads past the
        // cursor, and that needs the parser itself.
        let Some(last) = self.stmt.last().cloned() else { return !self.declaring };
        if self.lang.tag == "TCL" && last.kind == Kind::Ident && self.has_def_word(&self.stmt) {
            return false;
        }
        // In Rust, `=> {` inside an expression (e.g. `let x = match y { Pat => { ... } }`)
        // is a match arm, not a lambda; opening a block here turns it into an anonymous
        // function and replaces the arm's body with `0` in the enclosing assignment.
        if (self.is_op(&last, b"=>") && !(self.lang.tag == "RS" && !self.brackets.is_empty()))
            || (self.is_op(&last, b"->") && !self.brackets.is_empty())
        {
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
            return (self.is_op(&last, b")") && self.function_word_in_tail())
                || self.expr_class_at(&self.stmt).is_some();
        }
        // `'label: { ... }`: a labeled block is a block of statements, not a
        // struct literal or expression value.
        let label = self.rust_label_prefix(&self.stmt);
        if label > 0 && self.stmt.len() == label {
            return true;
        }
        // `export function f(s: string): { readonly group: string } { ... }`:
        // the `{` after `:`, `&`, or `|` opens an inline return type literal,
        // not the function body. `declare function f(...): { ... }` has no
        // body and stays handled by `on_open_brace` as `type_only`.
        // `export interface encodeKeys<` newline `  S extends Top & { … },`
        // newline `> extends decodeTo<…> {` and `export declare function req<T>(x: T): ZodObject<{ … }>;`:
        // an object or mapped type inside an open `<…>` type-parameter or
        // type-argument list is not the declaration's body. Read as one, its
        // members became runtime statements the CST never writes.
        if matches!(self.lang.tag, "TS" | "TSX")
            && (self.type_declaration(&self.stmt) || self.has_def_word(&self.stmt))
            && self.angle_depth() > 0
        {
            return false;
        }
        // `[A] extends [B] ? { … } : …` — a brace after the `?` of a conditional
        // type opens a type literal, never a block, in every TypeScript
        // statement (there is no ternary whose arms are blocks). Read as a
        // block, its `}` ended the declaration and the rest of the conditional
        // chain became a merge on the arm's line (001886_Config.ts:492).
        let conditional_type_brace =
            matches!(self.lang.tag, "TS" | "TSX") && last.kind == Kind::Op && self.raw(&last) == b"?";
        let ts_return_type_brace = conditional_type_brace
            || (matches!(self.lang.tag, "TS" | "TSX")
                && last.kind == Kind::Op
                && matches!(self.raw(&last), b":" | b"&" | b"|")
                && !self.is_declare_function(&self.stmt));
        if (!ts_return_type_brace && self.has_def_word(&self.stmt)) || self.assigned_class(&self.stmt).is_some() {
            return true;
        }
        if matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX")
            && self
                .top_level_assign(&self.stmt)
                .is_some_and(|equal| self.assigned_class_head(&self.stmt[equal + 1..]).is_some())
        {
            return true;
        }
        if self.assigned_class_statement(&self.stmt).is_some() {
            return true;
        }
        if matches!(self.lang.tag, "TS" | "TSX") && self.conditional_type_head(&self.stmt) {
            return true;
        }
        if last.kind == Kind::Ident && !BLOCK_WORDS.contains(&self.raw(&last)) && self.field_key_ahead() {
            return false;
        }
        // Go writes a control header without parentheses: `for i := 0; i < n; i++ {`,
        // `if err := f(); err != nil {`, `switch v := f(x); v {`. The `:=` made the
        // last test below read the brace as a composite literal, so the whole body
        // merged into one value and every statement in it lost its own event.
        if self.lang.tag == "GO"
            && self
                .word_at(&self.stmt, 0)
                .as_deref()
                .is_some_and(|word| matches!(word, "for" | "if" | "switch" | "select"))
            && self.stmt.iter().any(|token| self.is_op(token, b";"))
        {
            return true;
        }
        if last.kind == Kind::Op {
            let raw = self.raw(&last);
            if raw != b")"
                && raw != b">"
                && !(matches!(self.lang.tag, "RS" | "TS" | "TSX") && raw == b">>")
                && raw != b"]"
                && raw != b"}"
            {
                return false;
            }
        }
        let last_word = self.word_at(&self.stmt, self.stmt.len() - 1);
        if last_word.is_some_and(|word| VALUE_FOLLOWS.contains(&word.as_str())) {
            return false;
        }
        !(self.top_level_assign(&self.stmt).is_some() && !self.is_op(&last, b")"))
    }

    /// Whether the statement so far ends inside the pattern of a `let`
    /// condition: `if let Self::Enabled`, `while let Some(Node::Branch`, and the
    /// later links of a let-chain. Everything between a condition's `let` and
    /// its binding `=` is a pattern, so a brace there names fields.
    fn rust_let_pattern_open(&self) -> bool {
        let Some(at) = self.stmt.iter().rposition(|token| token.kind == Kind::Ident && self.raw(token) == b"let") else {
            return false;
        };
        let opens_condition = at > 0 && {
            let before = &self.stmt[at - 1];
            (before.kind == Kind::Ident && matches!(self.raw(before), b"if" | b"while"))
                || (before.kind == Kind::Op && matches!(self.raw(before), b"&&" | b"||"))
        };
        if !opens_condition {
            return false;
        }
        let mut depth = 0i32;
        for token in &self.stmt[at + 1..] {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    b"=" if depth == 0 => return false,
                    _ => {}
                }
            }
        }
        true
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
        // Kotlin has no struct/object literals; `{ ... }` is always a block or lambda.
        if self.lang.tag == "KT" {
            return false;
        }
        if self.peek_word(0).kind != Kind::Ident {
            return false;
        }
        // `if name == nil { return }`, `for { break }`: the word behind the brace
        // opens a statement, and Go has no shorthand field, so a bare word there
        // is never a key. Read as one, the brace was a value and the statement
        // behind it merged into the condition.
        if self.lang.tag == "GO" {
            let token = self.peek_word(0);
            let word = self.text(&token);
            if GO_STATEMENT_OPENERS.contains(&word.as_str()) {
                return false;
            }
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

    fn expr_class_at(&self, toks: &[Token]) -> Option<usize> {
        if !matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX") {
            return None;
        }
        let mut depth = 0i32;
        for (i, token) in toks.iter().enumerate().rev() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b")" | b"]" | b"}" => depth += 1,
                    b"(" | b"[" | b"{" => {
                        if depth == 0 {
                            return None;
                        }
                        depth -= 1;
                    }
                    b"," | b";" if depth == 0 => return None,
                    _ => {}
                }
            } else if depth == 0 && token.kind == Kind::Ident && self.raw(token) == b"class" {
                let member = i > 0 && toks.get(i - 1).is_some_and(|t| t.kind == Kind::Op && matches!(self.raw(t), b"." | b"?."));
                let key = toks.get(i + 1).is_some_and(|t| self.is_op(t, b":"));
                if !member && !key {
                    return Some(i);
                }
            }
        }
        None
    }

    fn function_word_in_tail(&self) -> bool {
        let mut depth = 0usize;
        let mut open = None;
        for (index, token) in self.stmt.iter().enumerate().rev() {
            if self.is_op(token, b")") {
                depth += 1;
            } else if self.is_op(token, b"(") {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    open = Some(index);
                    break;
                }
            }
        }
        let Some(mut cursor) = open else { return false };
        if cursor > 0 && self.is_op(&self.stmt[cursor - 1], b">") {
            let mut angle = 0usize;
            for index in (0..cursor).rev() {
                if self.is_op(&self.stmt[index], b">") {
                    angle += 1;
                } else if self.is_op(&self.stmt[index], b"<") {
                    angle -= 1;
                    if angle == 0 {
                        cursor = index;
                        break;
                    }
                }
            }
        }
        if cursor > 0 && self.stmt[cursor - 1].kind == Kind::Ident {
            if self.lang.defs.iter().any(|word| word.as_bytes() == self.raw(&self.stmt[cursor - 1])) {
                return true;
            }
            cursor -= 1;
        }
        if cursor > 0 && self.is_op(&self.stmt[cursor - 1], b"*") {
            cursor -= 1;
        }
        cursor > 0
            && self.stmt[cursor - 1].kind == Kind::Ident
            && self.lang.defs.iter().any(|word| word.as_bytes() == self.raw(&self.stmt[cursor - 1]))
    }

    /// Advance past leading modifiers, including Rust attributes (`#[...]`,
    /// `#![...]`) and visibility groups such as `pub(crate)` or `pub(super)`.
    fn skip_modifiers(&self, toks: &[Token]) -> usize {
        let mut index = 0;
        loop {
            if self.lang.tag == "RS" && toks.get(index).is_some_and(|t| self.is_op(t, b"#")) {
                let open = if toks.get(index + 1).is_some_and(|t| self.is_op(t, b"!")) {
                    index + 2
                } else {
                    index + 1
                };
                if toks.get(open).is_some_and(|t| self.is_op(t, b"[")) {
                    if let Some((_, close)) = self.group_at(toks, open) {
                        index = close + 1;
                        continue;
                    }
                    break;
                }
            }
            if self.lang.tag == "KT" && toks.get(index).is_some_and(|t| self.is_op(t, b"@")) {
                let mut next = index + 1;
                if toks.get(next).is_some_and(|t| t.kind == Kind::Ident) {
                    next += 1;
                    while toks.get(next).is_some_and(|t| self.is_op(t, b"."))
                        && toks.get(next + 1).is_some_and(|t| t.kind == Kind::Ident)
                    {
                        next += 2;
                    }
                    if toks.get(next).is_some_and(|t| self.is_op(t, b"(")) {
                        if let Some((_, close)) = self.group_at(toks, next) {
                            next = close + 1;
                        } else {
                            break;
                        }
                    }
                    index = next;
                    continue;
                }
            }
            let Some(word) = self.word_at(toks, index) else { break };
            if self.lang.defs.contains(&word.as_str()) || self.lang.classes.contains(&word.as_str()) {
                break;
            }
            // Kotlin writes `actual fun`, `expect class`, `suspend fun` and
            // `operator fun`: these words bind nothing and sit where the other
            // languages put `public`, so the declaration behind them is the
            // definition the line writes. Left unskipped, the word was taken for
            // the statement's first word and the function it declares went
            // unregistered - `actual fun w(): L = f(X)` kept its event on the
            // module and lost the symbol.
            let kotlin_modifier = self.lang.tag == "KT"
                && matches!(
                    word.as_str(),
                    "actual" | "expect" | "suspend" | "operator" | "infix" | "tailrec" | "lateinit" | "noinline"
                        | "crossinline" | "vararg" | "reified" | "annotation" | "enum" | "inner" | "value"
                );
            // `pub const fn` and `pub const unsafe extern "C" fn`: in Rust
            // `const` qualifies a function item just like `async` and `unsafe`.
            let rust_modifier = self.lang.tag == "RS" && word == "const";
            if !MODIFIERS.contains(&word.as_str()) && !kotlin_modifier && !rust_modifier {
                break;
            }
            index += 1;
            if self.lang.tag == "RS" && word == "pub" && toks.get(index).is_some_and(|t| self.is_op(t, b"(")) {
                if let Some((_, close)) = self.group_at(toks, index) {
                    index = close + 1;
                } else {
                    break;
                }
            } else if self.lang.tag == "RS"
                && word == "extern"
                && toks.get(index).is_some_and(|t| t.kind == Kind::Str)
            {
                index += 1;
            }
        }
        index
    }

    /// Advance past leading Rust attributes (`#[...]` / `#![...]`) before a declaration head.
    fn skip_rs_attrs(&self, toks: &[Token]) -> usize {
        let mut index = 0;
        while self.lang.tag == "RS" && toks.get(index).is_some_and(|t| self.is_op(t, b"#")) {
            let next = if toks.get(index + 1).is_some_and(|t| self.is_op(t, b"!")) {
                index + 2
            } else {
                index + 1
            };
            if toks.get(next).is_some_and(|t| self.is_op(t, b"[")) {
                if let Some((_, close)) = self.group_at(toks, next) {
                    index = close + 1;
                    continue;
                }
            }
            break;
        }
        index
    }

    fn has_def_word(&self, toks: &[Token]) -> bool {
        let index = self.skip_modifiers(toks);
        self.word_at(toks, index).is_some_and(|word| {
            self.lang.defs.contains(&word.as_str())
                || (self.lang.tag == "RS" && self.lang.classes.contains(&word.as_str()))
        })
    }

    /// A `function` expression in a statement that continues past its closing
    /// brace (`return r = a, n = function() { ... }, n()`), where no open bracket
    /// marks expression position.
    fn expr_function(&self, stmt: &[Token]) -> bool {
        if !stmt.last().is_some_and(|token| self.is_op(token, b")")) {
            return false;
        }
        if self.head_word(stmt).is_some_and(|word| CONTROL.contains(&word.as_str())) {
            return false;
        }
        if self.function_head(stmt, true).is_some() {
            return false;
        }
        let mut depth = 0i32;
        for token in stmt {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    _ => {}
                }
            } else if depth == 0
                && token.kind == Kind::Ident
                && self.lang.defs.iter().any(|word| word.as_bytes() == self.raw(token))
            {
                return true;
            }
        }
        false
    }

    fn rust_label_prefix(&self, stmt: &[Token]) -> usize {
        if self.lang.tag != "RS" {
            return 0;
        }
        match (stmt.first(), stmt.get(1), stmt.get(2)) {
            (Some(first), Some(colon), _)
                if first.kind == Kind::Ident && self.raw(first).starts_with(b"'") && self.is_op(colon, b":") =>
            {
                2
            }
            (Some(quote), Some(name), Some(colon))
                if self.is_op(quote, b"'") && name.kind == Kind::Ident && self.is_op(colon, b":") =>
            {
                3
            }
            _ => 0,
        }
    }

    fn open_block(&mut self) {
        // A bare name opening a block right after a lambda block closed is the
        // infix call that continues the expression above it (`} builder {` in
        // `fn("x") { … } builder { … }`): the CST folds it in and writes
        // nothing, while the one-pass reader filed `x n:builder` — 20 files of
        // the kotlin dump.
        let closed_line = std::mem::take(&mut self.closed_line);
        let after_lambda = closed_line != 0
            && self.stmt.first().is_some_and(|token| token.line == closed_line);
        let mut stmt = std::mem::take(&mut self.stmt);
        // `'descend: loop { … }` and `'label: { … }`: the label names the loop
        // or block for `continue`/`break` and is not its value. The CST writes
        // no event for the label, so the label and its colon are dropped.
        let label = self.rust_label_prefix(&stmt);
        if label > 0 {
            let control = stmt
                .get(label)
                .filter(|token| token.kind == Kind::Ident)
                .map(|token| self.raw(token))
                .is_some_and(|word| matches!(word, b"loop" | b"while" | b"for"));
            if stmt.len() == label || (control && stmt.len() > label) {
                stmt.drain(..label);
            }
        }
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
                    || stmt[equal + 1..].iter().any(|token| self.is_op(token, b"?"))
            });
        let expr_function = matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && !type_declaration
            && !in_expression
            && !arrow
            && self.expr_function(&stmt);
        if in_expression {
            if let Some(at) = self.expr_class_at(&stmt) {
                if let Some((name, bases)) = self.assigned_class_head(&stmt[at..]) {
                    let brackets = std::mem::take(&mut self.brackets);
                    let mut depth = 0i32;
                    let mut ext = None;
                    let mut imp = None;
                    for (i, t) in stmt[at..].iter().enumerate() {
                        if t.kind == Kind::Op {
                            match self.raw(t) {
                                b"(" | b"[" | b"{" => depth += 1,
                                b")" | b"]" | b"}" => depth -= 1,
                                _ => {}
                            }
                        } else if depth == 0 && t.kind == Kind::Ident {
                            if ext.is_none() && self.raw(t) == b"extends" {
                                ext = Some(at + i);
                            } else if ext.is_some() && self.raw(t) == b"implements" {
                                imp = Some(at + i);
                                break;
                            }
                        }
                    }
                    if let Some(ext_at) = ext {
                        if let Some(imp_at) = imp {
                            stmt.truncate(imp_at);
                        }
                        stmt.drain(at..ext_at + 1);
                    } else {
                        stmt.truncate(at);
                    }
                    self.push_class(name, bases, start_col, Some((stmt, brackets)));
                    return;
                }
            }
        }
        if in_expression || arrow || expr_function {
            // `else h(function() { … });`: the brace opens a function inside the
            // arm's statement, not the arm's block. The arm ends with that
            // statement, so it is settled here rather than carried into the
            // function body: left pending, the body's first statement would
            // claim the arm and pop a frame of its own, and the control frame
            // then outlived the branch and swallowed the `}` that closes the
            // enclosing function.
            if rearmed {
                self.final_arm(&stmt);
                // `else if (x) f(function () { … });` carries a header of its
                // own and its condition is a value; a plain `else f(...)` has
                // no condition, and reading the call as one invented an event
                // the CST never writes.
                if self.word_at(&stmt, 1).as_deref() == Some("if") {
                    self.control_condition(&stmt);
                }
                self.event("[\"j\"]");
                self.pop_frame();
            }
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
        } else if self.head_word(&stmt).is_some_and(|word| word == "extern")
            && (self.lang.tag == "RS"
                || (matches!(self.lang.tag, "C" | "CPP" | "OBJC")
                    && stmt.get(1).is_some_and(|token| token.kind == Kind::Str)))
        {
            // `extern "C" {`: a block header, and the CST writes no event for it.
            // The lexer's `m:[n:extern,0]` was a fact the CST never had, and
            // every extern block lost its file (8 corpus files). The C family
            // reads a `.h` holding `namespace` as CPP, and the C++ grammar's
            // linkage_specification writes nothing either, so the guarded
            // headers lost the same event (19 rows on the croaring corpus).
            let symbol = self.top().symbol;
            let class = self.top().class.clone();
            let receiver = self.top().receiver.clone();
            self.frames.push(Frame { kind: FrameKind::Plain, symbol, class, receiver, col: start_col, saved: None, defines: None, type_only: false });
        } else if self.lang.tag == "RS"
            && stmt.len() > 2
            && stmt.last().is_some_and(|token| self.raw(token) == b"else")
            && stmt.first().is_some_and(|token| self.raw(token) == b"let")
        {
            // The let half writes its assignment; the else's body is statements
            // of the enclosing symbol, and the header itself names nothing.
            let head: Vec<Token> = stmt[..stmt.len() - 1].to_vec();
            self.statement(&head);
            let symbol = self.top().symbol;
            let class = self.top().class.clone();
            let receiver = self.top().receiver.clone();
            self.frames.push(Frame { kind: FrameKind::Plain, symbol, class, receiver, col: start_col, saved: None, defines: None, type_only: false });
        } else if let Some((name, bases)) = self.class_head(&stmt).or_else(|| {
            matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX")
                .then(|| {
                    self.top_level_assign(&stmt)
                        .and_then(|equal| self.assigned_class_head(&stmt[equal + 1..]))
                })
                .flatten()
        }) {
            self.push_class(name, bases, start_col, None);
        } else if let Some((name, bases)) = self.assigned_class(&stmt) {
            self.push_class(name, bases, start_col, None);
        } else if let Some((name, bases)) = self.assigned_class_statement(&stmt) {
            self.push_class(name, bases, start_col, None);
        } else if let Some(head) = self.function_head(&stmt, true) {
            let assigned = head.assigned_name && matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX");
            self.push_function(head, start_col, None);
            if assigned {
                if let Some(frame) = self.frames.last_mut() {
                    frame.saved = Some(SavedStmt::Assigned(std::mem::take(&mut stmt), std::mem::take(&mut self.brackets)));
                }
            }
        } else if self.head_word(&stmt).is_some_and(|word| CONTROL.contains(&word.as_str())) {
            self.open_control(&stmt, start_col);
        } else {
            let typedef_block = matches!(self.lang.tag, "C" | "CPP" | "OBJC")
                && self.head_word(&stmt).as_deref() == Some("typedef");
            let infix_tail = self.lang.tag == "KT"
                && after_lambda
                && stmt.len() == 1
                && stmt[0].kind == Kind::Ident;
            if !infix_tail {
                self.statement(&stmt);
            }
            let symbol = self.top().symbol;
            let class = self.top().class.clone();
            let receiver = self.top().receiver.clone();
            self.frames.push(Frame { kind: FrameKind::Plain, symbol, class, receiver, col: start_col, saved: None, defines: None, type_only: false });
            if typedef_block {
                self.typedef_block = Some(self.frames.len());
            }
        }
        stmt.clear();
        self.stmt = stmt;
    }

    fn open_control(&mut self, mut stmt: &[Token], col: u32) {
        let mut peeled = false;
        while matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
            && self
                .word_at(stmt, 0)
                .as_deref()
                .is_some_and(|w| matches!(w, "if" | "for" | "while"))
        {
            let Some(end) = self.matching_close(stmt, 1) else { break };
            if !self
                .word_at(stmt, end + 1)
                .as_deref()
                .is_some_and(|w| matches!(w, "if" | "for" | "while"))
            {
                break;
            }
            if self.word_at(stmt, 0).as_deref() == Some("for") {
                self.loop_header(&stmt[2..end], stmt[0].line);
            } else {
                self.expression_event("x", &stmt[2..end]);
            }
            peeled = true;
            self.event("[\"b\"]");
            stmt = &stmt[end + 1..];
        }
        if (peeled
            || (matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX")
                && stmt.iter().any(|t| self.is_op(t, b","))
                && stmt.iter().any(|t| self.is_op(t, b";"))))
            && self.word_at(stmt, 0).as_deref() == Some("for")
        {
            if let Some(end) = self.matching_close(stmt, 1) {
                self.loop_header(&stmt[2..end], stmt[0].line);
                self.event("[\"b\"]");
                let symbol = self.top().symbol;
                let class = self.top().class.clone();
                let receiver = self.top().receiver.clone();
                self.frames.push(Frame { kind: FrameKind::Control, symbol, class, receiver, col, saved: None, defines: None, type_only: false });
                return;
            }
        }
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

    /// Where the statement's head word sits: `async for row in xs` and
    /// `async with … as x` bind exactly like their plain forms, and the
    /// modifier is not a binder (it was read as one, so the target list was
    /// `["async","row"]` and the file fell back).
    fn binding_head(&self, stmt: &[Token]) -> usize {
        let word = |index: usize| stmt.get(index).filter(|token| token.kind == Kind::Ident).map(|token| self.raw(token));
        match word(0) {
            Some(b"async") if matches!(word(1), Some(b"for") | Some(b"with")) => 1,
            _ => 0,
        }
    }

    fn binding_index(&self, stmt: &[Token]) -> Option<usize> {
        // `if x in y` and `while x in y` test membership; only a loop, a
        // context manager or a handler binds a name across `in`/`of`/`as`.
        let head = self.binding_head(stmt);
        if let Some(first) = stmt.get(head).filter(|token| token.kind == Kind::Ident) {
            if !matches!(self.raw(first), b"for" | b"foreach" | b"with" | b"except" | b"catch" | b"rescue") {
                return None;
            }
        }
        let mut depth = 0;
        for (index, token) in stmt.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    _ => {}
                }
            }
            if depth <= 1 && token.kind == Kind::Ident && index > head && matches!(self.raw(token), b"in" | b"of" | b"as") {
                return Some(index);
            }
        }
        None
    }

    /// Rust `let` patterns inside a condition: `if let Some(v) = x` and the
    /// let-chain `if a && let Some(v) = f() { … }`. Everything between a `let`
    /// and its binding `=` is a pattern, and a pattern is not a value, so the
    /// constructors inside it (`Some(`, `Tok::Open(`) were read as calls the CST
    /// never writes. `leading` allows the strip to start at the first token,
    /// which is right for a condition slice and wrong for a statement (where a
    /// leading `let` is a declaration that has its own path).
    fn strip_let_patterns(&self, toks: &[Token], leading: bool) -> Vec<Token> {
        let mut out: Vec<Token> = Vec::with_capacity(toks.len());
        let mut index = 0;
        while index < toks.len() {
            let token = &toks[index];
            let after_operator = out
                .last()
                .is_some_and(|last| last.kind == Kind::Op && matches!(self.raw(last), b"&&" | b"||"));
            let after_keyword = out
                .last()
                .is_some_and(|last| last.kind == Kind::Ident && matches!(self.raw(last), b"if" | b"while"));
            let boundary = after_operator || after_keyword || (leading && index == 0);
            if boundary && token.kind == Kind::Ident && self.raw(token) == b"let" {
                let mut depth = 0i32;
                let mut at = index + 1;
                let mut equal = None;
                while at < toks.len() {
                    let next = &toks[at];
                    if next.kind == Kind::Op {
                        match self.raw(next) {
                            b"(" | b"[" | b"{" => depth += 1,
                            b")" | b"]" | b"}" => depth -= 1,
                            b"=" if depth == 0 => {
                                equal = Some(at);
                                break;
                            }
                            _ => {}
                        }
                    }
                    at += 1;
                }
                match equal {
                    Some(at) => {
                        index = at + 1;
                        continue;
                    }
                    None => {
                        out.push(token.clone());
                        index += 1;
                        continue;
                    }
                }
            }
            out.push(token.clone());
            index += 1;
        }
        out
    }

    fn control_condition_before(&mut self, stmt: &[Token]) {
        // `catch (IOException ioe)` declares the handler's variable and assigns
        // nothing: the CST writes no value for the header, and read as one the
        // parameter became an `x` event with an empty target list — 1,246
        // losses over 438 files, the second-largest java cluster in the dump.
        if self.lang.tag == "JAVA" && self.word_at(stmt, 0).as_deref() == Some("catch") {
            return;
        }
        // Python's `match subject:` is a switch and `case pattern:` is a label:
        // neither writes a value the CST records, and the guard read both
        // events as lost the moment the CST (rightly) wrote nothing.
        if self.lang.tag == "PY" && matches!(self.raw(&stmt[0]), b"match" | b"case") {
            return;
        }
        if self.binding_index(stmt).is_none() && stmt.len() > 1 {
            // `if let (Some(d), Action::Reduce(p)) = (def, act)` binds a pattern
            // and calls nothing, and so does a let-chain: `if a && let Some(v) =
            // f() { … }`. Everything between a `let` and its binding `=` is a
            // pattern — read whole, its constructors became calls the CST never
            // writes (the largest rust cluster in the shape table, and the 15
            // files whose condition continues on a `&&` line).
            if self.lang.tag == "RS" {
                let kept = self.strip_let_patterns(&stmt[1..], true);
                if kept.len() != stmt.len() - 1 {
                    // The condition is stamped where it starts, which is where
                    // the CST stamps its `let_condition`. A pattern written over
                    // several lines leaves the first kept token — the value
                    // after `=` — lines below the `let`, and stamped there the
                    // one condition was two events no line could pair.
                    let start = stmt[1..]
                        .iter()
                        .find(|token| !(token.kind == Kind::Ident && matches!(self.raw(token), b"if" | b"while")))
                        .map_or(stmt[1].line, |token| token.line);
                    self.expression_event_at("x", &kept, start);
                    return;
                }
            }
            self.expression_event("x", &stmt[1..]);
        }
    }

    /// `for a, b in c`, `for (const a of b)`, `with c as a`.
    fn loop_binding(&mut self, stmt: &[Token]) {
        let Some(at) = self.binding_index(stmt) else { return };
        let is_with = self.raw(&stmt[at]) == b"as";
        let head = self.binding_head(stmt);
        let value_start = if is_with { self.with_item_start(&stmt[head + 1..at]) } else { 0 };
        let (targets, value) = if is_with {
            (&stmt[at + 1..], &stmt[head + 1 + value_start..at])
        } else {
            (&stmt[head + 1..at], &stmt[at + 1..])
        };
        self.assignment(targets, value, stmt[0].line, false);
    }

    /// Where the context manager an `as` binds starts. `with open(a), open(b)
    /// as fh:` binds `fh` to the second item, not to the whole clause; read
    /// from the `with`, the value was a tuple of every item and the CST's one
    /// call had no partner. The last comma at the clause's own depth is the
    /// boundary, so commas inside an item's call or literal do not split it.
    fn with_item_start(&self, clause: &[Token]) -> usize {
        let mut depth = 0i32;
        let mut start = 0;
        for (index, token) in clause.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" => depth += 1,
                b")" | b"]" | b"}" => depth -= 1,
                b"," if depth == 0 => start = index + 1,
                _ => {}
            }
        }
        start
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

    fn push_class(&mut self, name: String, bases: Vec<String>, col: u32, saved: Option<(Vec<Token>, Vec<u8>)>) {
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
            saved: saved.map(|(stmt, brackets)| SavedStmt::Expr(stmt, brackets)),
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
            saved: saved.map(|(stmt, brackets)| SavedStmt::Expr(stmt, brackets)),
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

    fn continues_after_brace(&mut self) -> bool {
        let next = self.peek_word(0);
        next.kind == Kind::Op && matches!(self.raw(&next), b"." | b"?." | b"(" | b"[")
    }

    fn close_block(&mut self) {
        if self.frames.len() <= 1 {
            return;
        }
        self.closed_line = self.last_line;
        if self.top().kind == FrameKind::Control {
            self.closed_branch = true;
            return;
        }
        let frame = self.pop_frame();
        match frame.saved {
            Some(SavedStmt::Expr(stmt, brackets)) => {
                self.stmt = stmt;
                self.brackets = brackets;
                self.stmt.push(Token { kind: Kind::Lambda, start: 0, end: 0, line: self.last_line, col: 0, interp: None });
            }
            Some(SavedStmt::Assigned(stmt, brackets)) if self.continues_after_brace() => {
                // The function expression is the receiver/callee of `.bind()` / `()`,
                // not the value assigned to the LHS, so it stays anonymous.
                if let Some(index) = frame.defines {
                    let symbol = &mut self.facts.symbols[index];
                    let anon = format!("<anonymous>@{}", symbol.line);
                    symbol.key = anon.clone();
                    symbol.name = anon;
                    symbol.owner.clear();
                }
                self.stmt = stmt;
                self.brackets = brackets;
                self.stmt.push(Token { kind: Kind::Lambda, start: 0, end: 0, line: self.last_line, col: 0, interp: None });
            }
            Some(SavedStmt::Assigned(..)) | None => {}
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
        self.expression_event_at(form, toks, first.line);
    }

    /// An expression event stamped on a line the caller names, for a value
    /// whose first token is not where its statement starts.
    fn expression_event_at(&mut self, form: &str, toks: &[Token], line: u32) {
        if toks.is_empty() {
            return;
        }
        let value = self.values(toks);
        if value != "0" || form == "r" {
            self.event(&format!("[\"{form}\",{line},{value}]"));
        }
    }

    /// A whole expression; several top-level values become one tuple.
    fn values(&mut self, toks: &[Token]) -> String {
        if self.lang.tag == "RS"
            && toks.len() > 2
            && toks.iter().any(|t| t.kind == Kind::Ident && self.raw(t) == b"let")
        {
            let kept = self.strip_let_patterns(toks, false);
            if kept.len() != toks.len() {
                return self.values(&kept);
            }
        }
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
        // A statement that opens with `.` is the tail of a chain whose head was
        // a block that just closed: `let line = if c { a } else { b }` then
        // `.max(1.0);`. The CST writes that chain inside the enclosing value, so
        // an event of its own is a line the other reader never writes
        // (001659_commit.rs:491).
        if self.lang.tag == "RS" && self.stmt.first().is_some_and(|token| self.is_op(token, b".")) {
            self.stmt.clear();
            self.brackets.clear();
            return;
        }
        if self.suppress_event {
            self.suppress_event = false;
            self.stmt.clear();
            self.brackets.clear();
            return;
        }
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
        // A C declaration is not a statement: `int __cxa_atexit(void (*)(void
        // *), void *);` names a function and calls nothing, `static double
        // pzero(double), qzero(double);` names two, and the one-pass reader
        // wrote every declarator as a call. `__attribute__((...))` contents
        // were calls too (`visibility("protected")`, `alloc_size(1)`). The CST
        // writes no value for a declaration, so nothing the lexer read out of
        // one is a fact the record should keep (18 corpus files).
        // `typedef uint8_t art_key_chunk_t;` names a type: no runtime value is
        // bound, and a reader of the record needs no event for it. The C++
        // grammar's declaration_statement keeps the name and the C grammar keeps
        // the whole specifier chain, so the two readers wrote a different number
        // of names for the same declaration and every typedef header lost its
        // file (5 rows on the croaring corpus). Whether the specifier chain is
        // written is the C reader's choice; the lexer writes none.
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && self.word_at(&self.stmt, 0).as_deref() == Some("typedef")
        {
            self.brackets.clear();
            self.stmt.clear();
            return;
        }
        // `using api::iterator;` and `using Id = int;` are C++ declarations with
        // no runtime value: the C++ grammar reads them as a using_declaration and
        // the fact writer binds nothing for it. Read as an expression the lexer
        // wrote `m:[n:using,n:api.iterator]` on every guarded header that names
        // an alias (7 rows on the croaring corpus).
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && self.stmt.len() > 1
            && self.word_at(&self.stmt, 0).as_deref() == Some("using")
        {
            self.brackets.clear();
            self.stmt.clear();
            return;
        }
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC") && self.c_declaration() {
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

    /// Whether the statement in hand is a C declaration: a type or storage
    /// class opens it (after any `__attribute__((...))`), it carries a
    /// declarator's parenthesis, and it has no initializer and no body.
    fn c_declaration(&self) -> bool {
        let stmt = &self.stmt;
        if stmt.is_empty() {
            return false;
        }
        // `int x = f();` is a declaration whose value the CST writes, and its
        // call is real: only a declaration with nothing to initialise is one
        // the reader should keep nothing from. A body means a definition.
        if stmt.iter().any(|token| self.is_op(token, b"=") || self.is_op(token, b"{")) {
            return false;
        }
        if !stmt.iter().any(|token| self.is_op(token, b"(")) {
            return false;
        }
        let mut at = 0;
        // `__attribute__((__visibility__("hidden"))) int f(void);`: an
        // attribute is part of the declaration it precedes, not a value of it.
        while at < stmt.len()
            && stmt[at].kind == Kind::Ident
            && matches!(self.raw(&stmt[at]), b"__attribute__" | b"__attribute")
        {
            let mut depth = 0i32;
            at += 1;
            while at < stmt.len() {
                if self.is_op(&stmt[at], b"(") {
                    depth += 1;
                } else if self.is_op(&stmt[at], b")") {
                    depth -= 1;
                    if depth <= 0 {
                        at += 1;
                        break;
                    }
                }
                at += 1;
            }
        }
        // `return callee(y);` also opens with two words in a row, and the
        // "two words declare a type" rule below read it as a prototype: the
        // returned call vanished from every C function that returns one (the
        // caller→callee edge test caught it). A statement keyword opens no
        // declaration, whatever follows it.
        if self
            .word_at(stmt, at)
            .is_some_and(|word| {
                matches!(
                    word.as_str(),
                    "return" | "goto" | "if" | "for" | "while" | "switch" | "break" | "continue" | "sizeof"
                        | "throw" | "delete" | "co_return" | "co_await"
                )
            })
        {
            return false;
        }
        let Some(head) = stmt.get(at) else { return false };
        if head.kind != Kind::Ident {
            return false;
        }
        if crate::expr::c_declaration_word(&self.text(head)) {
            return true;
        }
        // `hidden void __synccall(void (*)(void *), void *);`: an annotation
        // macro sits exactly where a type does, so the word before a type word
        // is the declaration's first word.
        if stmt
            .get(at + 1)
            .is_some_and(|next| next.kind == Kind::Ident && crate::expr::c_declaration_word(&self.text(next)))
        {
            return true;
        }
        // `size_t art_shrink_to_fit(art_t *art);`: the type is a name the
        // reader has no keyword for (`size_t`, `ART_ATTR`, a typedef), so the
        // head is two words in a row. A statement cannot open with two words
        // unless the first one declares the type, and a declarator's `(` is
        // already required above, so this is a prototype and not a call: the
        // declaration writes no value, and reading the declarator as a call
        // cost the header its file (3 rows on the croaring corpus).
        stmt.get(at + 1).is_some_and(|next| next.kind == Kind::Ident)
    }

    fn brace_statement(&mut self, stmt: &[Token]) {
        let rearmed = std::mem::take(&mut self.rearmed);
        let word = self.head_word(stmt);
        if matches!(word.as_deref(), Some("case" | "default")) {
            if self.top().kind == FrameKind::Control {
                self.event("[\"e\"]");
            }
            if let Some(colon) = stmt.iter().position(|token| self.is_op(token, b":")) {
                self.brace_statement(&stmt[colon + 1..]);
            }
            return;
        }
        if rearmed {
            // `else x = y;`: the arm is this one statement.
            self.final_arm(&stmt[..1]);
            self.brace_statement(&stmt[1..]);
            self.event("[\"j\"]");
            self.pop_frame();
            return;
        }
        if word.as_deref() == Some("else") && stmt.len() > 1 {
            self.brace_statement(&stmt[1..]);
            return;
        }
        if let Some(word) = word.filter(|word| matches!(word.as_str(), "if" | "for" | "while" | "foreach")) {
            let _ = word;
            if let Some(end) = self.matching_close(stmt, 1) {
                if end + 1 < stmt.len() {
                    // The loop header is an assignment of its variable to the
                    // iterable, not an expression: the CST writes `a` with the
                    // binder as the target. Read as `x` the header lost the
                    // iterable name it does read.
                    if word == "for" {
                        self.loop_header(&stmt[2..end], stmt[0].line);
                    } else {
                        self.expression_event("x", &stmt[2..end]);
                    }
                    self.event("[\"b\"]");
                    self.brace_statement(&stmt[end + 1..]);
                    self.event("[\"j\"]");
                    return;
                } else if word == "for" {
                    self.loop_header(&stmt[2..end], stmt[0].line);
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

    /// `for (var k in m)` reads as `a` with the binder as the target and, for
    /// for-in/of, the iterable as the value; a classic `for` keeps only its
    /// declarator and writes the constant value the CST writes.
    fn loop_header(&mut self, header: &[Token], line: u32) {
        let mut at = 0;
        let has_decl = header
            .first()
            .is_some_and(|token| matches!(self.raw(token), b"var" | b"let" | b"const"));
        if has_decl {
            at = 1;
        }
        let rest = &header[at..];
        let semi = rest.iter().position(|token| self.is_op(token, b";")).unwrap_or(rest.len());
        let bound = (semi == rest.len())
            .then(|| rest.iter().position(|token| token.kind == Kind::Ident && matches!(self.raw(token), b"in" | b"of")))
            .flatten();
        match bound {
            Some(bound) => self.assignment(&rest[..bound], &rest[bound + 1..], line, false),
            None => {
                let init = &rest[..semi];
                let mut depth = 0i32;
                let mut start = 0usize;
                for (i, token) in init.iter().enumerate() {
                    if token.kind == Kind::Op {
                        match self.raw(token) {
                            b"(" | b"[" | b"{" => depth += 1,
                            b")" | b"]" | b"}" => depth -= 1,
                            b"," if depth == 0 => {
                                let part = &init[start..i];
                                if let Some(eq) = part.iter().position(|t| self.is_op(t, b"=")) {
                                    self.assignment(&part[..eq], &part[eq + 1..], line, false);
                                } else if has_decl && part.len() == 1 && part[0].kind == Kind::Ident {
                                    self.expression_event("x", part);
                                }
                                start = i + 1;
                            }
                            _ => {}
                        }
                    }
                }
                let part = &init[start..];
                if let Some(eq) = part.iter().position(|t| self.is_op(t, b"=")) {
                    self.assignment(&part[..eq], &part[eq + 1..], line, false);
                } else if has_decl && part.len() == 1 && part[0].kind == Kind::Ident {
                    self.expression_event("x", part);
                } else if start == 0 && !part.is_empty() {
                    self.assignment(part, &[], line, false);
                }
                if semi < rest.len() {
                    let _ = self.values(&rest[semi..]);
                }
            }
        }
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

    /// The slices between the top-level commas of a token run: `a, f(b, c)`
    /// is two parts, `[a, b]` and `f(x, y)` are one each.
    /// `void (f) (int x)`: a declaration whose declared name is parenthesised,
    /// so the macro over that name cannot expand. It starts with the type, and
    /// the parenthesised name is followed by the parameter list.
    fn parenthesised_declarator(&self, stmt: &[Token]) -> bool {
        if stmt.first().is_none_or(|token| token.kind != Kind::Ident) {
            return false;
        }
        stmt.windows(4).any(|window| {
            self.is_op(&window[0], b"(")
                && window[1].kind == Kind::Ident
                && self.is_op(&window[2], b")")
                && self.is_op(&window[3], b"(")
        })
    }

    /// `int i = 0, cnt;` and `size_t size=0, nmem=0;`: a C declarator list
    /// declares one name per declarator, so the initialiser ends at the first
    /// top-level comma. Read as one value the lexer joined the next declarator
    /// to the initialiser and wrote a tuple — `t:[0,n:cnt]` — that the CST,
    /// which writes each declarator's own value, never has (musl mallocng,
    /// passwd: the "the CST wrote the value as the constant 0" losses).
    fn c_initialiser<'t>(&self, head: &[Token], value: &'t [Token]) -> &'t [Token] {
        if self.lang.tag != "C" || !self.c_typed_declarator(head) {
            return value;
        }
        self.top_commas(value).into_iter().next().unwrap_or(value)
    }

    /// Whether the words before a statement's `=` name a type and a declarator
    /// rather than an assignment target: `int i`, `size_t size`, `char *p`,
    /// `struct meta *m`. A target is one name or a path, so two identifiers and
    /// no member or index operator between them is a declaration (`x.y = 1` and
    /// `a[i] = 1` are assignments).
    fn c_typed_declarator(&self, head: &[Token]) -> bool {
        if head.iter().any(|token| {
            token.kind == Kind::Op && matches!(self.raw(token), b"." | b"[" | b"]" | b")" | b"->")
        }) {
            return false;
        }
        head.iter().filter(|token| token.kind == Kind::Ident).count() >= 2
    }

    fn top_commas<'t>(&self, toks: &'t [Token]) -> Vec<&'t [Token]> {
        let mut parts = Vec::new();
        let mut depth = 0i32;
        let mut start = 0;
        for (index, token) in toks.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" | b"<" => depth += 1,
                b")" | b"]" | b"}" | b">" => depth -= 1,
                b"," if depth == 0 => {
                    parts.push(&toks[start..index]);
                    start = index + 1;
                }
                _ => {}
            }
        }
        if start < toks.len() {
            parts.push(&toks[start..]);
        }
        parts
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
            self.push_class(name, bases, col, None);
        } else if let Some(mut head) = self.function_head(header, true) {
            head.body = None;
            self.push_function(head, col, None);
        } else if word.as_deref().is_some_and(|word| CONTROL.contains(&word)) && colon.is_some() {
            self.open_control(header, col);
        } else if word.as_deref() == Some("async")
            && self.word_at(header, 1).as_deref().is_some_and(|word| CONTROL.contains(&word))
            && colon.is_some()
        {
            self.open_control(&header[1..], col);
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
                self.push_class(name, bases, col, None);
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
        // `from` is a contextual keyword and can be imported (`import {from} from 'm'`
        // or `import from from 'm'`); the clause keyword sits at brace depth 0
        // immediately before the module string.
        let mut depth = 0i32;
        let from = stmt.iter().enumerate().position(|(i, token)| {
            match self.raw(token) {
                b"(" | b"[" | b"{" if token.kind == Kind::Op => depth += 1,
                b")" | b"]" | b"}" if token.kind == Kind::Op => depth -= 1,
                b"from" if depth == 0 && token.kind == Kind::Ident => {
                    return stmt.get(i + 1).is_some_and(|next| next.kind == Kind::Str);
                }
                _ => {}
            }
            false
        });
        // `import x = require("m")` is a TypeScript import-equals declaration,
        // not a static import: the module sits inside `require`, not at the
        // position after `import`. Read as a static import it found no string at
        // position 1 and returned false, so the statement fell through to the
        // assignment path and `require(...)` became a call the CST never writes
        // (14 .d.ts files, the largest untouched ts cluster).
        if word.as_deref() == Some("import") {
            if let Some(equal) = self.top_level_assign(stmt) {
                let tail = &stmt[equal + 1..];
                if tail.first().is_some_and(|token| token.kind == Kind::Ident && self.raw(token) == b"require") {
                    let module = tail.iter().find(|token| token.kind == Kind::Str).map(|token| self.string_value(token));
                    let alias = stmt[..equal].iter().rev().find(|token| token.kind == Kind::Ident).map(|token| self.text(token));
                    if let (Some(module), Some(alias)) = (module, alias) {
                        self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
                        return true;
                    }
                }
            }
        }
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
        let stmt = self.strip_outer_parens(stmt);
        let Some(at) = stmt.iter().position(|token| token.kind == Kind::Ident && self.raw(token) == b"require") else {
            return false;
        };
        // `require('../moment')` is a call with one string argument. Taking the
        // first string in the next three tokens read `require === 'function'`
        // as an import of the module "function" — the module must be the
        // argument, not a comparison operand.
        let module = match (stmt.get(at + 1), stmt.get(at + 2)) {
            // `require(\`${PROJECT_DIR}/lib/jake\`)` is a path the source builds,
            // not one it states: a template with an interpolation hole has no
            // module name until the program runs, and the CST records no import
            // for it. Reading the cooked text as the module invented fact
            // `import ${PROJECT_DIR}/lib/jake` (measured: jake's rule.js).
            (Some(open), Some(argument))
                if self.is_op(open, b"(") && argument.kind == Kind::Str && argument.interp.is_none() =>
            {
                Some(self.string_value(argument))
            }
            _ => None,
        };
        let Some(module) = module else {
            return false;
        };
        let equal = self.top_level_assign(stmt);
        let rhs_start = equal.map_or(0, |index| index + 1);
        // Do not cross into nested scopes (`=>`, `function`, `class`) on the
        // RHS: `const getX = memoize(() => require("m"))` binds `getX` to
        // `memoize(...)`, not to module `"m"`.
        if at >= rhs_start
            && stmt[rhs_start..at].iter().any(|token| {
                self.is_op(token, b"=>")
                    || (token.kind == Kind::Ident && matches!(self.raw(token), b"function" | b"class"))
            })
        {
            return false;
        }
        // A method body is the same nested scope without the keyword:
        // `get X() { return require("./m") }` written inside an object literal
        // is a getter, and the require is the body's return value, not the
        // value the enclosing statement binds. Read as one statement, the
        // target of `module.exports = mergeExports(resolve, {…})` named the
        // require (alias `exports`), an alias the CST never writes (measured:
        // webpack's index.js).
        if at >= rhs_start && self.crosses_a_body(&stmt[rhs_start..at]) {
            return false;
        }
        let Some(equal) = equal else {
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
            return true;
        };
        // `webidl ??= require('internal/crypto/webidl')`: a conditional or
        // compound assignment updates a value the program already has, so it
        // creates no binding and the target does not name the module. Only `=`
        // initializes one; the CST records the module without an alias here
        // (measured: node's webcrypto.js).
        if !self.is_op(&stmt[equal], b"=") {
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
            return true;
        }
        let targets = &stmt[..equal];
        // `require('npmlog').level = 'warn'` writes a property of the module:
        // the target IS the require call, so `level` is not a binding and the
        // import has no alias. The CST reads it that way; taking the last name
        // of the target invented an alias the source never states.
        if targets.iter().any(|token| token.kind == Kind::Ident && self.raw(token) == b"require") {
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
            return true;
        }
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
        // `using var ms = new MemoryStream();` and `using Socket s = …` are
        // using *statements*, not directives, and `using (…)` is a scope. Read
        // as an import they produced a module nothing could mean
        // (`varms`, `Socketsocket`) and 294 lost rows on one C# corpus alone.
        // A directive is a dotted name, an optional `static` or `alias =`,
        // and nothing else.
        if self.lang.tag == "CS" && word == "using" {
            let mut last_ident = false;
            for token in rest {
                if self.is_op(token, b";") {
                    break;
                }
                if self.is_op(token, b"(") {
                    return false;
                }
                if token.kind == Kind::Ident && self.raw(token) == b"static" {
                    continue;
                }
                if matches!(token.kind, Kind::Ident | Kind::Str) {
                    if last_ident {
                        return false;
                    }
                    last_ident = true;
                } else if matches!(token.kind, Kind::Op) {
                    last_ident = false;
                }
            }
        }
        // `import kotlin.contracts.*` is a wildcard and ends the directive. The
        // statement reader lets the line continue past a trailing `*` (it can
        // open a multiplication), so the next line's string was read as this
        // import's module: `import kotlin.contracts.*` over `val s = "1.1"`
        // filed `import 1.1`, one row per generated stdlib file and per .kts
        // with a star import (359 import losses in corpus45/kotlin).
        let rest = if self.lang.tag == "KT" {
            match rest.iter().position(|token| self.is_op(token, b"*")) {
                Some(end) => &rest[..=end],
                None => rest,
            }
        } else {
            rest
        };
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
                // `using Test = Xunit.FactAttribute;`: the name in front of
                // the `=` is the alias, not the module. Read as a break it left
                // the module as the alias's own name, and every aliased using
                // in the C# corpus disagreed with the tree that reads it right.
                Kind::Op if self.lang.tag == "CS" && !module.is_empty() && alias.is_empty() && self.is_op(token, b"=") => {
                    alias = module.clone();
                    module.clear();
                }
                Kind::Ident if self.raw(token) == b"as" => {
                    alias = self.word_at(rest, index + 1).unwrap_or_default();
                    index += 1;
                }
                // `import static java.util.Objects.requireNonNull`: `static`
                // is a modifier, not the first segment of the path. Pushed as
                // an identifier it glued onto the next word (`staticjava.…`),
                // and every static import in the Java corpus left a record
                // nothing else could mean.
                Kind::Ident
                    if module.is_empty() && matches!(self.text(token).as_str(), "static" | "module") =>
                {
                    if rest.get(index + 1).is_some_and(|next| self.is_op(next, b".")) {
                        index += 1;
                    }
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

    fn strip_outer_parens<'b>(&self, mut toks: &'b [Token]) -> &'b [Token] {
        while toks.len() >= 2
            && toks.first().is_some_and(|t| self.is_op(t, b"("))
            && toks.last().is_some_and(|t| self.is_op(t, b")"))
        {
            let mut depth = 0i32;
            let mut wraps_all = true;
            for (i, token) in toks.iter().enumerate() {
                if token.kind == Kind::Op {
                    match self.raw(token) {
                        b"(" | b"[" | b"{" => depth += 1,
                        b")" | b"]" | b"}" => depth -= 1,
                        _ => {}
                    }
                }
                if depth == 0 && i + 1 < toks.len() {
                    wraps_all = false;
                    break;
                }
            }
            if !wraps_all || depth != 0 {
                break;
            }
            toks = &toks[1..toks.len() - 1];
        }
        toks
    }

    /// Whether `toks` opens a body that is still open at its end — a `{`
    /// directly behind `)` is a method's or a function's statement block, and
    /// the tokens after it belong to that body rather than to the statement
    /// that surrounds it.
    fn crosses_a_body(&self, toks: &[Token]) -> bool {
        let mut depth = 0usize;
        let mut body: Option<usize> = None;
        for (index, token) in toks.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" => depth += 1,
                b")" | b"]" => depth = depth.saturating_sub(1),
                b"{" => {
                    depth += 1;
                    if index > 0 && self.is_op(&toks[index - 1], b")") {
                        body = Some(depth);
                    }
                }
                b"}" => {
                    depth = depth.saturating_sub(1);
                    if body.is_some_and(|open| depth < open) {
                        body = None;
                    }
                }
                _ => {}
            }
        }
        body.is_some()
    }

    fn top_level_assign(&self, toks: &[Token]) -> Option<usize> {
        let mut depth = 0;
        let mut angle = 0i32;
        let mut found = None;
        for (index, token) in toks.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" => depth += 1,
                b")" | b"]" | b"}" => depth -= 1,
                b"<" if depth == 0 => angle += 1,
                b">" if depth == 0 => angle = (angle - 1).max(0),
                b">>" if depth == 0 => angle = (angle - 2).max(0),
                b">>>" if depth == 0 => angle = (angle - 3).max(0),
                b"?" if depth == 0
                    && found.is_none()
                    && !toks.get(index + 1).is_some_and(|t| self.is_op(t, b":")) =>
                {
                    return None
                }
                b"," if depth == 0
                    && angle == 0
                    && matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") =>
                {
                    return None
                }
                // `..=` is read as `.` then `.=`, so the range's `=` arrives as
                // an augmented assignment of a dot. It assigns nothing.
                b".=" if depth == 0
                    && toks.get(index.wrapping_sub(1)).is_some_and(|prev| {
                        prev.kind == Kind::Op && self.raw(prev) == b"." && prev.end == token.start
                    }) =>
                {
                    continue
                }
                // `b'a'..=b'z' | b'[' => 3`: the `=` of a range is not an
                // assignment. Read as one, the arm's pattern became an
                // assignment whose target was the range's dot (8 corpus files).
                b"=" if depth == 0
                    && toks.get(index.wrapping_sub(1)).is_some_and(|prev| {
                        prev.kind == Kind::Op && self.raw(prev).ends_with(b".") && prev.end == token.start
                    }) =>
                {
                    continue
                }
                b"=" | b":=" | b"+=" | b"-=" | b".=" | b"|=" | b"||=" | b"&&=" | b"??=" | b"<-" | b"*=" | b"/=" | b"%="
                | b"&=" | b"^=" | b"<<=" | b">>="
                    if depth == 0
                        && found.is_none()
                        && !(matches!(self.lang.tag, "TS" | "TSX") && angle > 0) =>
                {
                    found = Some(index);
                    angle = 0;
                }
                _ => {}
            }
        }
        found
    }

    fn class_head(&self, toks: &[Token]) -> Option<(String, Vec<String>)> {
        let index = self.skip_modifiers(toks);
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
            if self.lang.tag == "RS" && self.word_at(toks, at).as_deref() == Some("dyn") {
                at += 1;
            }
            // `impl<'a> Reader for &'a Buffer {`: a borrow is not a name. Skip the
            // `&`, its lifetime and `mut` so the block opens the type it
            // implements — the CST names it `Buffer`, and without this the
            // methods inside had no owner at all.
            while self.lang.tag == "RS" {
                let Some(token) = toks.get(at) else { break };
                if self.is_op(token, b"&") {
                    at += 1;
                } else if self.is_op(token, b"'") {
                    at += 2;
                } else if token.kind == Kind::Ident && self.text(token).starts_with('\'') {
                    at += 1;
                } else if token.kind == Kind::Ident && matches!(self.text(token).as_str(), "mut" | "const") {
                    at += 1;
                } else {
                    break;
                }
            }
        }
        // `struct __attribute__ ((__packed__)) hdr { … }`: the attribute is not
        // the struct's name. Read as one it opened a class symbol called
        // `__attribute__` that the tree never writes, and the struct's own
        // members had no owner.
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && matches!(self.word_at(toks, at).as_deref(), Some("__attribute__") | Some("__attribute"))
        {
            at += 1;
            if toks.get(at).is_some_and(|token| self.is_op(token, b"(")) {
                if let Some(close) = self.matching_close(toks, at) {
                    at = close + 1;
                }
            }
        }
        // `struct group *getgrent() { … }`: in C, C++ and Objective-C a
        // definition whose return type is a struct or a class is a function,
        // not a struct with a body. Read as a class, the function's statements
        // belonged to a symbol the CST never writes — every event inside it
        // landed on the module and had to be paired against the whole file
        // (121 events on 68 musl files).
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && toks.get(at + 1..).is_some_and(|rest| rest.iter().any(|token| self.is_op(token, b"(")))
        {
            return None;
        }
        let mut name = self.word_at(toks, at)?;
        // `impl tonic::client::X {`: a path names its type by its last segment.
        // Taking the first one opened a class called `tonic`, a symbol the CST
        // never writes (5 corpus files).
        if self.lang.tag == "RS" {
            let mut scan = at + 1;
            while scan < toks.len() {
                if self.is_op(&toks[scan], b"::") {
                    if let Some(segment) = self.word_at(toks, scan + 1) {
                        name = segment;
                        scan += 2;
                        continue;
                    }
                }
                if self.is_op(&toks[scan], b"<") {
                    let mut depth = 0i32;
                    while scan < toks.len() {
                        if self.is_op(&toks[scan], b"<") {
                            depth += 1;
                        } else if self.is_op(&toks[scan], b">") {
                            depth -= 1;
                            if depth == 0 {
                                scan += 1;
                                break;
                            }
                        }
                        scan += 1;
                    }
                    continue;
                }
                break;
            }
        }
        let mut bases = Vec::new();
        if self.lang.tag == "RS" {
            // Rust states bounds in a header, not inheritance: `struct S<'a, T: Clone>`
            // declares a parameter and its bound, and `where T: Send` states another.
            // Reading every identifier after `<` or `:` as a base made `T`, `Clone`
            // and `Send` bases of the class — three symbols the CST never writes. An
            // `impl` is the one place a name really is inherited: `impl Trait for X`.
            if keyword == "impl" {
                if let Some(for_at) = toks.iter().position(|t| t.kind == Kind::Ident && self.raw(t) == b"for") {
                    if let Some(trait_name) = self.type_before(toks, for_at) {
                        bases.push(trait_name);
                    }
                }
            }
            return Some((name, bases));
        }
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

    /// The type spelled just before `at`, skipping a `<...>` argument list: the
    /// trait in `impl<T: Bound> Trait<U> for Type`. `None` when the tokens before
    /// `at` hold no type — `impl for` is not source any lexer should invent.
    fn type_before(&self, toks: &[Token], at: usize) -> Option<String> {
        let mut scan = at;
        while scan > 0 {
            scan -= 1;
            let token = &toks[scan];
            if self.is_op(token, b">") {
                let mut depth = 1i32;
                while scan > 0 {
                    scan -= 1;
                    if self.is_op(&toks[scan], b">") {
                        depth += 1;
                    } else if self.is_op(&toks[scan], b"<") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                continue;
            }
            if token.kind == Kind::Ident {
                let word = self.text(token);
                if !matches!(word.as_str(), "unsafe" | "const" | "default" | "dyn" | "impl") {
                    return Some(word);
                }
            }
        }
        None
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
            let mut depth = 0i32;
            while let Some(token) = toks.get(at) {
                if self.is_op(token, b"<") {
                    depth += 1;
                } else if self.is_op(token, b">") {
                    depth -= 1;
                    if depth == 0 {
                        return at + 1;
                    }
                } else if self.is_op(token, b">>") || self.is_op(token, b">>>") {
                    // `Readonly<[A, ...B[]]>>(`: two closing angles written
                    // next to each other are one token. Read as neither, the
                    // scan ran past the type and the head of
                    // `static create = <O extends R<Y>>(…) => …` was never
                    // found, so the field was blamed for an assignment the CST
                    // never writes.
                    depth -= if self.is_op(token, b">>") { 2 } else { 3 };
                    if depth <= 0 {
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

    /// `X = class Y { … }` and `module.exports = class …`: a class expression
    /// assigned to a name opens its own class frame on `{`, not a plain block.
    fn assigned_class(&self, stmt: &[Token]) -> Option<(String, Vec<String>)> {
        if !matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX") {
            return None;
        }
        let equal = self.top_level_assign(stmt)?;
        self.assigned_class_head(&stmt[equal + 1..])
    }

    /// `class` as the value of an assignment: the name it spells, if any,
    /// and its bases. `= class extends B {` is anonymous — the CST gives that
    /// class no name of its own, so neither does the lexer.
    fn assigned_class_statement(&self, stmt: &[Token]) -> Option<(String, Vec<String>)> {
        if !matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX") {
            return None;
        }
        let equal = self.top_level_assign(stmt)?;
        self.assigned_class_head(&stmt[equal + 1..])
    }

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
        // `export const g: (i: number) => number = (i: number): number => i`:
        // the annotation is a function type, so the token before the `=` is its
        // return type and the walk back over `name : T` pairs cannot reach the
        // name — `=>` is not a separator. The name is whatever the annotation's
        // own colon follows.
        if matches!(self.lang.tag, "TS" | "TSX") {
            let mut depth = 0i32;
            let mut angle = 0i32;
            let mut colon = None;
            let mut arrow = false;
            for (index, token) in toks[..equal].iter().enumerate() {
                if token.kind != Kind::Op {
                    continue;
                }
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    b"<" if depth == 0 => angle += 1,
                    b">" if depth == 0 => angle = (angle - 1).max(0),
                    b":" if depth == 0 && angle == 0 && colon.is_none() => colon = Some(index),
                    b"=>" if depth == 0 && angle == 0 => {
                        arrow = true;
                        break;
                    }
                    _ => {}
                }
            }
            if arrow {
                if let Some(colon) = colon {
                    if let Some(named) = toks[..colon].iter().rposition(|token| token.kind == Kind::Ident) {
                        start = named;
                    }
                }
            }
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
                head.assigned_name = true;
            }
            return Some(head);
        }
        let param_start = self.skip_generics(rhs, skip);
        let arrow = match rhs.get(param_start) {
            Some(token) if token.kind == Kind::Ident && param_start == skip => {
                if rhs.get(param_start + 1).is_some_and(|next| self.is_op(next, b"=>")) {
                    param_start + 1
                } else {
                    return None;
                }
            }
            Some(token) if self.is_op(token, b"(") => {
                let (_, close) = self.group_at(rhs, param_start)?;
                let after = close + 1;
                if rhs.get(after).is_some_and(|next| self.is_op(next, b"=>")) {
                    after
                } else if rhs.get(after).is_some_and(|next| self.is_op(next, b":")) {
                    let mut depth = 0i32;
                    let mut found = None;
                    for (index, token) in rhs.iter().enumerate().skip(after + 1) {
                        if token.kind == Kind::Op {
                            match self.raw(token) {
                                b"(" | b"[" | b"{" => depth += 1,
                                b")" | b"]" | b"}" => depth -= 1,
                                b"?" if depth == 0 => return None,
                                b"=>" if depth == 0 => {
                                    found = Some(index);
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    found?
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        let params = self.arrow_params(&rhs[param_start..arrow]);
        let body = equal + 1 + arrow + 1;
        Some(Head { name, owner, params, body: Some(body), assigned_name: true, ..Head::default() })
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
        let js_class = matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX") && self.top().kind == FrameKind::Class;
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
                b"[" if js_class => depth += 1,
                b"]" if js_class => depth -= 1,
                b":" if depth == 0 && !groups.is_empty() => break,
                _ => {}
            }
        }
        // `pub(super) fn parse_with(..)`: the visibility group is also a group
        // at depth zero, and taking it as the parameter list named the function
        // `pub` — or, once `pub` was refused, nothing at all. The head's group
        // is the first one that has a usable name in front of it.
        // In a JS/TS class body, `*[Symbol.iterator]()` has `]` before `(`.
        let open = *groups.iter().find(|&&open| {
            open > 0
                && (self
                    .word_at(toks, open - 1)
                    .is_some_and(|name| !CONTROL.contains(&name.as_str()) && !NOT_A_NAME.contains(&name.as_str()))
                    || (js_class && self.is_op(&toks[open - 1], b"]")))
        })?;
        let name_at = open - 1;
        let computed = js_class && self.is_op(&toks[name_at], b"]");
        let name = if computed {
            String::new()
        } else {
            let name = self.word_at(toks, name_at)?;
            if CONTROL.contains(&name.as_str()) || NOT_A_NAME.contains(&name.as_str()) {
                return None;
            }
            name
        };
        if self.head_word(toks).is_some_and(|word| CONTROL.contains(&word.as_str())) {
            return None;
        }
        // `go func() { … }()` and `defer f.Close()`: the Go statement keyword
        // comes first and the call follows, so `go func() {` is the shape of a
        // declaration — return type `go`, name `func`, parameters `()` — and it
        // opened a symbol called `func` that held the literal's body. A `go`,
        // `defer`, `range` or `select` in front makes the line a statement.
        if self.lang.tag == "GO"
            && self
                .head_word(toks)
                .is_some_and(|word| matches!(word.as_str(), "go" | "defer" | "range" | "select"))
        {
            return None;
        }
        // In JS/TS, a keyword-less function header before `{` is only a method
        // definition: never a dotted call (`t.advance(2)`), and nothing other
        // than a TS `: ReturnType` can sit between `)` and `{`.
        if !computed
            && matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX")
            && name_at > 0
            && self.is_op(&toks[name_at - 1], b".")
        {
            return None;
        }
        let owner = if computed {
            String::new()
        } else {
            let mut chain_start = name_at;
            while chain_start >= 2
                && toks[chain_start - 1].kind == Kind::Op
                && matches!(self.raw(&toks[chain_start - 1]), b"::" | b".")
                && toks[chain_start - 2].kind == Kind::Ident
            {
                chain_start -= 2;
            }
            self.name_chain(toks, chain_start).1
        };
        let (inner, close) = self.group_at(toks, open)?;
        // After the parameter list a method head can only be followed by its
        // body `{` or a TS return-type `:` — the latter also appears in a `.js`
        // file, where a type annotation is not a declaration but is still the
        // text after the head. Anything else is a call, not a definition.
        if matches!(self.lang.tag, "JS" | "JSX" | "TS" | "TSX")
            && close + 1 < toks.len()
            && !self.is_op(&toks[close + 1], b":")
            && !self.is_op(&toks[close + 1], b"{")
        {
            return None;
        }
        let params = self.params(&toks[inner..close]);
        Some(Head { name, owner, params, ..Head::default() })
    }

    fn function_head(&self, toks: &[Token], at_block: bool) -> Option<Head> {
        let start = self.skip_rs_attrs(toks);
        let index = start + self.skip_modifiers(&toks[start..]);
        if let Some(word) = self.word_at(toks, index) {
            if self.lang.defs.contains(&word.as_str()) {
                return self.keyword_head(toks, index);
            }
        }
        if let Some(head) = self.assigned_head(toks) {
            return Some(head);
        }
        (at_block && self.uses_braces()).then(|| self.c_style_head(&toks[start..])).flatten()
    }

    /// A statement of constants, braces and commas alone: no name, no call,
    /// no operator that could make it a declaration. Only a file-scope
    /// initializer fragment reads that way in C, and it states no fact.
    fn constant_fragment(&self, stmt: &[Token]) -> bool {
        let mut constants = 0;
        for token in stmt {
            match token.kind {
                Kind::Num | Kind::Str => constants += 1,
                Kind::Op if matches!(self.raw(token), b"," | b"{" | b"}" | b"(" | b")") => {}
                Kind::Newline => {}
                _ => return false,
            }
        }
        constants > 0
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
        // A Java member declares; it never assigns by expression. An enum
        // constant with arguments (`ALL(Integer.MIN_VALUE),`) and an annotation
        // with arguments (`@SuppressWarnings("unchecked")`) are declarations,
        // and the CST writes no value for either — read as expressions they
        // were a call per constant and a call per annotation (java: 184 and 13
        // files). A member that binds nothing has no event; a field with an
        // initialiser has its `=` and keeps its own.
        if self.lang.tag == "JAVA"
            && self.top().kind == FrameKind::Class
            && !stmt.iter().any(|token| self.is_op(token, b"="))
        {
            return;
        }
        // `#![allow(unused)]` and a `#[derive(Debug)]` written on its own line
        // are layout: the CST writes nothing for them. Read as a value, the inner
        // attribute became the call `allow(unused)` on the module symbol. A
        // statement that is only attributes therefore writes nothing (P5-13).
        if self.lang.tag == "RS" {
            let after = self.skip_rs_attrs(stmt);
            if after > 0 && after >= stmt.len() {
                return;
            }
        }
        // `@Test` on a line of its own annotates the declaration below it and
        // states no value: the CST files an annotation's event only when the
        // annotation carries arguments (`@Suppress("x")`), so reading the bare
        // name as an expression invented one `x` event per annotated
        // declaration - 765 corpus45/kotlin files, the largest kotlin cluster
        // in the shape table.
        if self.lang.tag == "KT" && self.kotlin_annotation_alone(stmt) {
            return;
        }
        // `init { }`: an initializer block is a member of the class, not a
        // value. The brace opens the frame this statement is the head of, so
        // the keyword arrives alone and the block's own statements are read as
        // the class's. Read as an expression it wrote `n:init`, an event the
        // CST never has.
        if self.lang.tag == "KT" && self.kotlin_declares_only(stmt) {
            return;
        }
        // `_intercepted` / `?: (context[ContinuationInterceptor]?.…)` and
        // `this === other || other is Instant` / `&& this.nanosecondsOfSecond
        // == other.nanosecondsOfSecond`: a line that opens with the operator
        // continuing the statement above it belongs to that statement. The CST
        // folds it in and writes no event, so reading it as a statement of its
        // own invented one `x` per continuation (24 rows of the kotlin dump).
        if self.lang.tag == "KT" && self.kotlin_continues(stmt) {
            return;
        }
        // `A,` / `B;` / `}`: an enum's entry list arriving as one statement,
        // because the `;` that closes it ends the statement the entry lines
        // began. No Kotlin statement is a bare comma-separated list of names,
        // and the expression reader wrote it as a tuple the CST never writes
        // (`KmVersionRequirementVersionKind` and the rest of the `Jvm,` rows).
        if self.lang.tag == "KT" && self.kotlin_entry_list(stmt) {
            return;
        }
        // `context(KaSession)` in front of a declaration is a receiver list,
        // not a call: the CST reads it as the declaration's context and writes
        // nothing, while the expression reader filed `c context`.
        if self.lang.tag == "KT" && self.kotlin_context_head(stmt) {
            return;
        }
        // A C header's data tables are fragments of an initializer, not
        // statements. musl includes `ctype/alpha.h` inside a declaration
        // (`static const unsigned char table[] = {` … `};`) and
        // `locale/gb18030utf.h` inside an initializer list, so the file on its
        // own is one long run of constants (`alpha.h`: 3,904 of them) or one
        // run per table row (`{ 0x80, 36 },`). A run with no name and no call
        // states no fact; the CST, whose translation unit holds declarations,
        // writes none. Read as a statement it made every table row a loss
        // (240 events in 34 corpus45/c files, `no event on a line both readers
        // can mean`). A brace at file scope opens a block frame whose symbol is
        // still the module, so the module symbol, not the frame kind, is what
        // says the line is file scope.
        if self.top().symbol == 0 && matches!(self.lang.tag, "C" | "CPP" | "OBJC") && self.constant_fragment(stmt) {
            return;
        }
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
        // `18,17,19,…` and `{ 0x80, 36 },`: a file that is only a table's rows is
        // a data fragment, not a translation unit - musl includes these in the
        // brace list that declares the table. The CST reads no statement there,
        // and read as an expression the whole fragment became one event of
        // constants that the source never opened (13 c files, 218 rows). A
        // statement with no word in it cannot declare or bind anything.
        // The braces of `{ 0x80, 36 },` open a block in this reader, so the
        // rule is the statement's shape, not the frame it sits in.
        if self.lang.tag == "C" {
            let mut numbers = 0usize;
            let fragment = stmt.iter().all(|token| match token.kind {
                Kind::Num => {
                    numbers += 1;
                    true
                }
                Kind::Str => true,
                Kind::Op => matches!(&self.src[token.start as usize..token.end as usize], b"," | b"{" | b"}" | b"[" | b"]"),
                _ => false,
            });
            if fragment && numbers > 0 {
                return;
            }
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
        // `impl<CT, RT> Handle<CT, RT> where ... {`: an item head writes no
        // value. The brace did not open a body (the header is not a block to this
        // reader), so the header was read as an expression and invented an event
        // the CST never writes.
        if self.lang.tag == "RS" {
            let index = self.skip_modifiers(stmt);
            if let Some(word) = self.word_at(stmt, index) {
                if matches!(word.as_str(), "struct" | "enum" | "impl" | "trait" | "mod" | "union" | "use" | "extern") {
                    return;
                }
            }
        }
        // `goto out;` jumps to a label and states no value: the CST writes no
        // event for a `goto_statement`, so reading the label as an expression
        // invented one on every cleanup jump in the C corpus. A computed
        // `goto *ptr;` is the same statement with an expression for a target.
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && self.word_at(stmt, 0).as_deref() == Some("goto")
            && stmt.len() >= 2
        {
            return;
        }
        // `extern "C" {` opens a linkage block and states no value: the CST's
        // `linkage_specification` header writes no event, so the words form the
        // lexer invented here (`m:[n:extern,0]` — a keyword and a zero) had no
        // counterpart and cost every `#ifdef __cplusplus` header in the corpus.
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && self.word_at(stmt, 0).as_deref() == Some("extern")
            && stmt.get(1).is_some_and(|token| token.kind == Kind::Str)
            && stmt[2..]
                .iter()
                .all(|token| token.kind == Kind::Op && matches!(self.raw(token), b"{" | b"}"))
        {
            return;
        }
        let word = self.word_at(stmt, 0);
        if matches!(word.as_deref(), Some("return")) {
            self.expression_event("r", &stmt[1..]);
            return;
        }
        let line = first.line;
        // A let-chain whose condition continues on the next line arrives as a
        // statement of its own: `&& let Some(old) = w.pop_front()`. Read as an
        // assignment, the callee became a call the CST never writes (15 files).
        if self.lang.tag == "RS"
            && stmt.len() > 2
            && stmt[0].kind == Kind::Op
            && matches!(self.raw(&stmt[0]), b"&&" | b"||")
        {
            let kept = self.strip_let_patterns(stmt, false);
            if kept.len() != stmt.len() {
                self.expression_event("x", &kept);
                return;
            }
        }
        // `for (var t = arguments.length, o = new Array(t), i = 1; i < t; i++) …`:
        // a loop header declares its binding even when the body is a bare
        // statement on the same line. Read as an assignment the whole header
        // became one tuple value of the first binding.
        if self.word_at(stmt, 0).as_deref() == Some("for") {
            if let Some(end) = self.matching_close(stmt, 1) {
                self.loop_header(&stmt[2..end], line);
                if end + 1 < stmt.len() {
                    self.simple_statement(&stmt[end + 1..]);
                }
                return;
            }
        }
        // `LUALIB_API void (luaL_setn) (lua_State *L, int t, int n);`: the
        // declared name is parenthesised so the macro over it cannot expand,
        // which is a declaration header and carries no value. Read as one, the
        // lexer wrote the name on its own line and the CST wrote the call
        // group, and every lua header lost the declaration (54 rows). A
        // parenthesised callee (`(f)(x);`) opens with the `(`; this one opens
        // with the type, so only a declaration head reaches it.
        if self.lang.tag == "C" && self.parenthesised_declarator(stmt) {
            return;
        }
        if let Some(equal) = self.top_level_assign(stmt) {
            let augmented = self.raw(&stmt[equal]) != b"=" && !matches!(self.raw(&stmt[equal]), b":=" | b"<-");
            let value = self.c_initialiser(&stmt[..equal], &stmt[equal + 1..]);
            self.note_class(&stmt[..equal], value);
            self.assignment(&stmt[..equal], value, line, augmented);
        } else if !(self.lang.tag == "JAVA" && self.java_declares_only(stmt))
            && !(self.lang.tag == "SWIFT" && self.swift_declares_only(stmt))
            && !(self.lang.tag == "KT" && self.kotlin_declares_only(stmt))
        {
            let expr = if self.lang.tag == "KT"
                && stmt.len() > 3
                && self.is_op(&stmt[0], b"@")
                && stmt[1].kind == Kind::Ident
                && self.is_op(&stmt[2], b":")
            {
                &stmt[3..]
            } else {
                stmt
            };
            self.expression_event("x", expr);
        }
    }

    /// A Swift attribute run on a line of its own (`@inlinable`,
    /// `@available(SwiftStdlib 6.0, *)`, `@_exported`) decorates the
    /// declaration under it and binds no value: the CST marks the following
    /// declaration with its modifiers and writes no event of its own for the
    /// attribute, so reading the attribute's name as an expression invented an
    /// `n:inlinable` on 123 lines. A statement that carries a declaration
    /// keyword is the declaration itself (`@objc func f()`) and keeps its
    /// event.
    fn swift_declares_only(&self, stmt: &[Token]) -> bool {
        stmt.first().is_some_and(|token| self.is_op(token, b"@"))
    }

    /// A Java member that declares and assigns nothing: a signature with no
    /// body (`V apply(K key);`, `public abstract void doIt();`, a `native` or an
    /// interface method), the modifier that opens an initializer block
    /// (`static {`), and an annotation written on its own line. The CST writes
    /// no event for any of them — a declaration is not an assignment — so
    /// reading each as an expression invented `x` events the other reader never
    /// had: 4,961 events over 1,204 files, the largest java cluster in the
    /// shape table.
    /// A statement that is nothing but annotations: `@Test`, `@file:JvmName`,
    /// `@org.junit.Test`. Kotlin files each as a call of its name, and the CST
    /// writes that call's event only when the annotation has an argument list,
    /// so a bare name on its own line states nothing both readers can agree on.
    fn kotlin_annotation_alone(&self, stmt: &[Token]) -> bool {
        if !stmt.first().is_some_and(|token| self.is_op(token, b"@")) {
            return false;
        }
        stmt.iter().all(|token| match token.kind {
            Kind::Ident => true,
            Kind::Op => matches!(self.raw(token), b"@" | b":" | b"."),
            _ => false,
        })
    }

    /// A Kotlin member that declares and assigns nothing: `public val log:
    /// Logger`, `public fun f(): Int`, `companion object`, `abstract val x:
    /// Int`, `internal lateinit var y: Channel`, `init {`. The CST writes no
    /// event for any of them — a declaration header is not a value — so reading
    /// each as an expression invented the event the other reader never had, and
    /// every interface member, companion object and initializer block in the
    /// corpus lost its line.
    /// A statement with a top-level `=` never reaches this: it is an
    /// assignment, and the initialiser is read as one.
    /// `context(KaSession)`: the receiver list in front of a declaration.
    fn kotlin_context_head(&self, stmt: &[Token]) -> bool {
        self.word_at(stmt, 0).as_deref() == Some("context")
            && stmt.get(1).is_some_and(|token| self.is_op(token, b"("))
    }

    /// A statement that is only names separated by commas and closed by `;`:
    /// the entry list of an enum, which the CST reads as entries and never as
    /// a comma expression.
    fn kotlin_entry_list(&self, stmt: &[Token]) -> bool {
        let mut names = 0usize;
        let mut comma = false;
        for token in stmt {
            if self.is_op(token, b",") {
                comma = true;
                continue;
            }
            if self.is_op(token, b";") {
                continue;
            }
            if token.kind != Kind::Ident {
                return false;
            }
            names += 1;
        }
        comma && names > 1
    }

    /// A line that opens with the operator continuing the statement above it:
    /// `?:`, `&&`, `||`, a chained `.`, or the `?` of a safe call. Such a line
    /// states no value of its own.
    fn kotlin_continues(&self, stmt: &[Token]) -> bool {
        stmt.first().is_some_and(|token| {
            token.kind == Kind::Op && matches!(self.raw(token), b"?" | b"?." | b"&&" | b"||" | b".")
        })
    }

    fn kotlin_declares_only(&self, stmt: &[Token]) -> bool {
        let mut index = 0;
        // `@SinceKotlin("1.1") public actual typealias Error = java.lang.Error`:
        // the annotation in front of a declaration is not a name, and reading
        // the line as an assignment invented the `a Error = java.lang.Error`
        // row the CST never writes (15 rows of the kotlin dump; the same line
        // without the annotation was already a declaration). The annotation's
        // dotted name, its use-site target (`@get:`) and its argument list are
        // one item, and there may be more than one.
        while index < stmt.len() {
            let token = &stmt[index];
            if self.is_op(token, b"@") {
                index += 1;
                while index < stmt.len() && stmt[index].kind == Kind::Ident {
                    index += 1;
                    if index < stmt.len() && self.is_op(&stmt[index], b":") {
                        index += 1;
                        continue;
                    }
                    if index < stmt.len() && self.is_op(&stmt[index], b".") {
                        index += 1;
                        continue;
                    }
                    break;
                }
                if index < stmt.len() && self.is_op(&stmt[index], b"(") {
                    let mut depth = 0usize;
                    while index < stmt.len() {
                        if self.is_op(&stmt[index], b"(") {
                            depth += 1;
                        } else if self.is_op(&stmt[index], b")") {
                            depth -= 1;
                        }
                        index += 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                continue;
            }
            if token.kind == Kind::Ident {
                let word = self.text(token);
                if KOTLIN_MODIFIERS.contains(&word.as_str()) {
                    index += 1;
                    continue;
                }
                return matches!(
                    word.as_str(),
                    "val" | "var" | "fun" | "class" | "interface" | "object" | "companion" | "enum" | "typealias" | "init"
                );
            }
            return false;
        }
        false
    }

    fn java_declares_only(&self, stmt: &[Token]) -> bool {
        let Some(first) = stmt.first() else { return false };
        let word = |token: &Token| if token.kind == Kind::Ident { Some(self.text(token)) } else { None };
        // A type declaration is a value, and the CST writes one for it:
        // `public abstract class A {` is not this rule.
        if stmt.iter().any(|token| {
            matches!(word(token).as_deref(), Some("class" | "interface" | "enum" | "record"))
        }) {
            return false;
        }
        // `new X(...)` is a value even when no name takes it.
        if stmt.iter().any(|token| word(token).as_deref() == Some("new")) {
            return false;
        }
        // `static {` arrives as the modifier alone, the brace having opened the
        // block this statement is the head of.
        if stmt.len() == 1 && word(first).as_deref().is_some_and(java_modifier) {
            return true;
        }
        // `@Override` on a line of its own declares nothing the runtime assigns.
        if first.kind == Kind::Op && self.raw(first) == b"@" {
            return true;
        }
        // `void f() throws IOException`: the throws clause follows the params.
        let end = stmt
            .iter()
            .position(|token| word(token).as_deref() == Some("throws"))
            .unwrap_or(stmt.len());
        if end == 0 || !stmt.get(end - 1).is_some_and(|token| self.is_op(token, b")")) {
            return false;
        }
        let Some(open) = (0..end).find(|index| self.is_op(&stmt[*index], b"(")) else {
            return false;
        };
        // `V apply(` has the return type and the name in front of the parameter
        // list; `go(` has only the name and is a call.
        if open < 2 || stmt[open - 1].kind != Kind::Ident {
            return false;
        }
        // A receiver makes it a call: `System.out.println(x)`, `this.go(x)`.
        if stmt[..open].iter().any(|token| self.is_op(token, b".")) {
            return false;
        }
        // The head before the name states a type, so every word in it is a
        // modifier, a primitive or a type name — never a value.
        stmt[..open - 1].iter().all(|token| match token.kind {
            Kind::Ident => {
                let word = self.text(token);
                java_modifier(&word)
                    || matches!(
                        word.as_str(),
                        "int" | "long" | "short" | "byte" | "char" | "boolean" | "float" | "double" | "void"
                            | "extends" | "super"
                    )
                    || word.chars().next().is_some_and(|ch| ch.is_ascii_uppercase())
            }
            Kind::Op => matches!(self.raw(token), b"<" | b">" | b"[" | b"]" | b"," | b"?"),
            _ => false,
        })
    }

    /// `repo = new Repo()` and `repo = Repo()` say what `repo.find` means.
    fn note_class(&mut self, targets: &[Token], value: &[Token]) {
        // `array_container_t *c_qua_array = CAST_array(c);` states the type in
        // the declaration, and a pointer declarator's binding is the specifier:
        // the C walk records no class for it, and the lexer's was the macro's
        // name, which is not a class at all (`CAST_array` is a #define). The
        // same reading caught `*type = CAST_shared(c)->typecode;`, whose target
        // is a dereference and whose class is the typecode's, not the cast's
        // (8 rows on the croaring corpus).
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && targets
                .iter()
                .any(|token| self.is_op(token, b"*") || self.is_op(token, b"&"))
        {
            return;
        }
        // `recv, send = RecvChannel(cid), SendChannel(cid)`: the class belongs
        // to the value in the same position. Read as one tuple, the last
        // target took the first value's class — `send = RecvChannel` — which
        // is a fact the source does not state. A target list longer than the
        // value list states no per-name class at all, so it names none.
        let target_parts = self.top_commas(targets);
        if target_parts.len() > 1 {
            let value_parts = self.top_commas(value);
            if target_parts.len() == value_parts.len() {
                for (name, expr) in target_parts.iter().zip(value_parts.iter()) {
                    self.note_class(name, expr);
                }
            }
            return;
        }
        // Parentheses are typing, not construction: `let d = (Value::Object(m))`
        // holds the variant exactly like the unparenthesised form, and the Rust
        // CST reads both the same way. The other grammars keep the parentheses
        // in the tree, so unwrapping there would invent a binding the CST does
        // not have (measured: `var x = (GetIntrinsic(n))` in JS).
        let mut value = value;
        if matches!(self.lang.tag, "Rust") {
            loop {
                if !value.first().is_some_and(|token| self.is_op(token, b"(")) {
                    break;
                }
                let Some((inner, close)) = self.group_at(value, 0) else { break };
                if close + 1 != value.len() || inner == close || self.top_commas(value).len() > 1 {
                    break;
                }
                value = &value[inner..close];
            }
        }
        let mut at = 0;
        let explicit = self.word_at(value, 0).as_deref() == Some("new");
        if explicit {
            at = 1;
        }
        let Some(class) = self.word_at(value, at) else { return };
        let (_, owner, next) = self.name_chain(value, at);
        let called = value.get(next).is_some_and(|token| self.is_op(token, b"("));
        // A call that the value then continues past does not leave the variant
        // in the binding: `Value::Object(m).to_string()` holds a String and
        // `Value::Object(m)?` unwraps it, so neither names the variant. C
        // reaches the rest of the value through `->` as well: `mt =
        // G(L)->mt[i]` holds a table slot, not a `G`, and reading it as one
        // bound four lua files to a macro name.
        if called {
            if let Some((_, close)) = self.group_at(value, next) {
                if value.get(close + 1).is_some_and(|token| {
                    self.is_op(token, b".")
                        || self.is_op(token, b"?")
                        || self.is_op(token, b"[")
                        || self.is_op(token, b"->")
                }) {
                    return;
                }
            }
        }
        let capital = class.starts_with(|ch: char| ch.is_uppercase());
        let name = if owner.is_empty() { class } else { self.name_chain(value, at).0 };
        if called && (explicit || capital) && name.starts_with(|ch: char| ch.is_uppercase()) {
            let target = self.target_path(targets);
            if let Some(target) = target {
                // A C# declarator names one identifier; a dotted target is a
                // member being assigned, not a variable being declared.
                // `sock.LingerState = new LingerOption(…)` bound `sock.LingerState`
                // to `LingerOption` and the CST, which never declares that name,
                // lost a variable for every such line.
                if !(self.lang.tag == "CS" && target.contains('.')) {
                    self.facts.vars.push((target, name));
                }
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

    /// `declare function f(...): { ... }`: an ambient function declaration has
    /// no body; its braced return type is a type body, not runtime statements.
    fn is_declare_function(&self, stmt: &[Token]) -> bool {
        if !stmt.last().is_some_and(|token| self.is_op(token, b":")) {
            return false;
        }
        let mut has_declare = false;
        let mut at = 0;
        while let Some(word) = self.word_at(stmt, at) {
            if self.lang.defs.contains(&word.as_str()) {
                return has_declare;
            }
            if !MODIFIERS.contains(&word.as_str()) {
                break;
            }
            if word == "declare" {
                has_declare = true;
            }
            at += 1;
        }
        false
    }

    /// `declare var M: typeof globalThis extends { onmessage: any; M: infer T }
    /// ? T : U`: the brace opens a conditional type's object type, not a block.
    /// Read as a block, its members became statements and the lexer wrote three
    /// events on the continuation line that the CST never writes.
    fn conditional_type_head(&self, stmt: &[Token]) -> bool {
        let mut seen_typeof = false;
        for at in 0..stmt.len() {
            match self.word_at(stmt, at).as_deref() {
                Some("typeof") => seen_typeof = true,
                Some("extends") if seen_typeof => return true,
                _ => {}
            }
        }
        false
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
                b">>" => depth -= 2,
                b">>>" => depth -= 3,
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
            // The subscript that ends the assignment, not the first `[` in the
            // target: `suite[0].children[a:b] = v` writes `suite[0].children`,
            // so the bracket to cut at is the one matching the final `]` and
            // the receiver walk continues from there.
            let mut depth = 0usize;
            let mut open = None;
            for index in (0..toks.len()).rev() {
                if self.is_op(&toks[index], b"]") {
                    depth += 1;
                } else if self.is_op(&toks[index], b"[") {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(index);
                        break;
                    }
                }
            }
            at = open?;
            if at > 0
                && (at == 1 || !self.is_join(&toks[at - 2]))
                && matches!(self.raw(&toks[at - 1]), b"const" | b"let" | b"var" | b"mut")
            {
                return None;
            }
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
                        if is_call || !self.is_receiver_group(&toks[index + 1..start - 1]) {
                            break;
                        }
                        start = index;
                    }
                    None => break,
                }
                continue;
            }
            // `elements[i].innerHTML`: the subscript belongs to the receiver,
            // so the path continues before its `[`. Stopping at the `]` named
            // the target `.innerHTML` and the assignment lost its receiver.
            if self.is_op(&toks[start - 1], b"]") {
                let mut depth = 0usize;
                let mut index = start - 1;
                loop {
                    if self.is_op(&toks[index], b"]") {
                        depth += 1;
                    } else if self.is_op(&toks[index], b"[") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    if index == 0 {
                        break;
                    }
                    index -= 1;
                }
                // `byte[] keyBytes = new byte[n]`: an empty bracket straight
                // after a name is Java's array type suffix, not a subscript —
                // the path starts at the declarator that follows it. Read
                // across it, the type was pasted onto the target
                // (`bytekeyBytes`, `longLONG_ARRAY_NOT_SUPPORTED`), the one
                // java shape the CST never writes: 612 rows over 265 files.
                // C# spells an array type the same way, and pasted it on
                // (`byte[] bytes = Encoding.GetBytes(s)` bound `bytebytes`).
                if matches!(self.lang.tag, "JAVA" | "CS")
                    && toks.get(index + 1).is_some_and(|token| self.is_op(token, b"]"))
                    && index > 0
                    && toks[index - 1].kind == Kind::Ident
                {
                    break;
                }
                start = index;
                continue;
            }
            let token = &toks[start - 1];
            let joins = token.kind == Kind::Op && matches!(self.raw(token), b"." | b"->" | b"::");
            if token.kind == Kind::Ident
                && (start == at || self.raw(&toks[start]) != b".")
                && start != at
                && !self.is_op(&toks[start], b"[")
            {
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
        let mut at_token = start;
        while at_token < at {
            let token = &toks[at_token];
            // A subscript names no part of the path, but its receiver does.
            if token.kind == Kind::Op && self.raw(token) == b"[" {
                let mut depth = 0usize;
                while at_token < at {
                    if self.is_op(&toks[at_token], b"[") {
                        depth += 1;
                    } else if self.is_op(&toks[at_token], b"]") {
                        depth -= 1;
                        if depth == 0 {
                            at_token += 1;
                            break;
                        }
                    }
                    at_token += 1;
                }
                continue;
            }
            // `self.rcvs.lock().await.inner = ..`: `.await` is a postfix
            // operator, not a member, and the CST names the target `.inner`.
            // Spelled as a segment the target was `.await.inner`, a place the
            // CST never writes.
            if self.lang.tag == "RS"
                && self.is_op(token, b".")
                && toks.get(at_token + 1).is_some_and(|next| next.kind == Kind::Ident && self.raw(next) == b"await")
            {
                at_token += 2;
                continue;
            }

            // `(x as any).async = v`: the cast's type is erased at runtime, so
            // the binding is `x.async`. The type is made of names like any
            // other, so the path took them and named the target
            // `xasany.async` — a name the source never has and the CST never
            // writes. Everything from `as` / `satisfies` to the closing paren
            // of the group it sits in is the type, and none of it is a segment.
            if matches!(self.lang.tag, "TS" | "TSX")
                && token.kind == Kind::Ident
                && matches!(self.text(token).as_str(), "as" | "satisfies")
                && !(at_token > start && self.is_join(&toks[at_token - 1]))
            {
                at_token += 1;
                let mut depth = 0usize;
                while at_token < at {
                    let inner = &toks[at_token];
                    if inner.kind == Kind::Op {
                        match self.raw(inner) {
                            b"(" | b"[" | b"{" => depth += 1,
                            b")" | b"]" | b"}" => {
                                if depth == 0 {
                                    break;
                                }
                                depth -= 1;
                            }
                            _ => {}
                        }
                    }
                    at_token += 1;
                }

                continue;
            }
            at_token += 1;
            if token.kind == Kind::Ident {
                let word = self.text(token);
                // `#cache` is the syntactic spelling of the private name
                // `cache`; the CST names the binding, never the sigil, so a
                // private field recorded with its `#` was a var the CST does
                // not have.
                let word = if matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") {
                    word.trim_start_matches('#').to_string()
                } else {
                    word
                };
                let receiver = self.top().receiver.as_str();
                // `this.inner` and `this[key]` are receivers: the CST names them
                // `self.inner` / `self[...]`. A bare `this` is a binding of its
                // own - Rust's `let this = self.project();` binds `this` - and
                // the CST writes `this` there, so mapping it named the target
                // after the receiver instead (000999 line 881).
                let this_is_a_receiver = word == "this"
                    && (self.lang.tag != "RS"
                        || toks
                            .get(at_token)
                            .is_some_and(|next| self.is_op(next, b".") || self.is_op(next, b"[")));
                if this_is_a_receiver || (!receiver.is_empty() && word == receiver) {
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

    /// True when a parenthesised group's top-level tokens form a path/call chain
    /// rather than an assignment or compound expression like `(a = b).c = d`.
    fn is_receiver_group(&self, inner: &[Token]) -> bool {
        if inner.is_empty() {
            return false;
        }
        let mut i = 0;
        while i < inner.len() {
            let token = &inner[i];
            if token.kind == Kind::Ident || self.is_join(token) {
                i += 1;
            } else if self.is_op(token, b"<") && matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX") {
                // `(x as Record<string, unknown>).window`: a TypeScript type
                // argument list is one nested unit, and its commas separate
                // type arguments rather than paths. Read as a comparison the
                // group was rejected and the receiver's own name was lost, so
                // the target was the member alone (`.window`).
                let mut depth = 0usize;
                while i < inner.len() {
                    if self.is_op(&inner[i], b"<") {
                        depth += 1;
                    } else if self.is_op(&inner[i], b">") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if self.is_op(&inner[i], b";") {
                        return false;
                    }
                    i += 1;
                }
                if depth != 0 {
                    return false;
                }
                i += 1;
            } else if self.is_op(token, b"(") || self.is_op(token, b"[") {
                let (open, close) = if self.is_op(token, b"(") { (b"(", b")") } else { (b"[", b"]") };
                let mut depth = 0usize;
                while i < inner.len() {
                    if self.is_op(&inner[i], open) {
                        depth += 1;
                    } else if self.is_op(&inner[i], close) {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                if depth != 0 {
                    return false;
                }
                i += 1;
            } else {
                return false;
            }
        }
        true
    }

    /// The words that introduce a binding, per language. A word is stripped
    /// from a target list only where the language has it as a keyword: in
    /// Python `var, callback = self.untraced.pop()` binds two names, and
    /// stripping `var` there lost the first target. `val` belongs to
    /// Kotlin/Scala alone, so `val[i] = x` in JS stays a subscript assignment.
    fn decl_word(tag: &str, word: &str) -> bool {
        match tag {
            "JS" | "TS" | "JSX" | "TSX" | "GO" | "CS" | "NIM" | "ZIG" | "SWIFT" | "DART" => {
                matches!(word, "const" | "let" | "var")
            }
            "KT" | "SCALA" => matches!(word, "const" | "val" | "var"),
            "PAS" | "GROOVY" => matches!(word, "const" | "var"),
            "RS" => matches!(word, "const" | "let" | "mut"),
            "PL" | "RB" => matches!(word, "my" | "our" | "local"),
            "SH" => matches!(word, "local"),
            _ => false,
        }
    }

    /// The declarators of a Go `var`/`const` statement, each reduced to its
    /// binder: `var W int = 1` binds `W`, `const lo, hi = 3, 70` binds both.
    fn go_declarators(&self, targets: &[Token]) -> Option<Vec<Token>> {
        if self.lang.tag != "GO" {
            return None;
        }
        let head = targets.first()?;
        if head.kind != Kind::Ident || !matches!(self.raw(head), b"var" | b"const") {
            return None;
        }
        let inner = &targets[1..];
        let mut picked: Vec<Token> = Vec::new();
        let mut depth = 0i32;
        let mut start = 0usize;
        for (index, token) in inner.iter().enumerate() {
            if token.kind != Kind::Op {
                continue;
            }
            match self.raw(token) {
                b"(" | b"[" | b"{" => depth += 1,
                b")" | b"]" | b"}" => depth -= 1,
                b"," if depth == 0 => {
                    if let Some(name) = inner[start..index].iter().find(|token| token.kind == Kind::Ident) {
                        picked.push(name.clone());
                    }
                    start = index + 1;
                }
                _ => {}
            }
        }
        if let Some(name) = inner[start..].iter().find(|token| token.kind == Kind::Ident) {
            picked.push(name.clone());
        }
        (!picked.is_empty()).then_some(picked)
    }

    fn unpack_targets(&self, toks: &[Token]) -> Option<Vec<String>> {
        let mut inner = toks;
        let class_field = matches!(self.lang.tag, "JS" | "TS" | "JSX" | "TSX");
        while let Some(word) = self.word_at(inner, 0) {
            let declaration = Self::decl_word(self.lang.tag, &word);
            // `static [entityKind] = "x"`: a class-field modifier is not the
            // binding. Stripped, the computed key is the target, which is what
            // the CST names; left in, the lexer named the field `static`.
            let modifier = class_field
                && matches!(
                    word.as_str(),
                    "static" | "public" | "private" | "protected" | "readonly" | "declare" | "abstract" | "override" | "accessor"
                );
            if !declaration && !modifier {
                break;
            }
            inner = &inner[1..];
        }
        let wrapped = inner.first().is_some_and(|t| self.is_op(t, b"(") || self.is_op(t, b"[") || self.is_op(t, b"{"));
        if wrapped {
            let (start, close) = self.group_at(inner, 0)?;
            if close + 1 == inner.len() {
                inner = &inner[start..close];
            }
            // `(a, b), c = xs`: the group is one element of a longer list, not
            // the list itself, so it is split below like any other element.
            // Returning here dropped the whole statement's targets and the
            // fall-through named only the last element.
        } else if !inner.iter().any(|token| self.is_op(token, b",")) {
            return None;
        }
        let mut targets = Vec::new();
        let mut depth = 0i32;
        let mut start = 0;
        for (index, token) in inner.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
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

    fn unpack_one(&self, part: &[Token]) -> Vec<String> {
        let mut depth = 0i32;
        let mut cut = part.len();
        for (index, token) in part.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    b"=" if depth == 0 => {
                        cut = index;
                        break;
                    }
                    _ => {}
                }
            }
        }
        let lhs = &part[..cut];
        // `{ a: b }` binds `b`; `{ a: { b, c } }` unpacks the nested pattern.
        let mut colon = None;
        let mut cdepth = 0i32;
        for (index, token) in lhs.iter().enumerate() {
            if token.kind == Kind::Op {
                match self.raw(token) {
                    b"(" | b"[" | b"{" => cdepth += 1,
                    b")" | b"]" | b"}" => cdepth -= 1,
                    b":" if cdepth == 0 => colon = Some(index),
                    _ => {}
                }
            }
        }
        let target = match colon {
            Some(at) => &lhs[at + 1..],
            None => lhs,
        };
        if target.first().is_some_and(|t| self.is_op(t, b"{") || self.is_op(t, b"[") || self.is_op(t, b"(")) {
            // Only a group that is the whole element is a nested pattern
            // (`(a, b), c = xs`). `(x or y)[k] = v` is a subscript whose
            // receiver is a group; recursing into it never shrank the slice.
            let spans = matches!(self.group_at(target, 0), Some((_, close)) if close + 1 == target.len());
            if spans {
                if let Some(nested) = self.unpack_targets(target) {
                    return nested;
                }
            }
        }
        let bound = self.target_path(target);
        // `{ a = true, b = null }`: a literal default is not a bound name, so
        // the CST keeps neither. `undefined` is an identifier and it does.
        if matches!(bound.as_deref(), Some("true" | "false" | "null")) {
            return Vec::new();
        }
        bound.into_iter().collect()
    }

    fn assignment(&mut self, targets: &[Token], value: &[Token], line: u32, augmented: bool) {
        // `let this: Handle<'a, JsValue> = ..`, `const m: Omit<A, B> = ..`:
        // the annotation is a type, not a target. Left in, its generic
        // argument was a comma at depth 0 and the binding split into three
        // names the CST never writes — and in TypeScript the type's own words
        // (`any`) became the statement's target list.
        let trimmed: Vec<Token>;
        /// The declarator list of a C declaration, when the statement is one.
        let c_split: Vec<Token>;
        // A C declaration's comma-separated declarators are separate bindings:
        // `int l, o, m=0;` initialises `m` alone, and the initialiser belongs to
        // the declarator's line rather than the type's. Read as one run, every
        // declaration with more than one declarator gave the assignment every
        // name and the type's line, so musl's own `int l, o, m=0;` and every
        // constant whose type sits on the line above lost its line.
        let mut line = line;
        let mut targets = targets;
        if matches!(self.lang.tag, "C" | "CPP" | "OBJC")
            && targets.first().is_some_and(|token| {
                token.kind == Kind::Ident && crate::expr::c_declaration_word(&self.text(token))
            })
        {
            let mut depth = 0i32;
            let mut cut = 0usize;
            for (index, token) in targets.iter().enumerate() {
                if token.kind != Kind::Op {
                    continue;
                }
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    b"," if depth == 0 => cut = index + 1,
                    _ => {}
                }
            }
            if cut > 0 {
                c_split = targets[cut..].to_vec();
                targets = &c_split;
            }
            if let Some(token) = targets.iter().rev().find(|token| token.kind == Kind::Ident) {
                line = token.line;
            }
        }
        // `var W int = 1`, `const timeout time.Duration = 5s`: Go writes the
        // type between the binder and the `=`, and the path reader named the
        // type (`int`) instead of the binder. The binder is the first name of
        // each declarator.
        let go_decl: Vec<Token>;
        if let Some(names) = self.go_declarators(targets) {
            go_decl = names;
            targets = &go_decl;
        }
        if matches!(self.lang.tag, "RS" | "TS" | "TSX") {
            let mut depth = 0i32;
            for (index, token) in targets.iter().enumerate() {
                if token.kind != Kind::Op {
                    continue;
                }
                match self.raw(token) {
                    b"(" | b"[" | b"{" => depth += 1,
                    b")" | b"]" | b"}" => depth -= 1,
                    b":" if depth == 0 => {
                        trimmed = targets[..index].to_vec();
                        targets = &trimmed;
                        break;
                    }
                    _ => {}
                }
            }
        } else if matches!(self.lang.tag, "TS" | "TSX") {
            // `const Proto: Omit<ValueMatcher<any, any>, "value"> = {`: the
            // annotation is a type, not a target, and the commas inside its
            // generic argument list sit at depth 0 for a reader that counts
            // only brackets. One binding was read as four names the CST never
            // writes. A colon inside a destructuring pattern is at depth 1 and
            // is left for `unpack_one` to read as a rename.
            let mut depth = 0i32;
            let mut colon = None;
            for (index, token) in targets.iter().enumerate() {
                if token.kind == Kind::Op {
                    match self.raw(token) {
                        b"(" | b"[" | b"{" => depth += 1,
                        b")" | b"]" | b"}" => depth -= 1,
                        b":" if depth == 0 => {
                            colon = Some(index);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            if let Some(at) = colon {
                trimmed = targets[..at].to_vec();
                targets = &trimmed;
            }
        }
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
