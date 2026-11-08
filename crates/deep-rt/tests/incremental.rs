//! The differential test for the incremental parser: incremental output must be
//! byte-identical to a full parse, and it must be faster.
//!
//! The grammar here is a *structural* Python grammar — statements, blocks,
//! indentation and a permissive expression soup — because the point is not to
//! parse Python perfectly but to have a real CST over real Python files with
//! hundreds of nested nodes, so a spliced subtree has something to be right or
//! wrong about. The fixtures are the repository's own
//! `crates/deep-grammars/fixtures/python/*.py`, read at run time.
//!
//! The module under test lives in `src/incremental.rs`; it is pulled in with
//! `#[path]` so the test runs before `pub mod incremental;` lands in `lib.rs`.
//! Once it does, replace the three shim modules and the `#[path]` line with
//! `use deep_rt::incremental::*;`.

// The shims make `crate::cst`, `crate::driver` and `crate::table` resolve the
// same way inside the included module as they would inside the crate.
mod cst {
    pub use deep_rt::cst::*;
}
mod driver {
    pub use deep_rt::driver::*;
}
mod table {
    pub use deep_rt::table::*;
}

#[path = "../src/incremental.rs"]
mod incremental;

use std::time::{Duration, Instant};

use deep_rt::{
    parse, reduce_cell, shift_cell, Config, Cst, Grammar, Lexer, Node, NodeId, StateId, SymbolId,
    Token, EOF_SYMBOL, NONE,
};
use incremental::{Edit, Report};

// ---------------------------------------------------------------------------
// The language: terminals, nonterminals, productions
// ---------------------------------------------------------------------------

const NAME: SymbolId = 1;
const NUM: SymbolId = 2;
const STR: SymbolId = 3;
const OP: SymbolId = 4;
const NL: SymbolId = 5;
const INDENT: SymbolId = 6;
const DEDENT: SymbolId = 7;
const LP: SymbolId = 8;
const RP: SymbolId = 9;
const LB: SymbolId = 10;
const RB: SymbolId = 11;
const LC: SymbolId = 12;
const RC: SymbolId = 13;
/// The `:` that ends a compound header, emitted by the lexer only at bracket
/// depth zero on a line that starts with `def`/`class`/a block keyword.
const COLON: SymbolId = 14;
const DEF: SymbolId = 15;
const CLASS: SymbolId = 16;
const HEAD: SymbolId = 17;
const AT: SymbolId = 18;
/// Any other `:` — dict entries, slices, annotations, lambdas.
const COLON2: SymbolId = 19;
/// A block keyword that is not the first token of its line (`x if y else z`).
const HEAD_EXPR: SymbolId = 20;
/// `@` in expression position (`a @ b`), as opposed to a decorator.
const AT_EXPR: SymbolId = 21;

const FILE: SymbolId = 24;
const STMTS: SymbolId = 25;
const STMT: SymbolId = 26;
const LINE: SymbolId = 27;
const ITEMS: SymbolId = 28;
const ITEM: SymbolId = 29;
const SUITE: SymbolId = 30;
const SYMBOLS: u32 = 31;

