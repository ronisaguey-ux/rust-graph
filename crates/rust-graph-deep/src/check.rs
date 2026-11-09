//! Resolves names and checks a grammar at compile time, computing what code
//! generation needs: which rules are groups, nullable, FIRST, FOLLOW, guards, and
//! which calls must go through the depth budget.
//!
//! Rejected, each with an error at the offending place: unknown names, left
//! recursion (direct or indirect), repetition of something that can be empty,
//! delimiters outside a group rule, unused rules, misplaced predicates, bad
//! layout or precedence declarations, and names that would clash in the
//! generated code. Alternatives that no run of up to three tokens tells apart
//! are rejected when the emitter builds their decision (`decide.rs`).
//!
//! A precedence table is analysed as the grammar it accepts,
//! `prefix* operand (infix self | postfix)*`, so FIRST, FOLLOW and decisions
//! treat it like any other rule; only the emitter writes it as a Pratt loop.

use crate::read::{Action, Expr, Fix, Grammar, Node, Table, TokenClass, TokenDef};
use crate::{Error, Spot};

/// Token indices, as a bit set.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Set(Vec<u64>);

impl Set {
    pub fn new(size: usize) -> Self {
        Set(vec![0; size.div_ceil(64)])
    }

    #[cfg(test)]
    pub fn of(size: usize, token: usize) -> Self {
        let mut set = Set::new(size);
        set.insert(token);
        set
    }

    pub fn insert(&mut self, token: usize) {
        self.0[token / 64] |= 1 << (token % 64);
    }

    pub fn contains(&self, token: usize) -> bool {
        self.0[token / 64] & (1 << (token % 64)) != 0
    }

    /// Adds `other`; true if that changed anything.
    pub fn union(&mut self, other: &Set) -> bool {
        let mut changed = false;
        for (word, add) in self.0.iter_mut().zip(&other.0) {
            changed |= *add & !*word != 0;
            *word |= add;
        }
        changed
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|word| *word == 0)
    }

    /// Keeps only what `other` has too.
    pub fn intersect(&mut self, other: &Set) {
        for (word, keep) in self.0.iter_mut().zip(&other.0) {
            *word &= keep;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.0.len() * 64).filter(|token| self.contains(*token))
    }
}

/// What a caller checks before parsing something, so the something can take its
/// leading tokens without looking.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Guard {
    /// The current token is one of these.
    Set(Set),
    /// The next tokens are exactly these, two or more.
    Prefix(Vec<usize>),
}

pub enum Shape {
    /// The first rule: the runtime opens its node and the body runs to the end.
    Root,
    /// `open inner close` with a matching pair: parsed by `Parser::group`, so
    /// the inner part sees `EOF` at the closer.
    Group(Option<Node>),
    /// A named node around its body.
    Node,
    /// A name starting with `_`: no node of its own, like tree-sitter's.
    Hidden,
    /// A hidden rule whose body is a precedence table: a Pratt loop.
    Table,
}

pub struct Checked {
    pub grammar: Grammar,
    /// Delimiter families, in order of first use.
    pub families: Vec<String>,
    pub shapes: Vec<Shape>,
    pub nullable: Vec<bool>,
    pub first: Vec<Set>,
    pub follow: Vec<Set>,
    /// `nested[from][to]`: a call from rule `from` to rule `to` closes a cycle
    /// that passes through no group, so it must go through the depth budget.
    pub nested: Vec<Vec<bool>>,
    /// Each rule's precedence table, resolved; its body holds the analysis form.
    pub tables: Vec<Option<Table>>,
    /// Node kinds made by precedence rows, in order of first use. They follow
    /// the named rules in the kind table.
    pub row_kinds: Vec<String>,
    /// Layout tokens: newline, indent, dedent.
    pub layout: Option<[usize; 3]>,
}

/// The constant naming a rule's node kind: `pair` -> `PAIR`.
pub fn rule_constant(name: &str) -> String {
    name.to_ascii_uppercase()
}

pub fn is_hidden(name: &str) -> bool {
    name.starts_with('_')
}

/// Name of the generated table of kinds.
pub const KINDS: &str = "KINDS";

pub fn check(mut grammar: Grammar) -> Result<Checked, Error> {
    let families = check_tokens(&grammar)?;
    let layout = check_layout(&mut grammar)?;
    check_rule_names(&grammar)?;
    check_modes(&grammar)?;
    let labels = table_labels(&grammar)?;
    let mut tables = Vec::new();
    for index in 0..grammar.rules.len() {
        let mut body = grammar.rules[index].body.clone();
        if let Expr::Table(table) = body.expr {
            let mut table = *table;
            if !is_hidden(&grammar.rules[index].name) {
                return Err(Error::at(grammar.rules[index].spot, "a precedence rule has no node of its own (its rows make them): name it `_like_this`"));
            }
            resolve(&grammar, &labels, &mut table.operand)?;
            for row in &mut table.rows {
                resolve(&grammar, &labels, &mut row.ops)?;
            }
            body = desugar(index, &table, body.spot);
            tables.push(Some(table));
        } else {
            resolve(&grammar, &labels, &mut body)?;
            tables.push(None);
        }
        grammar.rules[index].body = body;
    }
    let row_kinds = row_kinds(&grammar, &tables)?;
    let mut shapes = shapes(&grammar)?;
    for (shape, table) in shapes.iter_mut().zip(&tables) {
        if table.is_some() {
            *shape = Shape::Table;
        }
    }
    let edges: Vec<Vec<usize>> = grammar.rules.iter().map(|rule| references(&rule.body)).collect();
    check_reachable(&grammar, &edges)?;
    let size = grammar.tokens.len();
    let mut checked = Checked {
        nullable: vec![false; grammar.rules.len()],
        first: vec![Set::new(size); grammar.rules.len()],
        follow: vec![Set::new(size); grammar.rules.len()],
        nested: Vec::new(),
        grammar,
        families,
        shapes,
        tables,
        row_kinds,
        layout,
    };
    checked.solve_first();
    checked.check_left_recursion()?;
    for index in 0..checked.grammar.rules.len() {
        checked.check_node(&checked.grammar.rules[index].body, false)?;
        if let Some(table) = &checked.tables[index] {
            if checked.nullable(&table.operand) {
                return Err(Error::at(table.operand.spot, "a precedence operand must not match nothing"));
            }
            for row in &table.rows {
                // An operator after an operand may start with a predicate, as
                // TypeScript's `f<T>(x)` must against `a < b`.
                checked.check_node(&row.ops, row.fix != Fix::Prefix)?;
                if checked.nullable(&row.ops) {
                    return Err(Error::at(row.spot, "an operator must not match nothing"));
                }
            }
        }
    }
    checked.solve_follow();
    checked.nested = nested_calls(&checked.shapes, &edges);
    Ok(checked)
}

