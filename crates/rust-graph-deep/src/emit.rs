//! Writes a checked grammar as Rust: kind constants, the kind table, the
//! `Language` static and one function per rule, all on the parser runtime.
//!
//! Decisions: a choice is a decision tree on the next one to three tokens
//! (`decide.rs`) and commits as soon as one alternative is left. An option or
//! repetition is entered on its first token when nothing that may follow it
//! starts the same way; otherwise, and in the body of a group or the root, on
//! its whole leading run of tokens. A rule takes the leading tokens that every
//! caller has already checked without looking again.
//!
//! Recovery comes from the grammar alone:
//!
//! - a required item that is absent becomes an empty `ERROR` flagged `MISSING`
//!   when the next token may follow it (or the group ends), and otherwise the
//!   tokens up to one that may follow it become one `ERROR`;
//! - the body of the root and of every group runs to the end: `x*` wraps what
//!   cannot start an `x` in an `ERROR` up to the next `x`, and `x % sep` makes an
//!   `ERROR` of a stray separator or of anything up to the next separator;
//! - an alternative that matches nothing takes one token as an `ERROR`.
//!
//! A precedence table becomes `parse_<rule>_at(p, min)`, a Pratt loop: prefix
//! rows and the operand, then infix and postfix rows while their level is at
//! least `min`. Row `i` (weakest first) is level `i + 1`; a left-associative
//! row parses its right side at `level + 1`, a right-associative or prefix row
//! at `level`. `rule@label` enters the loop at that row's level.

use std::cell::{Cell, RefCell};
use std::fmt::Write;

use crate::check::{is_hidden, rule_constant, Checked, Guard, Set, Shape, KINDS};
use crate::decide::Decision;
use crate::read::{Expr, Fix, Node, TokenClass};
use crate::Error;

type Out = Result<String, Error>;

pub fn emit(checked: &Checked) -> Out {
    // First pass: what each rule's callers check. The code a rule's callers
    // emit does not depend on what the rule trusts, so one pass is enough.
    let probe = Emitter::new(checked, vec![0; checked.grammar.rules.len()]);
    probe.items()?;
    let trusted = probe.sites.into_inner().into_iter().map(|least| if least == usize::MAX { 0 } else { least }).collect();
    Emitter::new(checked, trusted).items()
}

struct Emitter<'c> {
    checked: &'c Checked,
    /// The rule being written, for [`Checked::nested`].
    rule: Cell<usize>,
    rt: &'c str,
    /// Per rule: how many of its leading tokens every caller has checked.
    trusted: Vec<usize>,
    /// The least each call site has checked, recorded as code is written.
    sites: RefCell<Vec<usize>>,
    /// Predicate functions, written after the rules.
    extra: RefCell<String>,
}

impl<'c> Emitter<'c> {
    fn new(checked: &'c Checked, trusted: Vec<usize>) -> Self {
        Emitter {
            checked,
            rule: Cell::new(0),
            rt: &checked.grammar.runtime,
            trusted,
            sites: RefCell::new(vec![usize::MAX; checked.grammar.rules.len()]),
            extra: RefCell::new(String::new()),
        }
    }