/// `(lhs, rhs)`, in production-id order. The first entry is the augmented
/// start; its state accepts instead of reducing, so it needs no kind of its own.
///
/// The line-start keywords have their own terminals (the scanner knows whether
/// a keyword is the first token of its logical line), which is what keeps
/// `def f():` from also being readable as the item list `def f ( ) :` — one
/// ambiguity there and the table goes non-LR.
const PRODUCTIONS: &[(SymbolId, &[SymbolId])] = &[
    (0, &[FILE]),                                     // 0  AUG -> FILE
    (FILE, &[STMTS]),                                 // 1
    (STMTS, &[STMTS, STMT]),                          // 2
    (STMTS, &[]),                                     // 3
    (STMT, &[LINE, NL]),                              // 4
    (STMT, &[NL]),                                    // 5
    (STMT, &[DEF, NAME, ITEMS, COLON, SUITE]),        // 6
    (STMT, &[DEF, NAME, ITEMS, COLON, LINE, NL]),     // 7
    (STMT, &[CLASS, NAME, COLON, SUITE]),             // 8
    (STMT, &[CLASS, NAME, COLON, LINE, NL]),          // 9
    (STMT, &[CLASS, NAME, ITEMS, COLON, SUITE]),      // 10
    (STMT, &[CLASS, NAME, ITEMS, COLON, LINE, NL]),   // 11
    (STMT, &[HEAD, ITEMS, COLON, SUITE]),             // 12
    (STMT, &[HEAD, ITEMS, COLON, LINE, NL]),          // 13
    (STMT, &[HEAD, COLON, SUITE]),                    // 14  `else:` / `try:`
    (STMT, &[HEAD, COLON, LINE, NL]),                 // 15
    (STMT, &[AT, ITEMS, NL]),                         // 16  a decorator
    (LINE, &[ITEMS]),                                 // 17
    (ITEMS, &[ITEMS, ITEM]),                          // 18
    (ITEMS, &[ITEM]),                                 // 19
    (ITEM, &[NAME]),                                  // 20
    (ITEM, &[NUM]),                                   // 21
    (ITEM, &[STR]),                                   // 22
    (ITEM, &[OP]),                                    // 23
    (ITEM, &[COLON2]),                                // 24
    (ITEM, &[LP, ITEMS, RP]),                         // 25
    (ITEM, &[LB, ITEMS, RB]),                         // 26
    (ITEM, &[LC, ITEMS, RC]),                         // 27
    (ITEM, &[HEAD_EXPR]),                             // 28  `x if y else z`
    (ITEM, &[AT_EXPR]),                               // 29  matrix `a @ b`
    (SUITE, &[NL, INDENT, STMTS, DEDENT]),            // 30
    (ITEM, &[LP, RP]),                                // 31  `f()`, `def f():`
    (ITEM, &[LB, RB]),                                // 32
    (ITEM, &[LC, RC]),                                // 33
];

struct PyTables {
    states: u32,
    action_index: Vec<i32>,
    action_table: Vec<u32>,
    goto_index: Vec<i32>,
    goto_table: Vec<u32>,
    default_reduce: Vec<SymbolId>,
    production_len: Vec<u8>,
}

impl PyTables {
    fn grammar(&self) -> Grammar<'_> {
        Grammar {
            name: "py-structural",
            states: self.states,
            symbols: SYMBOLS,
            action_index: &self.action_index,
            action_table: &self.action_table,
            goto_index: &self.goto_index,
            goto_table: &self.goto_table,
            default_reduce: &self.default_reduce,
            production_len: &self.production_len,
        }
    }
}

#[inline]
fn is_terminal(symbol: SymbolId) -> bool {
    symbol >= 1 && symbol < FILE
}

/// FIRST of a symbol, as a bit set over terminals. Lookahead bit 0 is EOF;
/// terminal `t` is bit `t`.
fn first_of(symbol: SymbolId, first: &[u32], nullable: &[bool]) -> (u32, bool) {
    if is_terminal(symbol) {
        (1u32 << symbol, false)
    } else {
        let at = symbol as usize;
        (first[at], nullable[at])
    }
}

/// FIRST of a symbol string, followed by the lookahead set `la`.
fn first_seq(seq: &[SymbolId], la: u32, first: &[u32], nullable: &[bool]) -> u32 {
    let mut set = 0u32;
    for &symbol in seq {
        let (f, n) = first_of(symbol, first, nullable);
        set |= f;
        if !n {
            return set;
        }
    }
    set | la
}

/// One canonical LR(1) item: production, dot position, lookahead set (one bit).
type Item1 = (u16, u16, u32);

fn closure1(seed: Vec<Item1>, first: &[u32], nullable: &[bool]) -> Vec<Item1> {
    let mut items = seed;
    items.sort_unstable();
    items.dedup();
    let mut at = 0usize;
    while at < items.len() {
        let (production, dot, lookahead) = items[at];
        at += 1;
        let rhs = PRODUCTIONS[production as usize].1;
        if dot as usize >= rhs.len() {
            continue;
        }
        let symbol = rhs[dot as usize];
        if is_terminal(symbol) {
            continue;
        }
        let expect = first_seq(&rhs[dot as usize + 1..], lookahead, first, nullable);
        let mut bit = 0u32;
        while bit < 32 {
            if expect & (1u32 << bit) != 0 {
                for (id, &(lhs, _)) in PRODUCTIONS.iter().enumerate() {
                    if lhs != symbol {
                        continue;
                    }
                    let item = (id as u16, 0u16, 1u32 << bit);
                    if !items.contains(&item) {
                        items.push(item);
                    }
                }
            }
            bit += 1;
        }
    }
    items.sort_unstable();
    items.dedup();
    items
}

