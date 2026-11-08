//! Reads the `grammar!` input into a [`Grammar`]. Names are not resolved here;
//! [`check`](crate::check) does that once every token and rule is known.

use crate::{Delim, Error, Spot, Tok, Tree};

pub struct Grammar {
    /// The `static` that holds the language, such as `JSON`.
    pub name: String,
    /// Which code path compiles this grammar.
    pub mode: Mode,
    /// The language's own name, such as `"json"`. Defaults to `name` in lower case.
    pub language: String,
    /// The lexer function, as a path.
    pub lex: String,
    /// The module holding the runtime. Defaults to `crate::deep`.
    pub runtime: String,
    pub tokens: Vec<TokenDef>,
    pub rules: Vec<Rule>,
    /// `layout = NEWLINE INDENT DEDENT;`: indentation as tokens.
    pub layout: Option<LayoutDef>,
}

/// How the macro compiles a grammar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Tables when the generator can express the grammar, the recursive path
    /// when it cannot: the default, so grammars that still need predicates or a
    /// precedence table keep building.
    Auto,
    /// Tables, or a build failure: a grammar the generator refuses is an error,
    /// never a silent fallback.
    Tables,
    /// The recursive-descent parser functions, unchanged.
    Recursive,
}

pub struct LayoutDef {
    pub newline: String,
    pub indent: String,
    pub dedent: String,
    pub spot: Spot,
}