    fn items(&self) -> Out {
        let grammar = &self.checked.grammar;
        let rt = self.rt;
        let mut out = String::new();
        let mut kinds = String::new();
        let mut offset = 0usize;
        for (index, token) in grammar.tokens.iter().enumerate() {
            let class = match &token.class {
                TokenClass::Token => "Token".to_string(),
                TokenClass::Bad => "BadToken".to_string(),
                TokenClass::Trivia => "Trivia".to_string(),
                TokenClass::Open(_) => format!("Open({})", self.checked.family(index).expect("a delimiter")),
                TokenClass::Close(_) => format!("Close({})", self.checked.family(index).expect("a delimiter")),
            };
            self.kind_row(&mut out, &mut kinds, &token.constant, &token.text, &class, offset);
            offset += 1;
        }
        for rule in &grammar.rules {
            if !is_hidden(&rule.name) {
                self.kind_row(&mut out, &mut kinds, &rule_constant(&rule.name), &rule.name, "Node", offset);
                offset += 1;
            }
        }
        for node in &self.checked.row_kinds {
            self.kind_row(&mut out, &mut kinds, &rule_constant(node), node, "Node", offset);
            offset += 1;
        }
        let _ = write!(out, "const {KINDS}: &[{rt}::syntax::KindInfo] = &[{kinds}];");
        let layout = match self.checked.layout {
            None => String::new(),
            Some([newline, indent, dedent]) => {
                let resets: Vec<String> = grammar.tokens.iter().filter(|token| token.reset).map(|token| token.constant.clone()).collect();
                format!(
                    ".with_layout(&{rt}::syntax::Layout {{ newline: {}, indent: {}, dedent: {}, resets: &[{}] }})",
                    self.constant(newline),
                    self.constant(indent),
                    self.constant(dedent),
                    resets.join(", ")
                )
            }
        };
        let root = &grammar.rules[0].name;
        let _ = write!(
            out,
            "pub static {}: {rt}::syntax::Language = {rt}::syntax::Language::new({:?}, {}, {KINDS}, {}, {}){layout};",
            grammar.name,
            grammar.language,
            rule_constant(root),
            grammar.lex,
            parse_fn(root),
        );
        for rule in 0..grammar.rules.len() {
            self.rule.set(rule);
            out.push_str(&self.rule_fn(rule)?);
        }
        out.push_str(&self.extra.borrow());
        Ok(out)
    }

    fn kind_row(&self, out: &mut String, kinds: &mut String, constant: &str, name: &str, class: &str, offset: usize) {
        let rt = self.rt;
        let _ = write!(out, "pub const {constant}: {rt}::syntax::Kind = {rt}::syntax::FIRST_LANGUAGE_KIND + {offset};");
        let _ = write!(kinds, "{rt}::syntax::KindInfo {{ name: {name:?}, class: {rt}::syntax::Class::{class} }},");
    }

    fn parser(&self) -> String {
        format!("{}::parser::Parser", self.rt)
    }

    fn rule_fn(&self, rule: usize) -> Out {
        let checked = self.checked;
        let definition = &checked.grammar.rules[rule];
        let name = &definition.name;
        let parser = self.parser();
        let signature = format!("pub fn {}(p: &mut {parser})", parse_fn(name));
        let trusted = self.trusted[rule];
        Ok(match &checked.shapes[rule] {
            Shape::Root => format!("{signature} {{ {} }}", self.fill(&definition.body)?),
            Shape::Group(inner) => {
                let body = match inner {
                    Some(inner) => format!("fn {}(p: &mut {parser}) {{ {} }}", group_fn(name), self.fill(inner)?),
                    None => format!("fn {}(_: &mut {parser}) {{}}", group_fn(name)),
                };
                format!("{signature} {{ p.group({}, {}); }} {body}", rule_constant(name), group_fn(name))
            }
            Shape::Node => format!(
                "{signature} {{ let marker = p.open(); {} p.close(marker, {}); }}",
                self.guarded(&definition.body, &checked.follow[rule], trusted)?,
                rule_constant(name)
            ),
            Shape::Hidden => format!("{signature} {{ {} }}", self.guarded(&definition.body, &checked.follow[rule], trusted)?),
            Shape::Table => self.pratt(rule)?,
        })
    }