/// Build canonical LR(1) tables from [`PRODUCTIONS`], panicking with the item
/// set on a conflict. The ABI has one action cell per (state, terminal), so a
/// conflict here would have to be resolved by luck, and the differential test
/// would then be comparing two guesses.
fn build_tables() -> PyTables {
    // FIRST and nullability, fixed point over the productions.
    let mut first = vec![0u32; SYMBOLS as usize];
    let mut nullable = vec![false; SYMBOLS as usize];
    for _ in 0..PRODUCTIONS.len() * 2 {
        for &(lhs, rhs) in PRODUCTIONS {
            if lhs == 0 {
                continue;
            }
            let mut set = 0u32;
            let mut all_nullable = true;
            for &symbol in rhs {
                let (f, n) = first_of(symbol, &first, &nullable);
                set |= f;
                if !n {
                    all_nullable = false;
                    break;
                }
            }
            first[lhs as usize] |= set;
            if all_nullable {
                nullable[lhs as usize] = true;
            }
        }
    }

    // Canonical LR(1) item sets. The augmented start has EOF as its lookahead.
    let mut states: Vec<Vec<Item1>> = Vec::new();
    let mut transitions: Vec<(usize, SymbolId, usize)> = Vec::new();
    states.push(closure1(vec![(0, 0, 1u32)], &first, &nullable));
    let mut at = 0usize;
    while at < states.len() {
        let mut symbols: Vec<SymbolId> = Vec::new();
        for &(production, dot, _) in &states[at] {
            let rhs = PRODUCTIONS[production as usize].1;
            if (dot as usize) < rhs.len() {
                let symbol = rhs[dot as usize];
                if !symbols.contains(&symbol) {
                    symbols.push(symbol);
                }
            }
        }
        symbols.sort_unstable();
        for symbol in symbols {
            let mut seed: Vec<Item1> = Vec::new();
            for &(production, dot, lookahead) in &states[at] {
                let rhs = PRODUCTIONS[production as usize].1;
                if (dot as usize) < rhs.len() && rhs[dot as usize] == symbol {
                    seed.push((production, dot + 1, lookahead));
                }
            }
            let next = closure1(seed, &first, &nullable);
            let target = match states.iter().position(|state| *state == next) {
                Some(found) => found,
                None => {
                    states.push(next);
                    states.len() - 1
                }
            };
            transitions.push((at, symbol, target));
        }
        at += 1;
    }

    let action_width = SYMBOLS as usize + 1;
    let mut action_table = vec![0u32; states.len() * action_width];
    let mut goto_table = vec![0u32; states.len() * SYMBOLS as usize];
    let mut default_reduce = vec![0 as SymbolId; states.len()];

    for &(from, symbol, to) in &transitions {
        if is_terminal(symbol) {
            let cell = &mut action_table[from * action_width + symbol as usize];
            let wanted = shift_cell(to as StateId);
            panic_on_conflict(cell, wanted, &states, from, symbol);
            *cell = wanted;
        } else {
            goto_table[from * SYMBOLS as usize + symbol as usize] = to as u32 + 1;
        }
    }

    for (id, items) in states.iter().enumerate() {
        for &(production, dot, lookahead) in items {
            let (lhs, rhs) = PRODUCTIONS[production as usize];
            if (dot as usize) < rhs.len() {
                continue;
            }
            if production == 0 {
                let cell = &mut action_table[id * action_width + SYMBOLS as usize];
                panic_on_conflict(cell, table::ACTION_ACCEPT, &states, id, EOF_SYMBOL);
                *cell = table::ACTION_ACCEPT;
                continue;
            }
            if default_reduce[id] == 0 {
                default_reduce[id] = lhs;
            } else {
                assert_eq!(
                    default_reduce[id], lhs,
                    "state {id} reduces two different symbols"
                );
            }
            let mut bit = 0u32;
            while bit < 32 {
                if lookahead & (1u32 << bit) != 0 {
                    let column = if bit == 0 {
                        SYMBOLS as usize
                    } else {
                        bit as usize
                    };
                    let cell = &mut action_table[id * action_width + column];
                    let wanted = reduce_cell(production as u32);
                    panic_on_conflict(cell, wanted, &states, id, column as SymbolId);
                    *cell = wanted;
                }
                bit += 1;
            }
        }
    }

    let mut action_index = vec![0i32; states.len() * 2];
    let mut goto_index = vec![0i32; states.len() * 2];
    for state in 0..states.len() {
        action_index[state * 2] = (state * action_width) as i32;
        goto_index[state * 2] = (state * SYMBOLS as usize) as i32;
    }

    PyTables {
        states: states.len() as u32,
        action_index,
        action_table,
        goto_index,
        goto_table,
        default_reduce,
        production_len: PRODUCTIONS.iter().map(|(_, rhs)| rhs.len() as u8).collect(),
    }
}