pub struct TokenDef {
    pub constant: String,
    /// The kind's name in trees, and how rules may refer to it: `"{"`, `"string"`.
    pub text: String,
    pub class: TokenClass,
    /// `reset`: starting a line inside brackets, this token ends them (layout only).
    pub reset: bool,
    pub spot: Spot,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TokenClass {
    Token,
    Bad,
    Trivia,
    Open(String),
    Close(String),
}

pub struct Rule {
    pub name: String,
    pub body: Node,
    pub spot: Spot,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub expr: Expr,
    pub spot: Spot,
}

#[derive(Clone, Debug)]
pub enum Expr {
    /// `"{"`: a token by its text. Resolved to [`Expr::Token`].
    Literal(String),
    /// `pair` or `STRING`: a rule, or a token by its constant. Resolved by `check`.
    Name(String),
    Token(usize),
    Rule(usize),
    Seq(Vec<Node>),
    Choice(Vec<Node>),
    /// `[x]` or `x?`
    Opt(Box<Node>),
    Star(Box<Node>),
    Plus(Box<Node>),
    /// `x % sep`: one or more `x`, separated by `sep`, or none. `x %? sep`
    /// (the flag) also allows one `sep` after the last `x`.
    Sep(Box<Node>, Box<Node>, bool),
    /// `&x`: a syntactic predicate. It starts an alternative, which is taken only
    /// if `x` parses here without an error; it consumes nothing.
    Pred(Box<Node>),
    /// `rule@label`: a precedence rule entered at the level of its row `label`.
    Level(String, String),
    /// Resolved [`Expr::Level`]: the rule, and the level (row index + 1).
    RuleAt(usize, u32),
    /// `precedence operand { rows }`, only ever a whole rule body.
    Table(Box<Table>),
}

/// An operator precedence table, which becomes a Pratt loop.
#[derive(Clone, Debug)]
pub struct Table {
    /// What an operator applies to, such as `_primary`.
    pub operand: Node,
    /// Weakest first.
    pub rows: Vec<Row>,
}

/// `[label:] (prefix | postfix | once | left | right) ops => node`.
#[derive(Clone, Debug)]
pub struct Row {
    pub label: Option<String>,
    pub fix: Fix,
    /// The operator: a token, or any expression such as `"not" "in"` or `"if" x "else"`.
    pub ops: Node,
    /// The node kind the row makes.
    pub node: String,
    pub spot: Spot,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fix {
    Prefix,
    Postfix,
    /// A postfix operator taken at most once: after it, the loop takes only
    /// operators of tighter rows. Go's `[]int{1}` in a header, whose body's `{`
    /// must not read as a second literal.
    Once,
    Left,
    Right,
}

pub fn grammar(input: Vec<Tok>) -> Result<Grammar, Error> {
    let mut input = input.into_iter();
    let (name, spot) = match input.next() {
        Some(Tok { tree: Tree::Ident(name), spot }) => (name, spot),
        other => return Err(Error::at(other.and_then(|tok| tok.spot), "expected the grammar's name, as in `grammar! { JSON { ... } }`")),
    };
    let body = match input.next() {
        Some(Tok { tree: Tree::Group(Delim::Brace, body), .. }) => body,
        other => return Err(Error::at(other.and_then(|tok| tok.spot).or(spot), format!("expected `{{ ... }}` after `{name}`"))),
    };
    if let Some(extra) = input.next() {
        return Err(Error::at(extra.spot, format!("unexpected tokens after the body of `{name}`")));
    }
    let mut grammar = Grammar {
        language: name.to_ascii_lowercase(),
        name,
        mode: Mode::Auto,
        lex: String::new(),
        runtime: "crate::deep".into(),
        tokens: Vec::new(),
        rules: Vec::new(),
        layout: None,
    };
    let mut lex_seen = false;
    let mut items = Stream::new(body);
    while let Some(first) = items.next() {
        let (word, spot) = match first.tree {
            Tree::Ident(word) => (word, first.spot),
            // `///` comments arrive as `#[doc = "..."]`; attributes are skipped.
            Tree::Punct('#', _) if matches!(items.peek(), Some(Tok { tree: Tree::Group(Delim::Bracket, _), .. })) => {
                items.next();
                continue;
            }
            other => return Err(Error::at(first.spot, format!("expected a setting, `tokens {{ ... }}` or a rule, found `{}`", other.describe()))),
        };
        match items.peek().map(|tok| &tok.tree) {
            Some(Tree::Punct('=', _)) => {
                items.next();
                let value = items.until(';');
                match word.as_str() {
                    "name" => grammar.language = string_value(&value, spot, "name")?,
                    "lex" => {
                        grammar.lex = path_value(&value, spot, "lex")?;
                        lex_seen = true;
                    }
                    "runtime" => grammar.runtime = path_value(&value, spot, "runtime")?,
                    "mode" => {
                        grammar.mode = match value.as_slice() {
                            [Tok { tree: Tree::Ident(word), .. }] if word == "auto" => Mode::Auto,
                            [Tok { tree: Tree::Ident(word), .. }] if word == "tables" => Mode::Tables,
                            [Tok { tree: Tree::Ident(word), .. }] if word == "recursive" => Mode::Recursive,
                            _ => return Err(Error::at(spot, "`mode` is `auto`, `tables` or `recursive`: `mode = tables;`")),
                        };
                    }
                    "layout" => match value.as_slice() {
                        [Tok { tree: Tree::Ident(newline), .. }, Tok { tree: Tree::Ident(indent), .. }, Tok { tree: Tree::Ident(dedent), .. }] => {
                            grammar.layout = Some(LayoutDef { newline: newline.clone(), indent: indent.clone(), dedent: dedent.clone(), spot });
                        }
                        _ => return Err(Error::at(spot, "`layout` takes three tokens: `layout = NEWLINE INDENT DEDENT;`")),
                    },
                    _ => return Err(Error::at(spot, format!("unknown setting `{word}`: the settings are `name`, `lex`, `runtime`, `layout` and `mode`"))),
                }
            }
            Some(Tree::Group(Delim::Brace, _)) if word == "tokens" => {
                let Some(Tok { tree: Tree::Group(_, body), .. }) = items.next() else { unreachable!("peeked") };
                if !grammar.tokens.is_empty() {
                    return Err(Error::at(spot, "only one `tokens { ... }` block is allowed"));
                }
                grammar.tokens = tokens(body)?;
            }
            Some(Tree::Punct(':', false)) => {
                items.next();
                let body = items.until(',');
                if body.is_empty() {
                    return Err(Error::at(spot, format!("rule `{word}` has an empty body")));
                }
                let body = match body.first() {
                    Some(Tok { tree: Tree::Ident(keyword), .. }) if keyword == "precedence" => table(body, spot)?,
                    _ => Expressions::new(body, spot).whole()?,
                };
                grammar.rules.push(Rule { name: word, body, spot });
            }
            _ => return Err(Error::at(spot, format!("expected `=`, `{{` or `:` after `{word}`"))),
        }
    }
    if !lex_seen {
        return Err(Error::at(spot, "a grammar needs a lexer: `lex = path::to::lex;`"));
    }
    if grammar.tokens.is_empty() {
        return Err(Error::at(spot, "a grammar needs a `tokens { ... }` block"));
    }
    if grammar.rules.is_empty() {
        return Err(Error::at(spot, "a grammar needs at least one rule"));
    }
    Ok(grammar)
}

/// `CONST = "text" [open family | close family | bad | trivia],`
fn tokens(body: Vec<Tok>) -> Result<Vec<TokenDef>, Error> {
    let mut defs = Vec::new();
    let mut items = Stream::new(body);
    while items.peek().is_some() {
        // `///` comments on a row arrive as `#[doc = "..."]`; attributes are skipped.
        while matches!(items.peek(), Some(Tok { tree: Tree::Punct('#', _), .. })) {
            items.next();
            items.next();
        }
        let mut row = items.until(',').into_iter();
        let (constant, spot) = match row.next() {
            Some(Tok { tree: Tree::Ident(constant), spot }) => (constant, spot),
            Some(other) => return Err(Error::at(other.spot, format!("expected a token constant, found `{}`", other.tree.describe()))),
            None => return Err(Error::at(None, "an empty row in `tokens`")),
        };
        if !matches!(row.next(), Some(Tok { tree: Tree::Punct('=', _), .. })) {
            return Err(Error::at(spot, format!("expected `=` after `{constant}`")));
        }
        let text = match row.next() {
            Some(Tok { tree: Tree::Literal(literal), spot }) => unquote(&literal).ok_or_else(|| Error::at(spot, "expected a string literal"))?,
            _ => return Err(Error::at(spot, format!("expected the text of `{constant}`, as in `{constant} = \"...\"`"))),
        };
        let mut words: Vec<Tok> = row.collect();
        let reset = matches!(words.last(), Some(Tok { tree: Tree::Ident(word), .. }) if word == "reset");
        if reset {
            words.pop();
        }
        let class = match words.as_slice() {
            [] => TokenClass::Token,
            [Tok { tree: Tree::Ident(word), .. }] if word == "bad" => TokenClass::Bad,
            [Tok { tree: Tree::Ident(word), .. }] if word == "trivia" => TokenClass::Trivia,
            [Tok { tree: Tree::Ident(word), .. }, Tok { tree: Tree::Ident(family), .. }] if word == "open" => TokenClass::Open(family.clone()),
            [Tok { tree: Tree::Ident(word), .. }, Tok { tree: Tree::Ident(family), .. }] if word == "close" => TokenClass::Close(family.clone()),
            [first, ..] => {
                return Err(Error::at(first.spot, format!("`{constant}`: expected nothing, `bad`, `trivia`, `open FAMILY`, `close FAMILY` or `reset` after the text")))
            }
        };
        if reset && class != TokenClass::Token {
            return Err(Error::at(spot, format!("`{constant}`: only a plain token can be `reset`")));
        }
        defs.push(TokenDef { constant, text, class, reset, spot });
    }
    Ok(defs)
}

/// `precedence operand { [label:] fix ops => node, ... }`
fn table(body: Vec<Tok>, spot: Spot) -> Result<Node, Error> {
    let mut body = body.into_iter();
    body.next();
    let operand = match body.next() {
        Some(tok @ Tok { tree: Tree::Ident(_), .. }) => Expressions::new(vec![tok], spot).whole()?,
        other => return Err(Error::at(other.and_then(|tok| tok.spot).or(spot), "expected the operand rule after `precedence`")),
    };
    let rows = match body.next() {
        Some(Tok { tree: Tree::Group(Delim::Brace, rows), .. }) => rows,
        other => return Err(Error::at(other.and_then(|tok| tok.spot).or(spot), "expected `{ rows }` after the operand")),
    };
    if let Some(extra) = body.next() {
        return Err(Error::at(extra.spot, "unexpected tokens after the precedence rows"));
    }
    let mut stream = Stream::new(rows);
    let mut out = Vec::new();
    while stream.peek().is_some() {
        let row = stream.until(',');
        let row_spot = row.first().and_then(|tok| tok.spot).or(spot);
        let mut at = 0;
        let label = match (row.first().map(|tok| &tok.tree), row.get(1).map(|tok| &tok.tree)) {
            (Some(Tree::Ident(label)), Some(Tree::Punct(':', false))) => {
                at = 2;
                Some(label.clone())
            }
            _ => None,
        };
        let fix = match row.get(at).map(|tok| &tok.tree) {
            Some(Tree::Ident(word)) if word == "prefix" => Fix::Prefix,
            Some(Tree::Ident(word)) if word == "postfix" => Fix::Postfix,
            Some(Tree::Ident(word)) if word == "once" => Fix::Once,
            Some(Tree::Ident(word)) if word == "left" => Fix::Left,
            Some(Tree::Ident(word)) if word == "right" => Fix::Right,
            _ => return Err(Error::at(row_spot, "a precedence row is `[label:] prefix | postfix | once | left | right OPERATOR => node`")),
        };
        let arrow = (at + 1..row.len().saturating_sub(1))
            .find(|&index| matches!(row[index].tree, Tree::Punct('=', true)) && matches!(row[index + 1].tree, Tree::Punct('>', _)));
        let Some(arrow) = arrow else {
            return Err(Error::at(row_spot, "a precedence row ends with `=> node`"));
        };
        let node = match row.get(arrow + 2..) {
            Some([Tok { tree: Tree::Ident(node), .. }]) => node.clone(),
            _ => return Err(Error::at(row_spot, "expected one node name after `=>`")),
        };
        if arrow == at + 1 {
            return Err(Error::at(row_spot, "a precedence row needs an operator"));
        }
        let ops = Expressions::new(row[at + 1..arrow].to_vec(), row_spot).whole()?;
        out.push(Row { label, fix, ops, node, spot: row_spot });
    }
    if out.is_empty() {
        return Err(Error::at(spot, "a precedence table needs at least one row"));
    }
    Ok(Node { expr: Expr::Table(Box::new(Table { operand, rows: out })), spot })
}

fn string_value(value: &[Tok], spot: Spot, setting: &str) -> Result<String, Error> {
    match value {
        [Tok { tree: Tree::Literal(literal), spot }] => unquote(literal).ok_or_else(|| Error::at(*spot, "expected a string literal")),
        _ => Err(Error::at(spot, format!("`{setting}` takes one string literal"))),
    }
}

/// A path such as `super::json::lex`, written back as text.
fn path_value(value: &[Tok], spot: Spot, setting: &str) -> Result<String, Error> {
    let mut path = String::new();
    for tok in value {
        match &tok.tree {
            Tree::Ident(word) => path.push_str(word),
            Tree::Punct(':', _) => path.push(':'),
            _ => return Err(Error::at(tok.spot, format!("`{setting}` takes a path"))),
        }
    }
    if path.is_empty() || path.starts_with(':') && !path.starts_with("::") || path.ends_with(':') {
        return Err(Error::at(spot, format!("`{setting}` takes a path")));
    }
    Ok(path)
}

/// The value of a Rust string literal: `"a\"b"` or `r#"a"b"#`. `None` for any
/// other literal.
pub fn unquote(literal: &str) -> Option<String> {
    if let Some(raw) = literal.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let inner = raw.get(hashes..raw.len().checked_sub(hashes)?)?;
        return inner.strip_prefix('"')?.strip_suffix('"').map(str::to_string);
    }
    let inner = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
impl Node {
    /// The label of row `row`, if this is a table.
    fn expr_label(&self, row: usize) -> Option<&str> {
        match &self.expr {
            Expr::Table(table) => table.rows[row].label.as_deref(),
            _ => None,
        }
    }
}

struct Stream {
    toks: std::iter::Peekable<std::vec::IntoIter<Tok>>,
}

impl Stream {
    fn new(toks: Vec<Tok>) -> Self {
        Stream { toks: toks.into_iter().peekable() }
    }