fn check_tokens(grammar: &Grammar) -> Result<Vec<String>, Error> {
    let tokens = &grammar.tokens;
    let mut families: Vec<String> = Vec::new();
    let mut opens: Vec<usize> = Vec::new();
    let mut closes: Vec<usize> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !is_constant(&token.constant) {
            return Err(Error::at(token.spot, format!("token constants are UPPER_CASE: `{}`", token.constant)));
        }
        if token.constant == KINDS || token.constant == grammar.name {
            return Err(Error::at(token.spot, format!("`{}` is a name the generated code uses", token.constant)));
        }
        if let Some(earlier) = tokens[..index].iter().find(|earlier| earlier.constant == token.constant) {
            return Err(Error::at(token.spot, format!("token `{}` is declared twice", earlier.constant)));
        }
        if let Some(earlier) = tokens[..index].iter().find(|earlier| earlier.text == token.text) {
            return Err(Error::at(token.spot, format!("`{}` and `{}` have the same text {:?}", earlier.constant, token.constant, token.text)));
        }
        if let TokenClass::Open(family) | TokenClass::Close(family) = &token.class {
            let id = families.iter().position(|known| known == family).unwrap_or_else(|| {
                families.push(family.clone());
                opens.push(0);
                closes.push(0);
                families.len() - 1
            });
            if matches!(token.class, TokenClass::Open(_)) {
                opens[id] += 1;
            } else {
                closes[id] += 1;
            }
        }
    }
    for (id, family) in families.iter().enumerate() {
        if opens[id] != 1 || closes[id] != 1 {
            let spot = tokens.iter().find(|token| matches!(&token.class, TokenClass::Open(f) | TokenClass::Close(f) if f == family)).and_then(|token| token.spot);
            return Err(Error::at(spot, format!("delimiter family `{family}` needs exactly one `open` and one `close` token")));
        }
    }
    if families.len() > usize::from(u8::MAX) + 1 {
        return Err(Error::at(None, "more than 256 delimiter families"));
    }
    if tokens.len() + grammar.rules.len() > usize::from(u16::MAX) - 16 {
        return Err(Error::at(None, "too many kinds for a u16"));
    }
    Ok(families)
}

/// The layout tokens, as `[newline, indent, dedent]`.
///
/// `layout = NEWLINE INDENT DEDENT;` names a plain token and an `open`/`close`
/// pair of one family. `layout = NEWLINE;` names only the line break: the
/// runtime still infers a zero-width `newline` after the last token of each
/// logical line (and none inside brackets or on blank lines), but there is no
/// block structure, so two *trivia* kinds are added to the token table to stand
/// in for the indentation pair. They are never emitted by the grammar's lexer
/// and the parser never sees them, because trivia is skipped: the language gets
/// line ends as syntax without indentation. A rule can therefore end at a line
/// end, which is what Elixir's `case`/`cond` clauses need to be told apart.
fn check_layout(grammar: &mut Grammar) -> Result<Option<[usize; 3]>, Error> {
    let find = |name: &str, spot| {
        grammar.tokens.iter().position(|token| token.constant == name).ok_or_else(|| Error::at(spot, format!("`{name}` is not a token")))
    };
    if grammar.layout.is_none() {
        if let Some(token) = grammar.tokens.iter().find(|token| token.reset) {
            return Err(Error::at(token.spot, "`reset` only means something with `layout = ...;`"));
        }
        return Ok(None);
    }
    let layout = grammar.layout.as_ref().expect("checked");
    let (newline, indent, dedent, spot) = (layout.newline.clone(), layout.indent.clone(), layout.dedent.clone(), layout.spot);
    let newline = find(&newline, spot)?;
    let (Some(indent), Some(dedent)) = (indent, dedent) else {
        // Line ends only. The two synthetic kinds follow the grammar's tokens,
        // so they get kind numbers of their own and every rule still refers to
        // the same tokens as before.
        if grammar.tokens[newline].class != TokenClass::Token {
            return Err(Error::at(spot, "`layout = NEWLINE;` names one plain token, the line break"));
        }
        let indent = grammar.tokens.len();
        let synthetic = |constant: &str, text: &str| TokenDef {
            constant: constant.into(),
            text: text.into(),
            class: TokenClass::Trivia,
            reset: false,
            spot,
        };
        grammar.tokens.push(synthetic("__DEEP_INDENT", "indent"));
        grammar.tokens.push(synthetic("__DEEP_DEDENT", "dedent"));
        return Ok(Some([newline, indent, indent + 1]));
    };
    let indent = find(&indent, spot)?;
    let dedent = find(&dedent, spot)?;
    match (&grammar.tokens[newline].class, &grammar.tokens[indent].class, &grammar.tokens[dedent].class) {
        (TokenClass::Token, TokenClass::Open(a), TokenClass::Close(b)) if a == b => Ok(Some([newline, indent, dedent])),
        _ => Err(Error::at(spot, "layout needs a plain newline token, then the `open` and `close` of one family")),
    }
}