fn panic_on_conflict(
    cell: &u32,
    wanted: u32,
    states: &[Vec<Item1>],
    state: usize,
    symbol: SymbolId,
) {
    if *cell != 0 && *cell != wanted {
        panic!(
            "parser conflict in state {state} on symbol {symbol}: have {:#x}, want {:#x}\n{:?}",
            *cell, wanted, states[state]
        );
    }
}

// ---------------------------------------------------------------------------
// The scanner: Python layout, strings, brackets
// ---------------------------------------------------------------------------

fn is_head_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "elif"
            | "else"
            | "while"
            | "for"
            | "with"
            | "try"
            | "except"
            | "finally"
            | "match"
            | "case"
    )
}

fn is_string_prefix(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "r" | "b" | "u" | "f" | "rb" | "br" | "fr" | "rf"
    )
}

fn scan_string(bytes: &[u8], at: usize) -> usize {
    let quote = bytes[at];
    let triple = at + 2 < bytes.len() && bytes[at + 1] == quote && bytes[at + 2] == quote;
    let mut i = if triple { at + 3 } else { at + 1 };
    if triple {
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == quote
                && i + 2 < bytes.len()
                && bytes[i + 1] == quote
                && bytes[i + 2] == quote
            {
                return i + 3;
            }
            i += 1;
        }
        return bytes.len();
    }
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            byte if byte == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// `(symbol, length)` for a byte that is a one-symbol token, if it is one.
fn punctuation(byte: u8) -> Option<SymbolId> {
    Some(match byte {
        b'(' => LP,
        b')' => RP,
        b'[' => LB,
        b']' => RB,
        b'{' => LC,
        b'}' => RC,
        b',' | b';' | b'=' | b'.' | b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'|'
        | b'&' | b'^' | b'~' | b'!' | b'?' => OP,
        _ => return None,
    })
}