    fn next(&mut self) -> Option<Tok> {
        self.toks.next()
    }

    fn peek(&mut self) -> Option<&Tok> {
        self.toks.peek()
    }

    /// Everything up to the next top-level `stop`, which is consumed.
    fn until(&mut self, stop: char) -> Vec<Tok> {
        let mut taken = Vec::new();
        for tok in self.toks.by_ref() {
            if matches!(tok.tree, Tree::Punct(c, _) if c == stop) {
                break;
            }
            taken.push(tok);
        }
        taken
    }
}

/// A rule body:
///
/// ```text
/// choice  := seq ('|' seq)*
/// seq     := postfix+
/// postfix := atom ('*' | '+' | '?' | '%' atom)*
/// atom    := "text" | name | '(' choice ')' | '[' choice ']'
/// ```
struct Expressions {
    toks: Vec<Tok>,
    at: usize,
    /// Where to point an error at the end of the body.
    end: Spot,
}

impl Expressions {
    fn new(toks: Vec<Tok>, end: Spot) -> Self {
        Expressions { toks, at: 0, end }
    }

    fn whole(mut self) -> Result<Node, Error> {
        let node = self.choice()?;
        if let Some(extra) = self.toks.get(self.at) {
            return Err(Error::at(extra.spot, format!("unexpected `{}` in a rule body", extra.tree.describe())));
        }
        Ok(node)
    }

