//! Writes a checked grammar as Rust: kind constants, the kind table, the
//! `Language` static and one function per rule, all on the parser runtime.
//!
//! Every rule function assumes its caller checked the rule's [`Guard`], so it
//! takes its leading tokens without looking again. Recovery comes from the
//! grammar alone:
//!
//! - a required item that is absent becomes an empty `ERROR` flagged `MISSING`
//!   when the next token may follow it (or the group ends), and otherwise the
//!   tokens up to one that may follow it become one `ERROR`;
//! - the body of the root and of every group runs to the end: `x*` wraps what
//!   cannot start an `x` in an `ERROR` up to the next `x`, and `x % sep` makes an
//!   `ERROR` of a stray separator or of anything up to the next separator;
//! - an alternative that matches nothing takes one token as an `ERROR`.

use std::fmt::Write;

use crate::check::{is_hidden, rule_constant, Checked, Guard, Set, Shape, KINDS};
use crate::read::{Expr, Node, TokenClass};

pub fn emit(checked: &Checked) -> String {
    let mut emitter = Emitter { checked, rule: 0, rt: &checked.grammar.runtime };
    emitter.items()
}

struct Emitter<'c> {
    checked: &'c Checked,
    /// The rule being written, for [`Checked::nested`].
    rule: usize,
    rt: &'c str,
}