/// `modes` and `switch`: mode names are UPPER_CASE and unique, every switch
/// names a declared token and a declared mode, and no token switches twice.
fn check_modes(grammar: &Grammar) -> Result<(), Error> {
    for (index, mode) in grammar.modes.iter().enumerate() {
        if !is_constant(&mode.name) {
            return Err(Error::at(mode.spot, format!("mode names are UPPER_CASE: `{}`", mode.name)));
        }
        if grammar.modes[..index].iter().any(|earlier| earlier.name == mode.name) {
            return Err(Error::at(mode.spot, format!("mode `{}` is declared twice", mode.name)));
        }
    }
    for (index, switch) in grammar.switches.iter().enumerate() {
        if !grammar.tokens.iter().any(|token| token.constant == switch.token) {
            return Err(Error::at(switch.spot, format!("`{}` is not a token", switch.token)));
        }
        if grammar.switches[..index].iter().any(|earlier| earlier.token == switch.token) {
            return Err(Error::at(switch.spot, format!("`{}` switches twice: one switch per token, the rest belongs in the step function", switch.token)));
        }
        if let Action::Set(mode) | Action::Push(mode) | Action::PopTo(mode) = &switch.action {
            if !grammar.modes.iter().any(|def| def.name == *mode) {
                return Err(Error::at(switch.spot, format!("`{mode}` is not a mode: declare it in `modes {{ ... }}`")));
            }
        }
    }
    Ok(())
}

/// For each precedence rule, its row labels, in order.
fn table_labels(grammar: &Grammar) -> Result<Vec<Option<Vec<Option<String>>>>, Error> {
    let mut out = Vec::new();
    for rule in &grammar.rules {
        let Expr::Table(table) = &rule.body.expr else {
            out.push(None);
            continue;
        };
        let labels: Vec<Option<String>> = table.rows.iter().map(|row| row.label.clone()).collect();
        for (at, row) in table.rows.iter().enumerate() {
            if row.label.is_some() && labels[..at].contains(&row.label) {
                return Err(Error::at(row.spot, format!("row label `{}` is used twice", row.label.as_deref().unwrap_or_default())));
            }
        }
        out.push(Some(labels));
    }
    Ok(out)
}

/// What a table accepts, as an ordinary rule body:
/// `(prefix ops)* operand ((infix ops) self | postfix ops)*`.
fn desugar(rule: usize, table: &Table, spot: crate::Spot) -> Node {
    let node = |expr| Node { expr, spot };
    let choice = |mut items: Vec<Node>| if items.len() == 1 { items.pop().expect("one") } else { node(Expr::Choice(items)) };
    let mut items = Vec::new();
    let prefix: Vec<Node> = table.rows.iter().filter(|row| row.fix == Fix::Prefix).map(|row| row.ops.clone()).collect();
    if !prefix.is_empty() {
        items.push(node(Expr::Star(Box::new(choice(prefix)))));
    }
    items.push(table.operand.clone());
    let tails: Vec<Node> = table
        .rows
        .iter()
        .filter(|row| row.fix != Fix::Prefix)
        .map(|row| if matches!(row.fix, Fix::Postfix | Fix::Once) { row.ops.clone() } else { node(Expr::Seq(vec![row.ops.clone(), node(Expr::Rule(rule))])) })
        .collect();
    if !tails.is_empty() {
        items.push(node(Expr::Star(Box::new(choice(tails)))));
    }
    if items.len() == 1 {
        items.pop().expect("one")
    } else {
        node(Expr::Seq(items))
    }
}

/// Node kinds that precedence rows make, without repeats. A row may share its
/// kind with other rows, but not with a rule or a token.
fn row_kinds(grammar: &Grammar, tables: &[Option<Table>]) -> Result<Vec<String>, Error> {
    let mut kinds: Vec<String> = Vec::new();
    for row in tables.iter().flatten().flat_map(|table| &table.rows) {
        if !is_rule_name(&row.node) || is_hidden(&row.node) {
            return Err(Error::at(row.spot, format!("a row's node is a snake_case name: `{}`", row.node)));
        }
        let constant = rule_constant(&row.node);
        if grammar.rules.iter().any(|rule| rule.name.trim_start_matches('_') == row.node)
            || grammar.tokens.iter().any(|token| token.constant == constant)
            || constant == KINDS
            || constant == grammar.name
        {
            return Err(Error::at(row.spot, format!("row node `{}` would define `{constant}`, which is already taken", row.node)));
        }
        if !kinds.contains(&row.node) {
            kinds.push(row.node.clone());
        }
    }
    Ok(kinds)
}