    fn peek_punct(&self) -> Option<char> {
        match self.toks.get(self.at).map(|tok| &tok.tree) {
            Some(Tree::Punct(c, _)) => Some(*c),
            _ => None,
        }
    }

    fn spot(&self) -> Spot {
        self.toks.get(self.at).map_or(self.end, |tok| tok.spot)
    }

    fn choice(&mut self) -> Result<Node, Error> {
        let spot = self.spot();
        let mut alternatives = vec![self.seq()?];
        while self.peek_punct() == Some('|') {
            self.at += 1;
            alternatives.push(self.seq()?);
        }
        Ok(if alternatives.len() == 1 { alternatives.pop().expect("one") } else { Node { expr: Expr::Choice(alternatives), spot } })
    }

    fn seq(&mut self) -> Result<Node, Error> {
        let spot = self.spot();
        let mut items = Vec::new();
        while self.at < self.toks.len() && self.peek_punct() != Some('|') {
            items.push(self.postfix()?);
        }
        match items.len() {
            0 => Err(Error::at(spot, "an empty alternative: use `[x]` for something optional")),
            1 => Ok(items.pop().expect("one")),
            _ => Ok(Node { expr: Expr::Seq(items), spot }),
        }
    }

    fn postfix(&mut self) -> Result<Node, Error> {
        let mut node = self.atom()?;
        loop {
            let spot = self.spot();
            node = match self.peek_punct() {
                Some('*') => Node { expr: Expr::Star(Box::new(node)), spot },
                Some('+') => Node { expr: Expr::Plus(Box::new(node)), spot },
                Some('?') => Node { expr: Expr::Opt(Box::new(node)), spot },
                Some('%') => {
                    self.at += 1;
                    let trailing = self.peek_punct() == Some('?');
                    if trailing {
                        self.at += 1;
                    }
                    let separator = self.atom()?;
                    node = Node { expr: Expr::Sep(Box::new(node), Box::new(separator), trailing), spot };
                    continue;
                }
                Some('@') => {
                    self.at += 1;
                    let Expr::Name(rule) = &node.expr else {
                        return Err(Error::at(spot, "`@level` follows the name of a precedence rule"));
                    };
                    let label = match self.toks.get(self.at).map(|tok| &tok.tree) {
                        Some(Tree::Ident(label)) => label.clone(),
                        _ => return Err(Error::at(self.spot(), "expected a row label after `@`")),
                    };
                    // The label is taken below, like a postfix operator.
                    Node { expr: Expr::Level(rule.clone(), label), spot: node.spot }
                }
                _ => return Ok(node),
            };
            self.at += 1;
        }
    }