    /// The body of the root or of a group, which runs to the end.
    fn fill(&self, node: &Node) -> Out {
        let checked = self.checked;
        let empty = Set::new(checked.size());
        Ok(match &node.expr {
            Expr::Star(inner) => {
                let guard = checked.guard(inner);
                format!(
                    "while !p.at_end() {{ if {} {{ {} }} else {{ p.error_until({}); }} }}",
                    self.cond(&guard, 0),
                    self.guarded(inner, &checked.repeat_after(inner, &empty), guard_len(&guard))?,
                    self.list(&checked.first(inner)),
                )
            }
            Expr::Sep(inner, separator, trailing) => {
                let Expr::Token(token) = separator.expr else { unreachable!("checked: a separator is a token") };
                let separator_kind = self.constant(token);
                let guard = checked.guard(inner);
                let next = if *trailing { "separator_trailing" } else { "separator" };
                format!(
                    "while !p.at_end() {{ if {} {{ {} }} else if p.at({separator_kind}) {{ p.error_one(); continue; }} else {{ p.error_until(&[{separator_kind}]); }} if !p.{next}({separator_kind}) {{ break; }} }}",
                    self.cond(&guard, 0),
                    self.guarded(inner, &checked.separated_after(separator, &empty), guard_len(&guard))?,
                )
            }
            _ => self.required(node, &empty)?,
        })
    }

    /// How an option or repetition of `inner`, followed by `after`, decides to
    /// enter: on its first token, unless that may also follow it.
    fn entry(&self, inner: &Node, after: &Set) -> Guard {
        let first = self.checked.first(inner);
        if first.iter().any(|token| after.contains(token)) {
            self.checked.guard(inner)
        } else {
            Guard::Set(first)
        }
    }

    /// Parses `node`, whose first `trusted` leading tokens (and the guard of its
    /// first item, if `trusted` is not 0) the caller has checked. Nullable forms
    /// check for themselves. `after` is what may follow `node`.
    fn guarded(&self, node: &Node, after: &Set, trusted: usize) -> Out {
        let checked = self.checked;
        Ok(match &node.expr {
            Expr::Token(_) => "p.bump();".into(),
            Expr::Rule(rule) => self.call(*rule, trusted),
            Expr::RuleAt(rule, level) => self.call_at(*rule, *level),
            Expr::Seq(items) => {
                let leading = checked.leading_tokens(items).len();
                let covered = trusted.min(leading).max(usize::from(trusted > 0 && !checked.nullable(&items[0])));
                let mut out = String::new();
                for at in 0..items.len() {
                    let rest = checked.first_of_rest(&items[at + 1..], after);
                    out.push_str(&if at < covered { self.guarded(&items[at], &rest, if at == 0 { trusted } else { 1 })? } else { self.required(&items[at], &rest)? });
                }
                out
            }
            Expr::Choice(items) => self.choice(items, after, node)?,
            Expr::Opt(inner) => {
                let guard = self.entry(inner, after);
                format!("if {} {{ {} }}", self.cond(&guard, 0), self.guarded(inner, after, guard_len(&guard))?)
            }
            Expr::Star(inner) => self.repeat(inner, after)?,
            Expr::Plus(inner) => format!("{}{}", self.guarded(inner, &checked.repeat_after(inner, after), trusted)?, self.repeat(inner, after)?),
            Expr::Sep(inner, separator, trailing) => {
                let Expr::Token(token) = separator.expr else { unreachable!("checked: a separator is a token") };
                let item_after = checked.separated_after(separator, after);
                let guard = self.entry(inner, &item_after);
                let item = self.guarded(inner, &item_after, guard_len(&guard))?;
                let sep = self.constant(token);
                if *trailing {
                    if after.contains(token) {
                        return Err(Error::at(separator.spot, "a trailing separator here could also be what follows the list: use `%`"));
                    }
                    format!("if {} {{ {item} while p.at({sep}) {{ p.bump(); if !({}) {{ break; }} {item} }} }}", self.cond(&guard, 0), self.cond(&guard, 0))
                } else {
                    format!("if {} {{ {item} while p.at({sep}) && {} {{ p.bump(); {item} }} }}", self.cond(&guard, 0), self.cond(&guard, 1))
                }
            }
            Expr::Pred(_) => unreachable!("checked: a predicate only starts an alternative"),
            Expr::Literal(_) | Expr::Name(_) | Expr::Level(..) | Expr::Table(_) => unreachable!("resolved"),
        })
    }