fn lex_py(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut tokens: Vec<Token> = Vec::with_capacity(len / 4 + 8);
    let mut indents: Vec<u32> = vec![0];
    let mut depth: i32 = 0;
    let mut at_line_start = true;
    let mut line_first: Option<SymbolId> = None;
    let mut i = 0usize;

    macro_rules! push {
        ($symbol:expr, $start:expr, $end:expr) => {{
            let symbol = $symbol;
            if line_first.is_none() && symbol != NL && symbol != INDENT && symbol != DEDENT {
                line_first = Some(symbol);
            }
            tokens.push(Token {
                symbol,
                start: $start as u32,
                end: $end as u32,
            });
        }};
    }

    while i < len {
        if at_line_start {
            let mut j = i;
            while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j >= len {
                i = j;
                break;
            }
            if bytes[j] == b'\n' || bytes[j] == b'\r' || bytes[j] == b'#' {
                // A blank or comment-only line: one NL, no layout change.
                if bytes[j] == b'#' {
                    while j < len && bytes[j] != b'\n' {
                        j += 1;
                    }
                }
                if j < len && bytes[j] == b'\r' {
                    j += 1;
                }
                if j < len && bytes[j] == b'\n' {
                    push!(NL, j, j + 1);
                    j += 1;
                }
                i = j;
                continue;
            }
            let width = (j - i) as u32;
            let current = *indents.last().unwrap();
            if width > current {
                indents.push(width);
                push!(INDENT, i, j);
            } else if width < current {
                while *indents.last().unwrap() > width {
                    indents.pop();
                    push!(DEDENT, i, j);
                }
                if *indents.last().unwrap() != width {
                    indents.push(width);
                }
            }
            i = j;
            at_line_start = false;
            line_first = None;
            continue;
        }

        let byte = bytes[i];
        match byte {
            b' ' | b'\t' | b'\r' => {
                i += 1;
            }
            b'\n' => {
                // Inside brackets a newline is not a token: Python continues
                // the logical line, and so does the grammar.
                if depth <= 0 {
                    depth = 0;
                    push!(NL, i, i + 1);
                    at_line_start = true;
                    line_first = None;
                }
                i += 1;
            }
            b'\\' if i + 1 < len && (bytes[i + 1] == b'\n' || bytes[i + 1] == b'\r') => {
                i += 1;
                while i < len && (bytes[i] == b'\r' || bytes[i] == b'\n') {
                    i += 1;
                    break;
                }
            }
            b'#' => {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'\'' | b'"' => {
                let end = scan_string(bytes, i);
                push!(STR, i, end);
                i = end;
            }
            b'0'..=b'9' => {
                let start = i;
                if bytes[i] == b'0' && i + 1 < len && matches!(bytes[i + 1], b'x' | b'X' | b'o' | b'O' | b'b' | b'B')
                {
                    i += 2;
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                } else {
                    while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                        i += 1;
                    }
                    if i + 1 < len && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                        i += 1;
                        while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                            i += 1;
                        }
                    }
                    if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
                        let mut j = i + 1;
                        if j < len && (bytes[j] == b'+' || bytes[j] == b'-') {
                            j += 1;
                        }
                        if j < len && bytes[j].is_ascii_digit() {
                            i = j;
                            while i < len && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                        }
                    }
                }
                push!(NUM, start, i);
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &source[start..i];
                if is_string_prefix(word) && i < len && (bytes[i] == b'\'' || bytes[i] == b'"') {
                    let end = scan_string(bytes, i);
                    push!(STR, start, end);
                    i = end;
                    continue;
                }
                if word == "async" {
                    // `async def f():` is one `def`: the token spans both words
                    // so every byte of the source stays inside the tree.
                    let mut j = i;
                    while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
                        j += 1;
                    }
                    let mut k = j;
                    while k < len && bytes[k].is_ascii_alphabetic() {
                        k += 1;
                    }
                    match &source[j..k] {
                        "def" => {
                            i = k;
                            push!(DEF, start, k);
                            continue;
                        }
                        "for" | "with" => {
                            i = k;
                            push!(HEAD, start, k);
                            continue;
                        }
                        _ => {}
                    }
                }
                let first = line_first.is_none();
                let symbol = match word {
                    // Line-start keywords get the terminal that can only start a
                    // block; the same word mid-line is just an item.
                    "def" if first => DEF,
                    "class" if first => CLASS,
                    other if is_head_keyword(other) => {
                        if first {
                            HEAD
                        } else {
                            HEAD_EXPR
                        }
                    }
                    _ => NAME,
                };
                push!(symbol, start, i);
            }
            b'@' => {
                let symbol = if line_first.is_none() { AT } else { AT_EXPR };
                push!(symbol, i, i + 1);
                i += 1;
            }
            b':' => {
                let symbol = if depth == 0
                    && matches!(line_first, Some(DEF) | Some(CLASS) | Some(HEAD))
                {
                    COLON
                } else {
                    COLON2
                };
                push!(symbol, i, i + 1);
                i += 1;
            }
            other => match punctuation(other) {
                Some(symbol) => {
                    match symbol {
                        LP | LB | LC => depth += 1,
                        RP | RB | RC => depth -= 1,
                        _ => {}
                    }
                    push!(symbol, i, i + 1);
                    i += 1;
                }
                None => {
                    push!(OP, i, i + 1);
                    i += 1;
                }
            },
        }
    }

    if let Some(last) = tokens.last() {
        if last.symbol != NL {
            push!(NL, len, len);
        }
    }
    while indents.len() > 1 {
        indents.pop();
        push!(DEDENT, len, len);
    }
    tokens
}

// ---------------------------------------------------------------------------
// Parse helpers
// ---------------------------------------------------------------------------

struct SliceLexer<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Lexer for SliceLexer<'_> {
    fn next_token(&mut self, _state: StateId) -> Option<Token> {
        let token = self.tokens.get(self.at).copied();
        if token.is_some() {
            self.at += 1;
        }
        token
    }
}

fn try_full_parse(grammar: &Grammar<'_>, tokens: &[Token]) -> Result<Cst, deep_rt::Fatal> {
    let mut lexer = SliceLexer { tokens, at: 0 };
    parse(grammar, &mut lexer, &Config::new()).map(|(cst, _)| cst)
}

fn full_parse(grammar: &Grammar<'_>, tokens: &[Token]) -> Cst {
    try_full_parse(grammar, tokens).expect("a tree")
}