fn is_constant(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_rule_name(name: &str) -> bool {
    let bare = name.strip_prefix('_').unwrap_or(name);
    bare.starts_with(|c: char| c.is_ascii_lowercase()) && bare.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_rule_names(grammar: &Grammar) -> Result<(), Error> {
    for (index, rule) in grammar.rules.iter().enumerate() {
        if !is_rule_name(&rule.name) {
            return Err(Error::at(rule.spot, format!("rule names are snake_case, with an optional leading `_`: `{}`", rule.name)));
        }
        let bare = rule.name.trim_start_matches('_');
        if let Some(earlier) = grammar.rules[..index].iter().find(|earlier| earlier.name.trim_start_matches('_') == bare) {
            let what = if earlier.name == rule.name { format!("rule `{}` is defined twice", rule.name) } else { format!("rules `{}` and `{}` would share generated names", earlier.name, rule.name) };
            return Err(Error::at(rule.spot, what));
        }
        if !is_hidden(&rule.name) {
            let constant = rule_constant(&rule.name);
            if constant == KINDS || constant == grammar.name || grammar.tokens.iter().any(|token| token.constant == constant) {
                return Err(Error::at(rule.spot, format!("rule `{}` would define `{constant}`, which is already taken", rule.name)));
            }
        }
    }
    Ok(())
}

fn resolve(grammar: &Grammar, labels: &[Option<Vec<Option<String>>>], node: &mut Node) -> Result<(), Error> {
    let spot = node.spot;
    match &mut node.expr {
        Expr::Level(name, label) => {
            let rule = grammar.rules.iter().position(|rule| rule.name == *name).ok_or_else(|| Error::at(spot, format!("`{name}` is not a rule")))?;
            let Some(rows) = &labels[rule] else {
                return Err(Error::at(spot, format!("`{name}` is not a precedence rule, so it has no levels")));
            };
            let row = rows.iter().position(|row| row.as_deref() == Some(label.as_str())).ok_or_else(|| Error::at(spot, format!("`{name}` has no row labelled `{label}`")))?;
            node.expr = Expr::RuleAt(rule, row as u32 + 1);
        }
        Expr::Pred(inner) => resolve(grammar, labels, inner)?,
        Expr::Table(_) => return Err(Error::at(spot, "a precedence table must be a whole rule body")),
        Expr::Literal(text) => {
            let token = grammar.tokens.iter().position(|token| token.text == *text);
            node.expr = Expr::Token(token.ok_or_else(|| Error::at(spot, format!("no token has the text {text:?}: declare it in `tokens`")))?);
        }
        Expr::Name(name) => {
            node.expr = if let Some(rule) = grammar.rules.iter().position(|rule| rule.name == *name) {
                Expr::Rule(rule)
            } else if let Some(token) = grammar.tokens.iter().position(|token| token.constant == *name) {
                Expr::Token(token)
            } else {
                return Err(Error::at(spot, format!("`{name}` is neither a rule nor a token")));
            };
        }
        Expr::Token(_) | Expr::Rule(_) | Expr::RuleAt(..) => {}
        Expr::Seq(items) | Expr::Choice(items) => {
            for item in items {
                resolve(grammar, labels, item)?;
            }
        }
        Expr::Opt(inner) | Expr::Star(inner) | Expr::Plus(inner) => resolve(grammar, labels, inner)?,
        Expr::Sep(inner, separator, _) => {
            resolve(grammar, labels, inner)?;
            resolve(grammar, labels, separator)?;
        }
    }
    if let Expr::Token(token) = node.expr {
        if grammar.tokens[token].class == TokenClass::Trivia {
            return Err(Error::at(spot, format!("`{}` is trivia, which rules never see", grammar.tokens[token].constant)));
        }
    }
    Ok(())
}

fn shapes(grammar: &Grammar) -> Result<Vec<Shape>, Error> {
    let mut shapes = Vec::new();
    for (index, rule) in grammar.rules.iter().enumerate() {
        let shape = if index == 0 {
            if is_hidden(&rule.name) {
                return Err(Error::at(rule.spot, "the first rule is the root, which needs a node: drop the `_`"));
            }
            Shape::Root
        } else if let Some(inner) = group_inner(grammar, &rule.body) {
            if is_hidden(&rule.name) {
                return Err(Error::at(rule.spot, format!("`{}` is a group, which needs a node: drop the `_`", rule.name)));
            }
            Shape::Group(inner)
        } else if is_hidden(&rule.name) {
            Shape::Hidden
        } else {
            Shape::Node
        };
        let rest = match (&shape, &rule.body.expr) {
            (Shape::Group(inner), _) => inner.as_ref(),
            // A table's rows are checked as they are emitted: an operator there may
            // not open a group either, but its operand rule and rows are rules.
            (_, Expr::Table(_)) => None,
            _ => Some(&rule.body),
        };
        if let Some(spot) = rest.and_then(|node| find_delimiter(grammar, node)) {
            return Err(Error::at(
                spot,
                "a delimiter may only open and close a whole rule, `name: \"(\" ... \")\"`: give this group a rule of its own",
            ));
        }
        shapes.push(shape);
    }
    Ok(shapes)
}

/// `Some(inner)` if `body` is `open inner close` of one family.
fn group_inner(grammar: &Grammar, body: &Node) -> Option<Option<Node>> {
    let Expr::Seq(items) = &body.expr else { return None };
    let (Expr::Token(open), Expr::Token(close)) = (&items[0].expr, &items[items.len() - 1].expr) else { return None };
    match (&grammar.tokens[*open].class, &grammar.tokens[*close].class) {
        (TokenClass::Open(a), TokenClass::Close(b)) if a == b => {}
        _ => return None,
    }
    let middle = &items[1..items.len() - 1];
    Some(match middle.len() {
        0 => None,
        1 => Some(middle[0].clone()),
        _ => Some(Node { expr: Expr::Seq(middle.to_vec()), spot: middle[0].spot }),
    })
}

/// The first *opening* delimiter outside a whole-rule group. A lone closer is
/// an ordinary terminal: the parser takes a closer only where a rule names it
/// (`p.bump()`), and it never opens anything, so shell's `case x in pat) ... ;;`
/// is expressible. An opener is still refused, because `p.bump()` would take it
/// alone and the group firewall (`Parser::group`) would not be established.
fn find_delimiter(grammar: &Grammar, node: &Node) -> Option<Spot> {
    match &node.expr {
        Expr::Token(token) if matches!(grammar.tokens[*token].class, TokenClass::Open(_)) => Some(node.spot),
        Expr::Seq(items) | Expr::Choice(items) => items.iter().find_map(|item| find_delimiter(grammar, item)),
        Expr::Opt(inner) | Expr::Star(inner) | Expr::Plus(inner) | Expr::Pred(inner) => find_delimiter(grammar, inner),
        Expr::Sep(inner, separator, _) => find_delimiter(grammar, inner).or_else(|| find_delimiter(grammar, separator)),
        Expr::Table(table) => find_delimiter(grammar, &table.operand).or_else(|| table.rows.iter().find_map(|row| find_delimiter(grammar, &row.ops))),
        _ => None,
    }
}

/// Every rule `node` refers to, in order, without repeats.
fn references(node: &Node) -> Vec<usize> {
    fn walk(node: &Node, out: &mut Vec<usize>) {
        match &node.expr {
            Expr::Rule(rule) | Expr::RuleAt(rule, _) if !out.contains(rule) => out.push(*rule),
            Expr::Seq(items) | Expr::Choice(items) => items.iter().for_each(|item| walk(item, out)),
            Expr::Opt(inner) | Expr::Star(inner) | Expr::Plus(inner) | Expr::Pred(inner) => walk(inner, out),
            Expr::Sep(inner, separator, _) => {
                walk(inner, out);
                walk(separator, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn check_reachable(grammar: &Grammar, edges: &[Vec<usize>]) -> Result<(), Error> {
    if let Some(from) = edges.iter().position(|targets| targets.contains(&0)) {
        return Err(Error::at(grammar.rules[from].spot, format!("`{}` refers to the root rule `{}`, which only the runtime opens", grammar.rules[from].name, grammar.rules[0].name)));
    }
    let mut seen = vec![false; grammar.rules.len()];
    let mut stack = vec![0];
    seen[0] = true;
    while let Some(rule) = stack.pop() {
        for &next in &edges[rule] {
            if !seen[next] {
                seen[next] = true;
                stack.push(next);
            }
        }
    }
    match seen.iter().position(|seen| !seen) {
        Some(unused) => Err(Error::at(grammar.rules[unused].spot, format!("rule `{}` is never used", grammar.rules[unused].name))),
        None => Ok(()),
    }
}

/// For each rule, the rules it can reach through calls made outside any group.
fn nested_calls(shapes: &[Shape], edges: &[Vec<usize>]) -> Vec<Vec<bool>> {
    let count = shapes.len();
    let unguarded = |rule: usize| -> &[usize] {
        // A group's body is behind its opener, which the depth budget counts.
        if matches!(shapes[rule], Shape::Group(_)) {
            &[]
        } else {
            &edges[rule]
        }
    };
    let reach: Vec<Vec<bool>> = (0..count)
        .map(|start| {
            let mut seen = vec![false; count];
            let mut stack = vec![start];
            while let Some(rule) = stack.pop() {
                for &next in unguarded(rule) {
                    if !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
            seen
        })
        .collect();
    (0..count).map(|from| (0..count).map(|to| unguarded(from).contains(&to) && (to == from || reach[to][from])).collect()).collect()
}

impl Checked {
    pub fn size(&self) -> usize {
        self.grammar.tokens.len()
    }

    pub fn nullable(&self, node: &Node) -> bool {
        match &node.expr {
            Expr::Token(_) => false,
            Expr::Rule(rule) | Expr::RuleAt(rule, _) => self.nullable[*rule],
            Expr::Seq(items) => items.iter().all(|item| self.nullable(item)),
            Expr::Choice(items) => items.iter().any(|item| self.nullable(item)),
            Expr::Opt(_) | Expr::Star(_) | Expr::Sep(..) | Expr::Pred(_) => true,
            Expr::Plus(inner) => self.nullable(inner),
            Expr::Literal(_) | Expr::Name(_) | Expr::Level(..) | Expr::Table(_) => unreachable!("resolved"),
        }
    }

    pub fn first(&self, node: &Node) -> Set {
        let mut set = Set::new(self.size());
        match &node.expr {
            Expr::Token(token) => set.insert(*token),
            Expr::Rule(rule) | Expr::RuleAt(rule, _) => {
                set.union(&self.first[*rule]);
            }
            Expr::Pred(_) => {}
            // `&p x` is only taken where `p` matches, so it starts with what
            // both `p` and `x` can start with: a loop over statements whose
            // declarations are guarded by their modifiers does not take a
            // token that only an unguarded declaration could start.
            Expr::Seq(items) if matches!(items.first().map(|item| &item.expr), Some(Expr::Pred(_))) && items[1..].iter().any(|item| !self.nullable(item)) => {
                let Expr::Pred(inner) = &items[0].expr else { unreachable!("matched") };
                set = self.first(inner);
                set.intersect(&self.first_of_rest(&items[1..], &Set::new(self.size())));
            }
            Expr::Seq(items) => {
                for item in items {
                    set.union(&self.first(item));
                    if !self.nullable(item) {
                        break;
                    }
                }
            }
            Expr::Choice(items) => items.iter().for_each(|item| {
                set.union(&self.first(item));
            }),
            Expr::Opt(inner) | Expr::Star(inner) | Expr::Plus(inner) | Expr::Sep(inner, ..) => {
                set.union(&self.first(inner));
            }
            Expr::Literal(_) | Expr::Name(_) | Expr::Level(..) | Expr::Table(_) => unreachable!("resolved"),
        }
        set
    }

    /// FIRST of `items`, then `after` if they can all be empty: what may follow
    /// the item before them.
    pub fn first_of_rest(&self, items: &[Node], after: &Set) -> Set {
        let mut set = Set::new(self.size());
        for item in items {
            set.union(&self.first(item));
            if !self.nullable(item) {
                return set;
            }
        }
        set.union(after);
        set
    }

    fn solve_first(&mut self) {
        loop {
            let mut changed = false;
            for rule in 0..self.grammar.rules.len() {
                let (nullable, first) = match &self.shapes[rule] {
                    Shape::Group(_) => {
                        let Expr::Seq(items) = &self.grammar.rules[rule].body.expr else { unreachable!("a group is a sequence") };
                        (false, self.first(&items[0]))
                    }
                    _ => (self.nullable(&self.grammar.rules[rule].body), self.first(&self.grammar.rules[rule].body)),
                };
                changed |= nullable != self.nullable[rule];
                self.nullable[rule] = nullable;
                changed |= self.first[rule].union(&first);
            }
            if !changed {
                return;
            }
        }
    }

    /// Rules that `node` can call before taking any token.
    fn left_calls(&self, node: &Node, out: &mut Vec<(usize, Spot)>) {
        match &node.expr {
            Expr::Rule(rule) | Expr::RuleAt(rule, _) => out.push((*rule, node.spot)),
            Expr::Seq(items) => {
                for item in items {
                    self.left_calls(item, out);
                    if !self.nullable(item) {
                        break;
                    }
                }
            }
            Expr::Choice(items) => items.iter().for_each(|item| self.left_calls(item, out)),
            Expr::Opt(inner) | Expr::Star(inner) | Expr::Plus(inner) | Expr::Sep(inner, ..) | Expr::Pred(inner) => self.left_calls(inner, out),
            _ => {}
        }
    }

    fn check_left_recursion(&self) -> Result<(), Error> {
        let rules = &self.grammar.rules;
        let left: Vec<Vec<(usize, Spot)>> = (0..rules.len())
            .map(|rule| {
                let mut out = Vec::new();
                if !matches!(self.shapes[rule], Shape::Group(_)) {
                    self.left_calls(&rules[rule].body, &mut out);
                }
                out
            })
            .collect();
        // 0 unvisited, 1 on the path, 2 done.
        let mut state = vec![0u8; rules.len()];
        for start in 0..rules.len() {
            if state[start] != 0 {
                continue;
            }
            let mut path: Vec<(usize, usize)> = vec![(start, 0)];
            state[start] = 1;
            while let Some(&mut (rule, ref mut next)) = path.last_mut() {
                if let Some(&(target, spot)) = left[rule].get(*next) {
                    *next += 1;
                    match state[target] {
                        0 => {
                            state[target] = 1;
                            path.push((target, 0));
                        }
                        1 => {
                            let from = path.iter().position(|(on, _)| *on == target).expect("on the path");
                            let mut cycle: Vec<&str> = path[from..].iter().map(|(on, _)| rules[*on].name.as_str()).collect();
                            cycle.push(&rules[target].name);
                            return Err(Error::at(spot, format!("left recursion: {}; use repetition such as `x (op x)*`", cycle.join(" -> "))));
                        }
                        _ => {}
                    }
                } else {
                    state[rule] = 2;
                    path.pop();
                }
            }
        }
        Ok(())
    }

    /// `alternative`: `node` is a whole alternative of a choice, where a
    /// sequence may start with a predicate.
    fn check_node(&self, node: &Node, alternative: bool) -> Result<(), Error> {
        match &node.expr {
            Expr::Seq(items) => {
                for (at, item) in items.iter().enumerate() {
                    if let Expr::Pred(inner) = &item.expr {
                        if !(alternative && at == 0) {
                            return Err(Error::at(item.spot, "a predicate `&x` may only start an alternative of a choice"));
                        }
                        self.check_node(inner, false)?;
                        if self.nullable(inner) {
                            return Err(Error::at(item.spot, "a predicate must not match nothing"));
                        }
                    } else {
                        self.check_node(item, false)?;
                    }
                }
                Ok(())
            }
            Expr::Pred(_) => Err(Error::at(node.spot, "a predicate `&x` starts an alternative and is followed by what it guards")),
            Expr::Choice(items) => {
                for item in items {
                    self.check_node(item, true)?;
                    if self.nullable(item) {
                        return Err(Error::at(item.spot, "an alternative can match nothing: make the whole choice optional instead"));
                    }
                }
                Ok(())
            }
            Expr::Opt(inner) | Expr::Star(inner) | Expr::Plus(inner) | Expr::Sep(inner, ..) => {
                self.check_node(inner, false)?;
                if !matches!(node.expr, Expr::Opt(_)) && self.nullable(inner) {
                    return Err(Error::at(inner.spot, "repeating something that can match nothing would never end"));
                }
                if let Expr::Sep(_, separator, _) = &node.expr {
                    if !matches!(separator.expr, Expr::Token(_)) {
                        return Err(Error::at(separator.spot, "a separator is one token"));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn solve_follow(&mut self) {
        loop {
            let mut follow = self.follow.clone();
            for (rule, shape) in self.shapes.iter().enumerate() {
                let empty = Set::new(self.size());
                match shape {
                    Shape::Root => self.walk_follow(&self.grammar.rules[rule].body, &empty, &mut follow),
                    Shape::Group(Some(inner)) => self.walk_follow(inner, &empty, &mut follow),
                    Shape::Group(None) => {}
                    Shape::Node | Shape::Hidden | Shape::Table => self.walk_follow(&self.grammar.rules[rule].body, &self.follow[rule], &mut follow),
                }
            }
            if follow == self.follow {
                return;
            }
            self.follow = follow;
        }
    }

    fn walk_follow(&self, node: &Node, after: &Set, follow: &mut [Set]) {
        match &node.expr {
            Expr::Rule(rule) | Expr::RuleAt(rule, _) => {
                follow[*rule].union(after);
            }
            Expr::Pred(inner) => self.walk_follow(inner, &Set::new(self.size()), follow),
            Expr::Seq(items) => {
                for at in 0..items.len() {
                    self.walk_follow(&items[at], &self.first_of_rest(&items[at + 1..], after), follow);
                }
            }
            Expr::Choice(items) => items.iter().for_each(|item| self.walk_follow(item, after, follow)),
            Expr::Opt(inner) => self.walk_follow(inner, after, follow),
            Expr::Star(inner) | Expr::Plus(inner) => self.walk_follow(inner, &self.repeat_after(inner, after), follow),
            Expr::Sep(inner, separator, _) => self.walk_follow(inner, &self.separated_after(separator, after), follow),
            _ => {}
        }
    }

    /// What may follow one `x` in `x*`: another `x`, or what follows the loop.
    pub fn repeat_after(&self, inner: &Node, after: &Set) -> Set {
        let mut set = self.first(inner);
        set.union(after);
        set
    }

    /// What may follow one `x` in `x % sep`: the separator, or what follows the list.
    pub fn separated_after(&self, separator: &Node, after: &Set) -> Set {
        let mut set = self.first(separator);
        set.union(after);
        set
    }

    /// See [`Guard`]. A sequence is guarded by its leading tokens, or else by
    /// its first item's guard when that item cannot be empty.
    pub fn guard(&self, node: &Node) -> Guard {
        match &node.expr {
            Expr::Rule(rule) | Expr::RuleAt(rule, _) => self.rule_guard(*rule),
            Expr::Seq(items) => {
                let leading = self.leading_tokens(items);
                if leading.len() >= 2 {
                    Guard::Prefix(leading)
                } else if !leading.is_empty() || !self.nullable(&items[0]) {
                    self.guard(&items[0])
                } else {
                    Guard::Set(self.first(node))
                }
            }
            Expr::Plus(inner) => self.guard(inner),
            _ => Guard::Set(self.first(node)),
        }
    }

    pub fn rule_guard(&self, rule: usize) -> Guard {
        match self.shapes[rule] {
            Shape::Group(_) => Guard::Set(self.first[rule].clone()),
            _ => self.guard(&self.grammar.rules[rule].body),
        }
    }

    pub fn leading_tokens(&self, items: &[Node]) -> Vec<usize> {
        items
            .iter()
            .map_while(|item| match item.expr {
                Expr::Token(token) => Some(token),
                _ => None,
            })
            .collect()
    }

    /// The family index of a delimiter token.
    pub fn family(&self, token: usize) -> Option<u8> {
        match &self.grammar.tokens[token].class {
            TokenClass::Open(family) | TokenClass::Close(family) => self.families.iter().position(|known| known == family).map(|id| id as u8),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::toks;

    fn checked(rules: &str) -> Result<Checked, String> {
        let src = format!(
            r#"G {{ lex = lex;
               tokens {{ A = "a", B = "b", C = "c", COMMA = ",", LP = "(" open parens, RP = ")" close parens, WS = "ws" trivia, }}
               {rules} }}"#
        );
        let grammar = crate::read::grammar(toks(&src)).map_err(|error| error.message)?;
        check(grammar).map_err(|error| error.message)
    }

    fn rejects(rules: &str, want: &str) {
        match checked(rules) {
            Ok(_) => panic!("accepted: {rules}"),
            Err(error) => assert!(error.contains(want), "{rules}: {error}"),
        }
    }

    fn names(checked: &Checked, set: &Set) -> String {
        set.iter().map(|token| checked.grammar.tokens[token].text.as_str()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn first_follow_and_nullable_are_solved() {
        let g = checked(r#"top: item*, item: [A] x | group, x: B C?, group: "(" (x % ",") ")","#).unwrap();
        assert_eq!(g.nullable, [true, false, false, false]);
        assert_eq!(names(&g, &g.first[1]), "a b (");
        assert_eq!(names(&g, &g.first[3]), "(");
        assert_eq!(names(&g, &g.follow[1]), "a b (");
        // `x` is followed by what follows `item` and, inside the group, by `,` only.
        assert_eq!(names(&g, &g.follow[2]), "a b , (");
        assert!(matches!(g.shapes[3], Shape::Group(Some(_))));
        assert!(matches!(g.shapes[0], Shape::Root));
    }

    #[test]
    fn guards_take_the_leading_tokens() {
        let g = checked(r#"top: (p | q)*, p: A B q, q: C,"#).unwrap();
        assert_eq!(g.rule_guard(1), Guard::Prefix(vec![0, 1]));
        assert_eq!(g.rule_guard(2), Guard::Set(Set::of(g.size(), 2)));
    }

    #[test]
    fn a_predicate_narrows_what_its_alternative_starts_with() {
        // `x` alone starts with A or B, but `&(A) x` is only taken on A, so a
        // loop over `y` claims A and C, not B.
        let g = checked(r#"top: y* [B], y: &(A) x | C, x: A | B,"#).unwrap();
        assert_eq!(names(&g, &g.first[1]), "a c");
        assert_eq!(names(&g, &g.first[2]), "a b");
    }

    #[test]
    fn recursion_outside_groups_goes_through_the_budget() {
        let g = checked(r#"top: x*, x: A x | B | group, group: "(" x ")","#).unwrap();
        assert!(g.nested[1][1], "x -> x passes through no group");
        assert!(!g.nested[2][1], "group -> x is inside the group");
        assert!(!g.nested[1][2], "x -> group closes no unguarded cycle");
    }

    #[test]
    fn a_lone_closer_is_an_ordinary_terminal() {
        // Shell's `case x in pat) ... ;; esac`: `)` closes nothing here, so it
        // is a plain token in the middle of a rule.
        let g = checked(r#"top: item*, item: A ")" B | group, group: "(" A ")","#).unwrap();
        assert!(matches!(g.shapes[2], Shape::Group(Some(_))));
        // An *opening* delimiter still has to open a whole rule: taken alone it
        // would not establish the group firewall.
        rejects(r#"top: x, x: A "(" B ")" "#, "give this group a rule of its own");
        rejects(r#"top: "(" A ")" "#, "give this group a rule of its own");
    }

    #[test]
    fn bad_grammars_are_rejected_where_they_go_wrong() {
        rejects("top: x, x: x A,", "left recursion: x -> x");
        rejects("top: x, x: y A, y: [B] x,", "left recursion: x -> y -> x");
        rejects("top: _e, _e: precedence A { left B => bin, left C => bin, x: left C => cat, x: left B => dog, },", "row label `x` is used twice");
        rejects("top: e, e: precedence A { left B => bin, },", "has no node of its own");
        rejects("top: _e, _e: precedence A { left B => top, },", "already taken");
        rejects("top: _e@nope, _e: precedence A { left B => bin, },", "no row labelled `nope`");
        rejects("top: x@y, x: A,", "not a precedence rule");
        rejects("top: A &(B) C,", "may only start an alternative");
        rejects("top: _e, _e: precedence _a { left B => bin, }, _a: [A],", "must not match nothing");
        rejects("top: A | [B],", "can match nothing");
        rejects("top: [A]*,", "would never end");
        rejects("top: A % x, x: B C,", "a separator is one token");
        rejects("top: nope,", "`nope` is neither a rule nor a token");
        rejects(r#"top: "zz","#, "no token has the text \"zz\"");
        rejects("top: A, unused: B,", "rule `unused` is never used");
        rejects("top: x, x: top,", "refers to the root rule");
        rejects(r#"top: x, x: A "(" B ")","#, "give this group a rule of its own");
        rejects(r#"top: "(" A ")","#, "give this group a rule of its own");
        rejects(r#"top: _g, _g: "(" A ")","#, "is a group, which needs a node");
        rejects("_top: A,", "the root, which needs a node");
        rejects("top: WS,", "is trivia");
        rejects("top: x, x: A, x: B,", "defined twice");
        rejects("top: x, x: A, _x: B,", "would share generated names");
        rejects("top: lp, lp: A,", "would define `LP`");
        rejects("top: Bad,", "neither a rule nor a token");
        rejects("Top: A,", "snake_case");
    }

    #[test]
    fn a_newline_only_layout_adds_trivia_indentation_kinds() {
        let src = r#"G { lex = l; layout = NL;
                       tokens { NL = "newline", A = "a", }
                       top: A NL?, }"#;
        let g = check(crate::read::grammar(toks(src)).unwrap()).unwrap();
        let [newline, indent, dedent] = g.layout.expect("a layout");
        assert_eq!((newline, indent, dedent), (0, 2, 3), "the synthetic kinds follow the declared tokens");
        assert_eq!(g.grammar.tokens[indent].class, TokenClass::Trivia);
        assert_eq!(g.grammar.tokens[dedent].class, TokenClass::Trivia);
        assert_eq!(g.grammar.tokens[indent].constant, "__DEEP_INDENT");
        // A rule cannot name them, because trivia never reaches the parser.
        let named = r#"G { lex = l; layout = NL;
                       tokens { NL = "newline", A = "a", }
                       top: A NL? | __DEEP_INDENT, }"#;
        let error = check(crate::read::grammar(toks(named)).unwrap()).err().expect("a rule naming a synthetic kind").message;
        assert!(error.contains("is trivia"), "{error}");
    }

    #[test]
    fn modes_must_be_upper_case_and_switches_must_name_them() {
        let check_src = |modes: &str, switches: &str| {
            let src = format!(r#"G {{ lex = lex; {modes} {switches} tokens {{ A = "a", B = "b", }} top: A, }}"#);
            check(crate::read::grammar(toks(&src)).unwrap()).err().map(|error| error.message)
        };
        assert!(check_src(r#"modes { lower = l, }"#, "").unwrap().contains("UPPER_CASE"));
        let error = check_src(r#"modes { X = l, }"#, "switch A => Y;").unwrap();
        assert!(error.contains("`Y` is not a mode"), "{error}");
        let error = check_src(r#"modes { X = l, }"#, "switch NOPE => X;").unwrap();
        assert!(error.contains("`NOPE` is not a token"), "{error}");
        let error = check_src(r#"modes { X = l, }"#, "switch A => X; switch A => pop;").unwrap();
        assert!(error.contains("switches twice"), "{error}");
        // `pop MODE` names a mode like the other two do.
        let error = check_src(r#"modes { X = l, }"#, "switch A => pop Y;").unwrap();
        assert!(error.contains("`Y` is not a mode"), "{error}");
        assert!(check_src(r#"modes { X = l, }"#, "switch A => pop X;").is_none());
        // A mode no switch enters is unused but harmless: the step function may
        // still be reached, and a later switch can be added without reordering.
        assert!(check_src(r#"modes { X = l, }"#, "").is_none());
        assert!(check_src(r#"modes { X = l, }"#, "switch A => X; switch B => pop;").is_none());
    }

    #[test]
    fn bad_token_tables_are_rejected() {
        let read = |tokens: &str| {
            let src = format!("G {{ lex = lex; tokens {{ {tokens} }} top: A, }}");
            check(crate::read::grammar(toks(&src)).unwrap()).err().map(|error| error.message).unwrap_or_default()
        };
        assert!(read(r#"A = "a", A = "b","#).contains("declared twice"));
        assert!(read(r#"A = "a", B = "a","#).contains("same text"));
        assert!(read(r#"A = "a", L = "(" open p,"#).contains("exactly one `open` and one `close`"));
        assert!(read(r#"A = "a", Low = "l","#).contains("UPPER_CASE"));
        assert!(read(r#"A = "a", KINDS = "k","#).contains("generated code uses"));
        assert!(read(r#"A = "a" reset,"#).contains("only means something with `layout"));
    }
}