    fn repeat(&self, inner: &Node, after: &Set) -> Out {
        let checked = self.checked;
        let guard = self.entry(inner, &checked.repeat_after(inner, after));
        Ok(format!("while {} {{ {} }}", self.cond(&guard, 0), self.guarded(inner, &checked.repeat_after(inner, after), guard_len(&guard))?))
    }

    /// Parses `node`, which must be here; see the module comment for recovery.
    /// It is entered on its first token, so a wrong token later still leaves
    /// its node in place with an error inside.
    fn required(&self, node: &Node, after: &Set) -> Out {
        let checked = self.checked;
        if checked.nullable(node) {
            return self.guarded(node, after, 0);
        }
        if let Expr::Token(token) = node.expr {
            return Ok(format!("p.expect({});", self.constant(token)));
        }
        let mut stop = "p.at_end()".to_string();
        if !after.is_empty() {
            let _ = write!(stop, " || {}", self.cond(&Guard::Set(after.clone()), 0));
        }
        Ok(format!(
            "if {} {{ {} }} else if {stop} {{ p.missing(); }} else {{ p.error_until({}); }}",
            self.cond(&Guard::Set(checked.first(node)), 0),
            self.guarded(node, after, 1)?,
            self.list(after),
        ))
    }

    /// A decision tree on up to three tokens. Alternatives that start with a
    /// predicate are tried first, in order, before the tree.
    fn choice(&self, items: &[Node], after: &Set, node: &Node) -> Out {
        let checked = self.checked;
        let (guarded_by_predicate, plain): (Vec<&Node>, Vec<&Node>) = items.iter().partition(|item| matches!(&item.expr, Expr::Seq(seq) if matches!(seq[0].expr, Expr::Pred(_))));
        let decision = checked.decide(&plain, after, node.spot, true)?;
        let mut out = self.decision(&decision, &|alt, matched| self.guarded(plain[alt], after, matched), "p.error_one();")?;
        for item in guarded_by_predicate.iter().rev() {
            let Expr::Seq(seq) = &item.expr else { unreachable!("partitioned") };
            let Expr::Pred(inner) = &seq[0].expr else { unreachable!("partitioned") };
            let rest = if seq.len() == 2 { seq[1].clone() } else { Node { expr: Expr::Seq(seq[1..].to_vec()), spot: seq[1].spot } };
            let predicate = self.predicate(inner)?;
            out = format!(
                "if {} && {predicate}(p) {{ {} }} else {{ {out} }}",
                self.cond(&Guard::Set(checked.first(inner)), 0),
                self.guarded(&rest, after, 0)?,
            );
        }
        Ok(out)
    }

    /// `decision` as nested `match`es. `take(alt, matched)` writes an
    /// alternative; `none` is what runs when no alternative matches.
    fn decision(&self, decision: &Decision, take: &dyn Fn(usize, usize) -> Out, none: &str) -> Out {
        Ok(match decision {
            Decision::Take { alt, matched } => take(*alt, *matched)?,
            Decision::Branch { depth, arms, default } => {
                let mut out = if *depth == 0 { "match p.current() {".to_string() } else { format!("match p.nth({depth}) {{") };
                for (tokens, inner) in arms {
                    let pattern: Vec<String> = tokens.iter().map(|token| self.constant(*token)).collect();
                    let _ = write!(out, " {} => {{ {} }}", pattern.join(" | "), self.decision(inner, take, none)?);
                }
                match default {
                    Some(inner) => {
                        let _ = write!(out, " _ => {{ {} }} }}", self.decision(inner, take, none)?);
                    }
                    None => {
                        let _ = write!(out, " _ => {{ {none} }} }}");
                    }
                }
                out
            }
        })
    }