/// A canonical serialisation: kind and byte range in document order, with the
/// nesting made explicit. Two trees are the same tree iff their shapes are equal.
fn shape(nodes: &[Node], root: NodeId) -> String {
    let mut out = String::with_capacity(nodes.len() * 12);
    let mut stack: Vec<(NodeId, bool)> = vec![(root, false)];
    let mut kids: Vec<NodeId> = Vec::new();
    while let Some((id, close)) = stack.pop() {
        let node = nodes[id as usize];
        if close {
            out.push(')');
            continue;
        }
        out.push('(');
        out.push_str(&node.kind.to_string());
        out.push('@');
        out.push_str(&node.start.to_string());
        out.push_str("..");
        out.push_str(&node.end.to_string());
        stack.push((id, true));
        kids.clear();
        let mut child = node.first_child;
        while child != NONE {
            kids.push(child);
            child = nodes[child as usize].next_sibling;
        }
        for &child in kids.iter().rev() {
            stack.push((child, false));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Fixtures and the edits applied to them
// ---------------------------------------------------------------------------

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("deep-grammars")
        .join("fixtures")
        .join("python")
}

fn fixtures() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = fixture_dir();
    let mut names: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().map(|ext| ext == "py").unwrap_or(false))
        .collect();
    names.sort();
    for path in names {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).expect("read fixture");
        out.push((name, text));
    }
    assert!(out.len() >= 10, "the python fixtures are the corpus");
    out
}

/// `(start, end)` of every line, end exclusive of the newline only when the
/// line is blank; `end` always points at the line's terminating newline or EOF.
fn line_spans(source: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut at = 0;
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            spans.push((at, index + 1));
            at = index + 1;
        }
    }
    if at < source.len() {
        spans.push((at, source.len()));
    }
    spans
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (at, byte) in source.bytes().enumerate() {
        if byte == b'\n' && at + 1 < source.len() {
            starts.push(at + 1);
        }
    }
    starts
}

/// A deterministic set of edits: at least thirty by the time it is run over the
/// ten fixtures. Every replacement text matches its `Edit::new_len`.
fn edits_for(source: &str) -> Vec<(String, Edit, String)> {
    let bytes = source.as_bytes();
    let starts = line_starts(source);
    let mut edits: Vec<(String, Edit, String)> = Vec::new();

    // Insert a statement at the start of a line, deep enough to be inside a
    // block: a one-line change in the middle of the file.
    for (n, &at) in starts.iter().enumerate().skip(8).step_by(11) {
        // Same indentation as the line it is inserted before: a new statement
        // in the same block, not an accidental indent.
        let mut width = 0usize;
        while at + width < bytes.len() && bytes[at + width] == b' ' {
            width += 1;
        }
        let inserted = format!("{}x = 1\n", " ".repeat(width));
        edits.push((
            format!("insert-statement-at-line-{n}"),
            Edit::insert(at as u32, inserted.len() as u32),
            inserted,
        ));
    }

    // Delete a whole line.
    for (n, window) in starts.windows(2).enumerate().skip(5).step_by(13) {
        let (at, next) = (window[0], window[1]);
        edits.push((
            format!("delete-line-{n}"),
            Edit::delete(at as u32, (next - at) as u32),
            String::new(),
        ));
    }

    // Replace one word with a longer and a shorter one.
    for (label, needle, replacement) in [
        ("widen-a-name", "self", "this_object"),
        ("narrow-a-name", "return", "ret"),
        ("number", "1", "1234"),
    ] {
        if let Some(at) = source.find(needle) {
            edits.push((
                label.to_string(),
                Edit::replace(at as u32, needle.len() as u32, replacement.len() as u32),
                replacement.to_string(),
            ));
        }
    }

    // Insert exactly at a node boundary: right before a `def` at column zero.
    if let Some(at) = source.find("\ndef ") {
        let at = at + 1;
        let text = "BOUNDARY = 1\n";
        edits.push((
            "insert-at-def-boundary".to_string(),
            Edit::insert(at as u32, text.len() as u32),
            text.to_string(),
        ));
    }

    // Indentation change that keeps the file valid: dedent a method out of its
    // class. This is the case where the moved subtree cannot be reused.
    if let Some(at) = source.find("\n    def ") {
        let at = at + 1;
        edits.push((
            "dedent-a-method-out-of-its-class".to_string(),
            Edit::delete(at as u32, 4),
            String::new(),
        ));
    }

    // Indentation change that is purely local: indent a whole line further.
    for (n, window) in starts.windows(2).enumerate().skip(9).step_by(17) {
        let (at, next) = (window[0], window[1]);
        if !bytes[at..].starts_with(b"    ") || next == at {
            continue;
        }
        // Only where the line above ends in a colon: indenting a line further
        // is valid python exactly when it becomes a block.
        let above = &source[..at];
        let above = above.trim_end();
        if !above.ends_with(':') {
            continue;
        }
        edits.push((
            format!("indent-line-{n}-into-a-block"),
            Edit::insert(at as u32, 4),
            "    ".to_string(),
        ));
    }

    // Blank-line edits: boundary between two statements, no tokens change.
    if let Some(at) = source.find("\n\n") {
        edits.push((
            "join-two-blank-separated-statements".to_string(),
            Edit::delete((at + 1) as u32, 1),
            String::new(),
        ));
    }

    // Replace a whole top-level function: the largest local change.
    if let Some(at) = source.find("\ndef ") {
        let from = at + 1;
        let rest = &source[from..];
        if let Some(offset) = rest[1..].find("\ndef ").or_else(|| rest[1..].find("\nclass ")) {
            let to = from + 1 + offset + 1;
            let body = "# replaced\n";
            edits.push((
                "replace-a-whole-function".to_string(),
                Edit::replace(from as u32, (to - from) as u32, body.len() as u32),
                body.to_string(),
            ));
        }
    }

    // Append a whole function at the end of the file.
    let text = "\ndef appended(a, b):\n    return a + b\n".to_string();
    edits.push((
        "append-a-whole-function".to_string(),
        Edit::insert(source.len() as u32, text.len() as u32),
        text,
    ));

    edits
}