impl Emitter<'_> {
    fn items(&mut self) -> String {
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
        let _ = write!(out, "const {KINDS}: &[{rt}::syntax::KindInfo] = &[{kinds}];");
        let root = &grammar.rules[0].name;
        let _ = write!(
            out,
            "pub static {}: {rt}::syntax::Language = {rt}::syntax::Language::new({:?}, {}, {KINDS}, {}, {});",
            grammar.name,
            grammar.language,
            rule_constant(root),
            grammar.lex,
            parse_fn(root),
        );
        for rule in 0..grammar.rules.len() {
            self.rule = rule;
            out.push_str(&self.rule_fn(rule));
        }
        out
    }

    fn kind_row(&self, out: &mut String, kinds: &mut String, constant: &str, name: &str, class: &str, offset: usize) {
        let rt = self.rt;
        let _ = write!(out, "pub const {constant}: {rt}::syntax::Kind = {rt}::syntax::FIRST_LANGUAGE_KIND + {offset};");
        let _ = write!(kinds, "{rt}::syntax::KindInfo {{ name: {name:?}, class: {rt}::syntax::Class::{class} }},");
    }

    fn rule_fn(&self, rule: usize) -> String {
        let checked = self.checked;
        let definition = &checked.grammar.rules[rule];
        let name = &definition.name;
        let parser = format!("{}::parser::Parser", self.rt);
        let signature = format!("pub fn {}(p: &mut {parser})", parse_fn(name));
        match &checked.shapes[rule] {
            Shape::Root => format!("{signature} {{ {} }}", self.fill(&definition.body)),
            Shape::Group(inner) => {
                let body = match inner {
                    Some(inner) => format!("fn {}(p: &mut {parser}) {{ {} }}", group_fn(name), self.fill(inner)),
                    None => format!("fn {}(_: &mut {parser}) {{}}", group_fn(name)),
                };
                format!("{signature} {{ p.group({}, {}); }} {body}", rule_constant(name), group_fn(name))
            }
            Shape::Table => panic!("grammar!: precedence tables are emitted by the decision emitter, the next commit"),
            Shape::Node => format!(
                "{signature} {{ let marker = p.open(); {} p.close(marker, {}); }}",
                self.guarded(&definition.body, &checked.follow[rule]),
                rule_constant(name)
            ),
            Shape::Hidden => format!("{signature} {{ {} }}", self.guarded(&definition.body, &checked.follow[rule])),
        }
    }

    /// The body of the root or of a group, which runs to the end.
    fn fill(&self, node: &Node) -> String {
        let checked = self.checked;
        let empty = Set::new(checked.size());
        match &node.expr {
            Expr::Star(inner) => format!(
                "while !p.at_end() {{ if {} {{ {} }} else {{ p.error_until({}); }} }}",
                self.cond(&checked.guard(inner), 0),
                self.guarded(inner, &checked.repeat_after(inner, &empty)),
                self.list(&checked.first(inner)),
            ),
            Expr::Sep(inner, separator, _) => {
                let Expr::Token(token) = separator.expr else { unreachable!("checked: a separator is a token") };
                let separator_kind = self.constant(token);
                format!(
                    "while !p.at_end() {{ if {} {{ {} }} else if p.at({separator_kind}) {{ p.error_one(); continue; }} else {{ p.error_until(&[{separator_kind}]); }} if !p.separator({separator_kind}) {{ break; }} }}",
                    self.cond(&checked.guard(inner), 0),
                    self.guarded(inner, &checked.separated_after(separator, &empty)),
                )
            }
            _ => self.required(node, &empty),
        }
    }

    /// Parses `node`, whose guard the caller has checked. Nullable forms check
    /// for themselves. `after` is what may follow `node`.
    fn guarded(&self, node: &Node, after: &Set) -> String {
        let checked = self.checked;
        match &node.expr {
            Expr::Token(_) => "p.bump();".into(),
            Expr::Rule(rule) => self.call(*rule),
            Expr::Seq(items) => {
                let leading = checked.leading_tokens(items).len();
                let covered = if leading >= 2 { leading } else { usize::from(leading == 1 || !checked.nullable(&items[0])) };
                let mut out = String::new();
                for at in 0..items.len() {
                    let rest = checked.first_of_rest(&items[at + 1..], after);
                    out.push_str(&if at < covered { self.guarded(&items[at], &rest) } else { self.required(&items[at], &rest) });
                }
                out
            }
            Expr::Choice(items) => self.choice(items, after),
            Expr::Opt(inner) => format!("if {} {{ {} }}", self.cond(&checked.guard(inner), 0), self.guarded(inner, after)),
            Expr::Star(inner) => self.repeat(inner, after),
            Expr::Plus(inner) => format!("{}{}", self.guarded(inner, &checked.repeat_after(inner, after)), self.repeat(inner, after)),
            Expr::Sep(inner, separator, _) => {
                let Expr::Token(token) = separator.expr else { unreachable!("checked: a separator is a token") };
                let guard = checked.guard(inner);
                let item = self.guarded(inner, &checked.separated_after(separator, after));
                format!(
                    "if {} {{ {item} while p.at({}) && {} {{ p.bump(); {item} }} }}",
                    self.cond(&guard, 0),
                    self.constant(token),
                    self.cond(&guard, 1)
                )
            }
            Expr::Literal(_) | Expr::Name(_) | Expr::Level(..) | Expr::Table(_) => unreachable!("resolved"),
            Expr::Pred(_) | Expr::RuleAt(..) => panic!("grammar!: predicates and precedence levels are emitted by the decision emitter, the next commit"),
        }
    }

    fn repeat(&self, inner: &Node, after: &Set) -> String {
        let checked = self.checked;
        format!("while {} {{ {} }}", self.cond(&checked.guard(inner), 0), self.guarded(inner, &checked.repeat_after(inner, after)))
    }

    /// Parses `node`, which must be here; see the module comment for recovery.
    fn required(&self, node: &Node, after: &Set) -> String {
        let checked = self.checked;
        if checked.nullable(node) {
            return self.guarded(node, after);
        }
        if let Expr::Token(token) = node.expr {
            return format!("p.expect({});", self.constant(token));
        }
        let mut stop = "p.at_end()".to_string();
        if !after.is_empty() {
            let _ = write!(stop, " || {}", self.cond(&Guard::Set(after.clone()), 0));
        }
        format!(
            "if {} {{ {} }} else if {stop} {{ p.missing(); }} else {{ p.error_until({}); }}",
            self.cond(&checked.guard(node), 0),
            self.guarded(node, after),
            self.list(after),
        )
    }

    /// One token decides; checked to be unambiguous. A `match` when every
    /// alternative is decided by its first token alone.
    fn choice(&self, items: &[Node], after: &Set) -> String {
        let checked = self.checked;
        let guards: Vec<Guard> = items.iter().map(|item| checked.guard(item)).collect();
        if guards.iter().all(|guard| matches!(guard, Guard::Set(_))) {
            let mut out = "match p.current() {".to_string();
            for (item, guard) in items.iter().zip(&guards) {
                let Guard::Set(set) = guard else { unreachable!("all sets") };
                let pattern: Vec<String> = set.iter().map(|token| self.constant(token)).collect();
                let _ = write!(out, " {} => {{ {} }}", pattern.join(" | "), self.guarded(item, after));
            }
            out.push_str(" _ => p.error_one(), }");
            return out;
        }
        let mut out = String::new();
        for (item, guard) in items.iter().zip(&guards) {
            let _ = write!(out, "if {} {{ {} }} else ", self.cond(guard, 0), self.guarded(item, after));
        }
        out.push_str("{ p.error_one(); }");
        out
    }

    fn call(&self, rule: usize) -> String {
        let name = parse_fn(&self.checked.grammar.rules[rule].name);
        if self.checked.nested[self.rule][rule] {
            format!("p.nested({name});")
        } else {
            format!("{name}(p);")
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
                    (_, 0) => format!("p.at_any(&[{}])", kinds.join(", ")),
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

    fn emitted(rules: &str) -> String {
        let src = format!(
            r#"G {{ lex = lex; runtime = rt;
               tokens {{ A = "a", B = "b", C = "c", COMMA = ",", LP = "(" open parens, RP = ")" close parens, }}
               {rules} }}"#
        );
        emit(&crate::check::check(crate::read::grammar(toks(&src)).unwrap()).unwrap())
    }

    #[test]
    fn the_kind_table_lists_tokens_then_named_rules() {
        let out = emitted("top: _x*, _x: A | list, list: \"(\" (B % \",\") \")\",");
        assert!(out.contains("pub const A: rt::syntax::Kind = rt::syntax::FIRST_LANGUAGE_KIND + 0;"), "{out}");
        assert!(out.contains("pub const TOP: rt::syntax::Kind = rt::syntax::FIRST_LANGUAGE_KIND + 6;"), "{out}");
        assert!(out.contains("pub const LIST: rt::syntax::Kind = rt::syntax::FIRST_LANGUAGE_KIND + 7;"), "{out}");
        assert!(!out.contains("pub const X"), "a hidden rule has no kind: {out}");
        assert!(out.contains(r#"rt::syntax::KindInfo { name: "(", class: rt::syntax::Class::Open(0) }"#), "{out}");
        assert!(out.contains(r#"pub static G: rt::syntax::Language = rt::syntax::Language::new("g", TOP, KINDS, lex, parse_top);"#), "{out}");
    }

    #[test]
    fn rules_become_runtime_calls() {
        let out = emitted("top: _x*, _x: A | list, list: \"(\" (B % \",\") \")\",");
        assert!(out.contains("pub fn parse_top(p: &mut rt::parser::Parser) { while !p.at_end() { if p.at_any(&[A, LP]) { parse_x(p); } else { p.error_until(&[A, LP]); } } }"), "{out}");
        assert!(out.contains("match p.current() { A => { p.bump(); } LP => { parse_list(p); } _ => p.error_one(), }"), "{out}");
        assert!(out.contains("pub fn parse_list(p: &mut rt::parser::Parser) { p.group(LIST, group_list); }"), "{out}");
        assert!(out.contains("if !p.separator(COMMA) { break; }"), "{out}");
    }

    #[test]
    fn a_required_item_recovers_with_its_follow_set() {
        let out = emitted("top: pair*, pair: A B x, x: C,");
        assert!(out.contains("pub fn parse_pair(p: &mut rt::parser::Parser) { let marker = p.open(); p.bump();p.bump();if p.at(C) { parse_x(p); } else if p.at_end() || p.at(A) { p.missing(); } else { p.error_until(&[A]); } p.close(marker, PAIR); }"), "{out}");
        assert!(out.contains("if p.at(A) && p.nth(1) == B { parse_pair(p); }"), "{out}");
    }

    #[test]
    fn recursion_outside_a_group_is_nested() {
        let out = emitted("top: x*, x: A x | B,");
        assert!(out.contains("A => { p.bump();if p.at_any(&[A, B]) { p.nested(parse_x); }"), "{out}");
    }
}