    /// A function that tries `inner` and reports whether it parsed with no
    /// error, then rewinds: the input is untouched either way.
    fn predicate(&self, inner: &Node) -> Out {
        let mut extra = self.extra.borrow_mut();
        let name = format!("predicate_{}", extra.matches("fn predicate_").count());
        let body = {
            drop(extra);
            self.required(inner, &Set::new(self.checked.size()))?
        };
        extra = self.extra.borrow_mut();
        let _ = write!(
            extra,
            "fn {name}(p: &mut {}) -> bool {{ let checkpoint = p.checkpoint(); let errors = p.errors(); {body} let parsed = p.errors() == errors; p.rewind(checkpoint); parsed }}",
            self.parser()
        );
        Ok(name)
    }

    /// The Pratt loop for a precedence rule.
    fn pratt(&self, rule: usize) -> Out {
        let checked = self.checked;
        let table = checked.tables[rule].as_ref().expect("a table rule");
        let name = parse_fn(&checked.grammar.rules[rule].name);
        let at = format!("{name}_at");
        let parser = self.parser();
        let me = Guard::Set(checked.first[rule].clone());
        // What may follow an operand: an operator, or what follows the rule.
        let mut after_operand = checked.follow[rule].clone();
        for row in table.rows.iter().filter(|row| row.fix != Fix::Prefix) {
            after_operand.union(&checked.first(&row.ops));
        }
        let mut after_any = after_operand.clone();
        after_any.union(&checked.first[rule]);
        let operand = |level: u32| format!("if {} {{ p.nested(|p| {at}(p, {level})); }} else {{ p.missing(); }}", self.cond(&me, 0));

        let mut nud: Vec<(&Node, Option<usize>)> = Vec::new();
        for (index, row) in table.rows.iter().enumerate() {
            if row.fix == Fix::Prefix {
                nud.extend(alternatives(&row.ops).into_iter().map(|alt| (alt, Some(index))));
            }
        }
        nud.push((&table.operand, None));
        let nodes: Vec<&Node> = nud.iter().map(|(node, _)| *node).collect();
        let decision = checked.decide(&nodes, &after_any, table.operand.spot, true)?;
        let prefix = self.decision(
            &decision,
            &|alt, matched| match nud[alt] {
                (node, Some(index)) => Ok(format!(
                    "let marker = p.open(); {} {} p.close(marker, {});",
                    self.guarded(node, &checked.first[rule], matched)?,
                    operand(index as u32 + 1),
                    rule_constant(&table.rows[index].node)
                )),
                (node, None) => self.guarded(node, &after_operand, matched),
            },
            "p.error_one();",
        )?;

        let mut led: Vec<(&Node, usize)> = Vec::new();
        for (index, row) in table.rows.iter().enumerate() {
            if row.fix != Fix::Prefix {
                led.extend(alternatives(&row.ops).into_iter().map(|alt| (alt, index)));
            }
        }
        let tail = if led.is_empty() {
            "let _ = (start, min);".to_string()
        } else {
            let nodes: Vec<&Node> = led.iter().map(|(node, _)| *node).collect();
            let decision = checked.decide(&nodes, &after_any, table.rows[led[0].1].spot, false)?;
            let body = self.decision(
                &decision,
                &|alt, matched| {
                    let (node, index) = led[alt];
                    let row = &table.rows[index];
                    let level = index as u32 + 1;
                    let (ops_after, right) = match row.fix {
                        Fix::Left => (&checked.first[rule], operand(level + 1)),
                        Fix::Right => (&checked.first[rule], operand(level)),
                        _ => (&after_operand, String::new()),
                    };
                    let mut out = format!(
                        "if {level} < min {{ break; }} let marker = p.wrap(start); {} {right} p.close(marker, {});",
                        self.guarded(node, ops_after, matched)?,
                        rule_constant(&row.node)
                    );
                    // An operator is taken on its whole leading run: `not` alone
                    // after an operand is not `not in`.
                    if let Guard::Prefix(tokens) = checked.guard(node) {
                        if tokens.len() > matched {
                            out = format!("if {} {{ {out} }} else {{ break; }}", self.cond(&Guard::Prefix(tokens[matched..].to_vec()), matched as u32));
                        }
                    }
                    Ok(out)
                },
                "break;",
            )?;
            format!("loop {{ {body} }}")
        };
        Ok(format!(
            "pub fn {name}(p: &mut {parser}) {{ {at}(p, 0); }} pub fn {at}(p: &mut {parser}, min: u32) {{ let start = p.event_count(); {prefix} {tail} }}"
        ))
    }