// ---------------------------------------------------------------------------
// The differential test
// ---------------------------------------------------------------------------


#[test]
fn incremental_matches_full_parse_on_python_fixtures() {
    let tables = build_tables();
    let grammar = tables.grammar();
    let mut cases = 0usize;
    let mut splices = 0usize;
    let mut unparseable = 0usize;

    for (name, source) in fixtures() {
        let tokens = lex_py(&source);
        let old = full_parse(&grammar, &tokens);
        assert_eq!(
            old.errors, 0,
            "fixture {name} must parse cleanly for reuse to be meaningful"
        );

        let mut covered = vec![false; source.len()];
        let mut stack = vec![old.root];
        while let Some(id) = stack.pop() {
            let node = old.nodes[id as usize];
            if node.flags & deep_rt::FLAG_ERROR != 0 {
                continue;
            }
            for at in node.start as usize..node.end as usize {
                covered[at] = true;
            }
            let mut child = node.first_child;
            while child != NONE {
                stack.push(child);
                child = old.nodes[child as usize].next_sibling;
            }
        }
        if !source.trim().is_empty() {
            assert!(
                covered.iter().all(|&x| x),
                "fixture {name}: the tree covers every byte"
            );
        }

        for (label, edit, text) in edits_for(&source) {
            assert_eq!(text.len() as u32, edit.new_len);
            let new_source = edit.apply(source.as_bytes(), text.as_bytes());
            let new_source = String::from_utf8(new_source).expect("utf8 eof");
            let new_tokens = lex_py(&new_source);
            let incremental_result = incremental::reparse(
                &grammar,
                &old,
                source.len() as u32,
                &edit,
                &new_tokens,
                &Config::new(),
            );
            let full = match try_full_parse(&grammar, &new_tokens) {
                Ok(full) => full,
                Err(fatal) => {
                    // An edit can make the source unparseable at all (an
                    // unterminated string, say). Then the incremental pass has
                    // nothing to be identical to; it must fail the same way.
                    assert_eq!(
                        incremental_result.err(),
                        Some(fatal),
                        "fixture {name}, edit {label}: both parses must agree on failure"
                    );
                    unparseable += 1;
                    continue;
                }
            };
            let (incremental_tree, report) = incremental_result.expect("a tree");

            let want = shape(&full.nodes, full.root);
            let got = shape(&incremental_tree.nodes, incremental_tree.root);
            assert_eq!(
                want, got,
                "fixture {name}, edit {label}: incremental tree differs from a full parse"
            );
            assert_eq!(
                full.errors, incremental_tree.errors,
                "fixture {name}, edit {label}: error count differs"
            );
            assert_eq!(
                incremental_tree.nodes.len() as u32,
                report.total_nodes,
                "fixture {name}, edit {label}: node count bookkeeping"
            );
            if full.errors == 0 {
                assert!(
                    !report.fell_back,
                    "fixture {name}, edit {label}: a clean source must stay incremental"
                );
                splices += report.splices as usize;
                cases += 1;
            }
        }
    }

    assert!(
        cases >= 30,
        "only {cases} edits exercised the incremental path"
    );
    // A splice is refused whenever the edit moved a layout token inside the
    // candidate's bytes, which is exactly the case indentation edits create.
    assert!(splices >= 20, "only {splices} splices over all edits");
    eprintln!(
        "{cases} edits compared, {splices} splices, {unparseable} edits made the source unparseable"
    );
}