    fn atom(&mut self) -> Result<Node, Error> {
        let Some(tok) = self.toks.get(self.at) else {
            return Err(Error::at(self.end, "a rule body ends where an item was expected"));
        };
        let spot = tok.spot;
        let expr = match &tok.tree {
            Tree::Literal(literal) => Expr::Literal(unquote(literal).ok_or_else(|| Error::at(spot, "expected a string literal"))?),
            Tree::Ident(name) => Expr::Name(name.clone()),
            Tree::Punct('&', _) => {
                self.at += 1;
                let inner = self.atom()?;
                return Ok(Node { expr: Expr::Pred(Box::new(inner)), spot });
            }
            Tree::Group(delim @ (Delim::Paren | Delim::Bracket), inner) => {
                let inner = Expressions::new(inner.clone(), spot).whole()?;
                self.at += 1;
                return Ok(if *delim == Delim::Bracket { Node { expr: Expr::Opt(Box::new(inner)), spot } } else { inner });
            }
            other => return Err(Error::at(spot, format!("unexpected `{}` in a rule body", other.describe()))),
        };
        self.at += 1;
        Ok(Node { expr, spot })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::toks;

    fn body(src: &str) -> String {
        let node = Expressions::new(toks(src), None).whole().unwrap();
        show(&node)
    }

    fn show(node: &Node) -> String {
        match &node.expr {
            Expr::Literal(text) => format!("{text:?}"),
            Expr::Name(name) => name.clone(),
            Expr::Token(index) => format!("t{index}"),
            Expr::Rule(index) => format!("r{index}"),
            Expr::Seq(items) => format!("(seq {})", items.iter().map(show).collect::<Vec<_>>().join(" ")),
            Expr::Choice(items) => format!("(alt {})", items.iter().map(show).collect::<Vec<_>>().join(" ")),
            Expr::Opt(inner) => format!("(opt {})", show(inner)),
            Expr::Star(inner) => format!("(star {})", show(inner)),
            Expr::Plus(inner) => format!("(plus {})", show(inner)),
            Expr::Sep(inner, separator, false) => format!("(sep {} {})", show(inner), show(separator)),
            Expr::Sep(inner, separator, true) => format!("(sep? {} {})", show(inner), show(separator)),
            Expr::Pred(inner) => format!("(pred {})", show(inner)),
            Expr::Level(rule, label) => format!("{rule}@{label}"),
            Expr::RuleAt(rule, level) => format!("r{rule}@{level}"),
            Expr::Table(table) => {
                let rows: Vec<String> = table.rows.iter().map(|row| format!("{:?} {} {}", row.fix, show(&row.ops), row.node)).collect();
                format!("(table {} {})", show(&table.operand), rows.join("; "))
            }
        }
    }

    #[test]
    fn bodies_parse_with_postfix_binding_tightest() {
        assert_eq!(body(r#""{" (pair % ",") "}""#), r#"(seq "{" (sep pair ",") "}")"#);
        assert_eq!(body("a b | c* d+ [e] f?"), "(alt (seq a b) (seq (star c) (plus d) (opt e) (opt f)))");
        assert_eq!(body(r#""def" IDENT "(" [params] ")" ":" block"#), r#"(seq "def" IDENT "(" (opt params) ")" ":" block)"#);
        assert_eq!(body("(a | b)*"), "(star (alt a b))");
        assert_eq!(body(r#"&(A b) c | (x %? ",") y@low"#), r#"(alt (seq (pred (seq A b)) c) (seq (sep? x ",") y@low))"#);
    }

    #[test]
    fn a_precedence_table_reads_its_rows() {
        let grammar = read(
            r#"G { lex = l; layout = NL IN DE; tokens { A = "a" reset, }
               _e: precedence _atom { low: right "=" => assign, left "+" | "-" => bin, prefix "-" => neg, postfix "(" ")" => call, }, }"#,
        )
        .unwrap();
        assert!(grammar.tokens[0].reset);
        let layout = grammar.layout.as_ref().unwrap();
        assert_eq!((layout.newline.as_str(), layout.indent.as_str(), layout.dedent.as_str()), ("NL", "IN", "DE"));
        assert_eq!(
            show(&grammar.rules[0].body),
            r#"(table _atom Right "=" assign; Left (alt "+" "-") bin; Prefix "-" neg; Postfix (seq "(" ")") call)"#
        );
        assert_eq!(grammar.rules[0].body.expr_label(0), Some("low"));
    }

    #[test]
    fn literals_unquote_like_rust() {
        assert_eq!(unquote(r#""a\"b\\""#).as_deref(), Some("a\"b\\"));
        assert_eq!(unquote(r##"r#"a"b"#"##).as_deref(), Some("a\"b"));
        assert_eq!(unquote("'c'"), None);
        assert_eq!(unquote("12"), None);
    }

    fn read(src: &str) -> Result<Grammar, String> {
        grammar(toks(src)).map_err(|error| error.message)
    }

    #[test]
    fn a_grammar_reads_settings_tokens_and_rules() {
        let grammar = read(
            r#"Calc { name = "calculator"; lex = super::lex; runtime = crate::rt;
               tokens { NUM = "num", LP = "(" open parens, RP = ")" close parens, JUNK = "junk" bad, }
               expr: NUM | group,
               group: "(" expr ")", }"#,
        )
        .unwrap();
        assert_eq!((grammar.name.as_str(), grammar.language.as_str()), ("Calc", "calculator"));
        assert_eq!((grammar.lex.as_str(), grammar.runtime.as_str()), ("super::lex", "crate::rt"));
        let classes: Vec<_> = grammar.tokens.iter().map(|token| (token.constant.as_str(), token.text.as_str(), token.class.clone())).collect();
        assert_eq!(
            classes,
            [
                ("NUM", "num", TokenClass::Token),
                ("LP", "(", TokenClass::Open("parens".into())),
                ("RP", ")", TokenClass::Close("parens".into())),
                ("JUNK", "junk", TokenClass::Bad)
            ]
        );
        assert_eq!(grammar.rules.iter().map(|rule| rule.name.as_str()).collect::<Vec<_>>(), ["expr", "group"]);
    }

    #[test]
    fn malformed_grammars_say_what_is_wrong() {
        for (src, want) in [
            ("{ }", "expected the grammar's name"),
            ("G x", "expected `{ ... }` after `G`"),
            ("G { } extra", "unexpected tokens after the body"),
            (r#"G { tokens { A = "a", } a: A, }"#, "needs a lexer"),
            ("G { lex = l; a: b, }", "needs a `tokens"),
            (r#"G { lex = l; tokens { A = "a", } }"#, "at least one rule"),
            (r#"G { lex = l; tokens { A = "a", } a: , }"#, "empty body"),
            (r#"G { lex = l; tokens { A = "a", } a: A | , }"#, "empty alternative"),
            (r#"G { lex = l; tokens { A = "a" weird, } a: A, }"#, "expected nothing, `bad`"),
            (r#"G { lex = l; tokens { A = b, } a: A, }"#, "expected the text of `A`"),
            (r#"G { lex = l; color = "red"; tokens { A = "a", } a: A, }"#, "unknown setting `color`"),
            (r#"G { lex = l; tokens { A = "a", } a: A %, }"#, "ends where an item was expected"),
            (r#"G { lex = "l"; tokens { A = "a", } a: A, }"#, "`lex` takes a path"),
            (r#"G { lex = l; layout = A; tokens { A = "a", } a: A, }"#, "`layout` takes three tokens"),
            (r#"G { lex = l; tokens { A = "a" bad reset, } a: A, }"#, "only a plain token can be `reset`"),
            (r#"G { lex = l; tokens { A = "a", } a: precedence b { up "+" => x, }, }"#, "a precedence row is"),
            (r#"G { lex = l; tokens { A = "a", } a: precedence b { left "+" x, }, }"#, "ends with `=> node`"),
            (r#"G { lex = l; tokens { A = "a", } a: precedence b { left => x, }, }"#, "needs an operator"),
            (r#"G { lex = l; tokens { A = "a", } a: "x"@y, }"#, "follows the name of a precedence rule"),
        ] {
            let error = read(src).err().unwrap_or_else(|| panic!("{src} was accepted"));
            assert!(error.contains(want), "{src}: {error}");
        }
    }
}