    fn call(&self, rule: usize, trusted: usize) -> String {
        {
            let mut sites = self.sites.borrow_mut();
            sites[rule] = sites[rule].min(trusted);
        }
        let name = parse_fn(&self.checked.grammar.rules[rule].name);
        if self.checked.nested[self.rule.get()][rule] {
            format!("p.nested({name});")
        } else {
            format!("{name}(p);")
        }
    }

    fn call_at(&self, rule: usize, level: u32) -> String {
        let name = format!("{}_at", parse_fn(&self.checked.grammar.rules[rule].name));
        if self.checked.nested[self.rule.get()][rule] {
            format!("p.nested(|p| {name}(p, {level}));")
        } else {
            format!("{name}(p, {level});")
        }
    }

    /// The test for `guard`, starting `offset` tokens ahead.
    fn cond(&self, guard: &Guard, offset: u32) -> String {
        match guard {
            Guard::Set(set) => {
                let kinds: Vec<String> = set.iter().map(|token| self.constant(token)).collect();
                match (kinds.len(), offset) {
                    (0, _) => "false".into(),
                    (1, 0) => format!("p.at({})", kinds[0]),
                    (_, 0) => format!("matches!(p.current(), {})", kinds.join(" | ")),
                    (1, _) => format!("p.nth({offset}) == {}", kinds[0]),
                    (_, _) => format!("matches!(p.nth({offset}), {})", kinds.join(" | ")),
                }
            }
            Guard::Prefix(tokens) => {
                let mut parts = Vec::new();
                for (at, token) in tokens.iter().enumerate() {
                    let ahead = offset + at as u32;
                    parts.push(if ahead == 0 { format!("p.at({})", self.constant(*token)) } else { format!("p.nth({ahead}) == {}", self.constant(*token)) });
                }
                parts.join(" && ")
            }
        }
    }

    fn list(&self, set: &Set) -> String {
        let kinds: Vec<String> = set.iter().map(|token| self.constant(token)).collect();
        format!("&[{}]", kinds.join(", "))
    }

    fn constant(&self, token: usize) -> String {
        self.checked.grammar.tokens[token].constant.clone()
    }
}

/// How many leading tokens a guard checks.
fn guard_len(guard: &Guard) -> usize {
    match guard {
        Guard::Set(_) => 1,
        Guard::Prefix(tokens) => tokens.len(),
    }
}

/// The alternatives of an operator: `"+" | "-"` is two.
fn alternatives(node: &Node) -> Vec<&Node> {
    match &node.expr {
        Expr::Choice(items) => items.iter().collect(),
        _ => vec![node],
    }
}

pub fn parse_fn(rule: &str) -> String {
    format!("parse_{}", rule.trim_start_matches('_'))
}