#[test]
fn incremental_keeps_most_of_a_clean_edit() {
    let tables = build_tables();
    let grammar = tables.grammar();
    let source = std::fs::read_to_string(fixture_dir().join("models.py")).unwrap();
    let tokens = lex_py(&source);
    let old = full_parse(&grammar, &tokens);

    // One line changed in the middle: the smallest covering node is small, and
    // everything after it is spliced back.
    let at = source.find("self.amount + other.amount").unwrap();
    let text = "self.amount + other.amount * 2";
    let edit = Edit::replace(
        at as u32,
        "self.amount + other.amount".len() as u32,
        text.len() as u32,
    );
    let new_source = edit.apply(source.as_bytes(), text.as_bytes());
    let new_source = String::from_utf8(new_source).unwrap();
    let new_tokens = lex_py(&new_source);
    let full = full_parse(&grammar, &new_tokens);
    let (tree, report) = incremental::reparse(
        &grammar,
        &old,
        source.len() as u32,
        &edit,
        &new_tokens,
        &Config::new(),
    )
    .expect("a tree");

    assert!(!report.fell_back, "a clean edit must not fall back");
    assert_eq!(
        shape(&full.nodes, full.root),
        shape(&tree.nodes, tree.root),
        "same tree"
    );
    assert!(
        report.reuse_ratio() > 0.90,
        "a one-line edit should keep nearly the whole tree, kept {:.3}",
        report.reuse_ratio()
    );
    assert!(report.splices >= 1, "the tail must be spliced");
}

// ---------------------------------------------------------------------------
// Structural edits: what cannot be reused, and how much still is
// ---------------------------------------------------------------------------

/// Start offset of every statement at column zero.
fn top_level_starts(source: &str) -> Vec<usize> {
    line_spans(source)
        .into_iter()
        .filter(|&(at, _)| {
            let line = &source[at..];
            line.starts_with("def ") || line.starts_with("class ") || line.starts_with("@")
        })
        .map(|(at, _)| at)
        .collect()
}

#[test]
fn incremental_reuses_the_rest_of_a_structural_edit() {
    let tables = build_tables();
    let grammar = tables.grammar();
    let source = std::fs::read_to_string(fixture_dir().join("models.py")).unwrap();
    let tokens = lex_py(&source);
    let old = full_parse(&grammar, &tokens);

    // Swapping two adjacent top-level blocks moves both: a node's identity
    // includes where it sits, so neither can be reused. Everything before and
    // after the pair still lines up and is spliced.
    let tops = top_level_starts(&source);
    assert!(tops.len() >= 3, "models.py has several top-level blocks");
    let first = tops[0];
    let second = tops[1];
    let third = tops.get(2).copied().unwrap_or(source.len());
    let a = &source[first..second];
    let b = &source[second..third];
    let swapped = format!("{b}{a}");
    let edit = Edit::replace(first as u32, (third - first) as u32, swapped.len() as u32);
    let new_source = edit.apply(source.as_bytes(), swapped.as_bytes());
    let new_source = String::from_utf8(new_source).unwrap();
    let new_tokens = lex_py(&new_source);
    let full = full_parse(&grammar, &new_tokens);
    let (tree, report) = incremental::reparse(
        &grammar,
        &old,
        source.len() as u32,
        &edit,
        &new_tokens,
        &Config::new(),
    )
    .expect("a tree");

    assert_eq!(
        shape(&full.nodes, full.root),
        shape(&tree.nodes, tree.root),
        "swapping two blocks: same tree"
    );
    assert_eq!(full.errors, 0, "the swapped file is still valid python");
    assert!(!report.fell_back, "a moved block is still a spliceable tree");
    assert!(
        report.splices >= 1,
        "the file after the moved blocks is still spliced"
    );
    assert!(
        report.reuse_ratio() < 0.95,
        "the two moved blocks cannot be reused: {:.3}",
        report.reuse_ratio()
    );
    assert!(
        report.rebuilt_nodes > 5,
        "the moved blocks were rebuilt, rebuilt {}",
        report.rebuilt_nodes
    );
    eprintln!(
        "structural edit (two blocks swapped): {} nodes, reused {} ({:.1}%), rebuilt {}, splices {}",
        report.total_nodes,
        report.reused_nodes,
        report.reuse_ratio() * 100.0,
        report.rebuilt_nodes,
        report.splices
    );
}