fn group_fn(rule: &str) -> String {
    format!("group_{}", rule.trim_start_matches('_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::toks;

    fn try_emit(rules: &str) -> Result<String, String> {
        let src = format!(
            r#"G {{ lex = lex; runtime = rt;
               tokens {{ A = "a", B = "b", C = "c", D = "d", COMMA = ",", PLUS = "+", STAR = "*", MINUS = "-", EQ = "=", NOT = "not", IN = "in",
                         LP = "(" open parens, RP = ")" close parens, }}
               {rules} }}"#
        );
        let checked = crate::check::check(crate::read::grammar(toks(&src)).map_err(|error| error.message)?).map_err(|error| error.message)?;
        emit(&checked).map_err(|error| error.message)
    }

    fn emitted(rules: &str) -> String {
        try_emit(rules).unwrap()
    }

    #[test]
    fn the_kind_table_lists_tokens_then_named_rules_then_rows() {
        let out = emitted("top: _x*, _x: A | list | _e, list: \"(\" (B % \",\") \")\", _e: precedence C { left \"+\" => sum, },");
        assert!(out.contains("pub const A: rt::syntax::Kind = rt::syntax::FIRST_LANGUAGE_KIND + 0;"), "{out}");
        assert!(out.contains("pub const TOP: rt::syntax::Kind = rt::syntax::FIRST_LANGUAGE_KIND + 13;"), "{out}");
        assert!(out.contains("pub const LIST: rt::syntax::Kind = rt::syntax::FIRST_LANGUAGE_KIND + 14;"), "{out}");
        assert!(out.contains("pub const SUM: rt::syntax::Kind = rt::syntax::FIRST_LANGUAGE_KIND + 15;"), "{out}");
        assert!(!out.contains("pub const X"), "a hidden rule has no kind: {out}");
        assert!(out.contains(r#"rt::syntax::KindInfo { name: "(", class: rt::syntax::Class::Open(0) }"#), "{out}");
        assert!(out.contains(r#"pub static G: rt::syntax::Language = rt::syntax::Language::new("g", TOP, KINDS, lex, parse_top);"#), "{out}");
    }

    #[test]
    fn rules_become_runtime_calls() {
        let out = emitted("top: _x*, _x: A | list, list: \"(\" (B % \",\") \")\",");
        assert!(out.contains("pub fn parse_top(p: &mut rt::parser::Parser) { while !p.at_end() { if matches!(p.current(), A | LP) { parse_x(p); } else { p.error_until(&[A, LP]); } } }"), "{out}");
        assert!(out.contains("match p.current() { A => { p.bump(); } LP => { parse_list(p); } _ => { p.error_one(); } }"), "{out}");
        assert!(out.contains("pub fn parse_list(p: &mut rt::parser::Parser) { p.group(LIST, group_list); }"), "{out}");
        assert!(out.contains("if !p.separator(COMMA) { break; }"), "{out}");
    }

    #[test]
    fn a_required_item_recovers_with_its_follow_set() {
        // `pair` is entered on `a b` from the loop, so it takes both unchecked.
        let out = emitted("top: pair*, pair: A B x, x: C,");
        assert!(out.contains("pub fn parse_pair(p: &mut rt::parser::Parser) { let marker = p.open(); p.bump();p.bump();if p.at(C) { parse_x(p); } else if p.at_end() || p.at(A) { p.missing(); } else { p.error_until(&[A]); } p.close(marker, PAIR); }"), "{out}");
        assert!(out.contains("if p.at(A) && p.nth(1) == B { parse_pair(p); }"), "{out}");
    }

    #[test]
    fn a_choice_commits_on_its_first_distinct_token() {
        // `def 1():` is the motivating case: `def` decides, so the rule is
        // entered and the bad name becomes an error inside it.
        let out = emitted("top: s*, s: f | h, f: A B C, h: D,");
        assert!(out.contains("A => { parse_f(p); }"), "{out}");
        assert!(out.contains("pub fn parse_f(p: &mut rt::parser::Parser) { let marker = p.open(); p.bump();p.expect(B);p.expect(C);"), "{out}");
    }

    #[test]
    fn a_shared_first_token_is_decided_by_the_second() {
        let out = emitted("top: arg*, arg: kw | B | A C*, kw: A \"=\" B,");
        // After `a`: `c` or the next `arg` continue the expression, `=` makes a keyword.
        assert!(out.contains("A => { match p.nth(1) { A | B | C => { p.bump();while p.at(C) { p.bump(); } } EQ => { parse_kw(p); } _ => { p.bump();while p.at(C) { p.bump(); } } } }"), "{out}");
        assert!(out.contains("pub fn parse_kw(p: &mut rt::parser::Parser) { let marker = p.open(); p.bump();p.bump();"), "kw is called after `a =`: {out}");
        let error = try_emit("top: x*, x: A B C D | A B C B,").unwrap_err();
        assert!(error.contains("within 3 tokens"), "{error}");
    }

    #[test]
    fn a_predicate_tries_its_alternative_first() {
        let out = emitted("top: s*, s: &(A B) m | A C, m: A B D,");
        assert!(out.contains("if p.at(A) && predicate_0(p) { parse_m(p); } else { match p.current() { A => { p.bump();p.expect(C); } _ => { p.error_one(); } } }"), "{out}");
        assert!(out.contains("fn predicate_0(p: &mut rt::parser::Parser) -> bool { let checkpoint = p.checkpoint(); let errors = p.errors(); if p.at(A) { p.bump();p.expect(B); }"), "{out}");
        assert!(try_emit("top: s*, s: A &(B) C,").unwrap_err().contains("only start an alternative"));
    }

    #[test]
    fn a_precedence_table_is_a_pratt_loop() {
        let out = emitted("top: _e*, _e: precedence _atom { sum: left \"+\" | \"-\" => binary, right \"*\" => power, prefix \"-\" => negative, postfix args => call, cmp: left \"not\" \"in\" => test, }, _atom: A | B, args: \"(\" (_e@sum % \",\") \")\",");
        assert!(out.contains("pub fn parse_e(p: &mut rt::parser::Parser) { parse_e_at(p, 0); } pub fn parse_e_at(p: &mut rt::parser::Parser, min: u32) { let start = p.event_count();"), "{out}");
        assert!(out.contains("MINUS => { let marker = p.open(); p.bump(); if matches!(p.current(), A | B | MINUS) { p.nested(|p| parse_e_at(p, 3)); } else { p.missing(); } p.close(marker, NEGATIVE); }"), "{out}");
        assert!(out.contains("PLUS => { if 1 < min { break; } let marker = p.wrap(start); p.bump(); if matches!(p.current(), A | B | MINUS) { p.nested(|p| parse_e_at(p, 2)); } else { p.missing(); } p.close(marker, BINARY); }"), "left: {out}");
        assert!(out.contains("STAR => { if 2 < min { break; } let marker = p.wrap(start); p.bump(); if matches!(p.current(), A | B | MINUS) { p.nested(|p| parse_e_at(p, 2)); }"), "right: {out}");
        assert!(out.contains("LP => { if 4 < min { break; } let marker = p.wrap(start); parse_args(p);  p.close(marker, CALL); }"), "postfix: {out}");
        assert!(out.contains("NOT => { if p.nth(1) == IN { if 5 < min { break; }"), "a two-token operator needs both: {out}");
        assert!(out.contains("_ => { break; }"), "{out}");
        assert!(out.contains("parse_e_at(p, 1)"), "`@sum` enters at level 1: {out}");
    }

    #[test]
    fn layout_and_trailing_separators_are_emitted() {
        let src = r#"G { lex = lex; runtime = rt; layout = NL IN DE;
               tokens { A = "a" reset, B = "b", COMMA = ",", NL = "newline", IN = "indent" open block, DE = "dedent" close block, }
               top: _s*, _s: block | A (B %? ",") NL, block: IN _s* DE, }"#;
        let out = emit(&crate::check::check(crate::read::grammar(toks(src)).unwrap()).unwrap()).unwrap();
        assert!(out.contains("parse_top).with_layout(&rt::syntax::Layout { newline: NL, indent: IN, dedent: DE, resets: &[A] });"), "{out}");
        assert!(out.contains("if p.at(B) { p.bump(); while p.at(COMMA) { p.bump(); if !(p.at(B)) { break; } p.bump(); } }"), "{out}");
    }

    #[test]
    fn recursion_outside_a_group_is_nested() {
        let out = emitted("top: x*, x: A x | B,");
        assert!(out.contains("A => { p.bump();if matches!(p.current(), A | B) { p.nested(parse_x); }"), "{out}");
    }
}
