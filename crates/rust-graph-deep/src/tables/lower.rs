//! EBNF to BNF: every repetition, option and nested group becomes an anonymous
//! non-terminal, so the generator only ever sees a production list.
//!
//! The order productions are added in matters twice: the first rule added is the
//! generator's start symbol, and each non-terminal's symbol number follows
//! first-definition order. So a rule's own alternatives are inserted ahead of
//! the anonymous non-terminals its body asked for.

use crate::read::{Expr, Grammar, Node, TokenClass};
use crate::{Error, Spot};

/// The DSL as productions, with anonymous non-terminals for repetition.
pub struct Lowering<'g> {
    grammar: &'g Grammar,
    /// Terminal names, in the order the builder numbers them.
    pub terminals: Vec<String>,
    /// One entry per alternative, `(lhs, rhs, where it was written)`.
    rules: Vec<(String, Vec<String>, Spot)>,
    next_generated: usize,
}

impl<'g> Lowering<'g> {
    pub fn new(grammar: &'g Grammar) -> Result<Self, Error> {
        let mut terminals = Vec::new();
        for token in &grammar.tokens {
            if token.class == TokenClass::Trivia {
                continue;
            }
            if terminals.contains(&token.text) {
                return Err(Error::at(token.spot, format!("another token already has the text {:?}", token.text)));
            }
            if grammar.rules.iter().any(|rule| rule.name == token.text) {
                return Err(Error::at(
                    token.spot,
                    format!("the token text {:?} is also a rule name; a table numbers both in one namespace", token.text),
                ));
            }
            terminals.push(token.text.clone());
        }
        Ok(Lowering { grammar, terminals, rules: Vec::new(), next_generated: 0 })
    }

    /// Every rule's productions, the root's first.
    pub fn productions(&mut self) -> Result<Vec<(String, Vec<String>, Spot)>, Error> {
        for rule in 0..self.grammar.rules.len() {
            let name = self.grammar.rules[rule].name.clone();
            let spot = self.grammar.rules[rule].spot;
            // Lowering the body appends the anonymous non-terminals it needs;
            // this rule's own productions are inserted ahead of them so the
            // root stays the first rule the builder sees, which is what makes
            // it the start symbol.
            let at = self.rules.len();
            let mut alternatives = Vec::new();
            self.alternatives(&self.grammar.rules[rule].body, &mut alternatives)?;
            let mut inserted = 0;
            for alternative in alternatives {
                if self.rules.iter().any(|(lhs, rhs, _)| *lhs == name && *rhs == alternative) {
                    continue;
                }
                self.rules.insert(at + inserted, (name.clone(), alternative, spot));
                inserted += 1;
            }
        }
        Ok(std::mem::take(&mut self.rules))
    }

    fn alternatives(&mut self, node: &Node, out: &mut Vec<Vec<String>>) -> Result<(), Error> {
        match &node.expr {
            Expr::Choice(items) => {
                for item in items {
                    out.push(self.sequence(item)?);
                }
            }
            _ => out.push(self.sequence(node)?),
        }
        Ok(())
    }

    /// One alternative: the symbols of a sequence, without a top-level `|`.
    fn sequence(&mut self, node: &Node) -> Result<Vec<String>, Error> {
        match &node.expr {
            Expr::Seq(items) => {
                let mut out = Vec::new();
                for item in items {
                    out.push(self.item(item)?);
                }
                Ok(out)
            }
            _ => Ok(vec![self.item(node)?]),
        }
    }

    /// One symbol for `node`, making an anonymous non-terminal where the DSL
    /// allows something that is not a symbol.
    fn item(&mut self, node: &Node) -> Result<String, Error> {
        let spot = node.spot;
        match &node.expr {
            Expr::Literal(text) => self.token_by_text(text, spot),
            Expr::Name(name) => {
                if self.grammar.rules.iter().any(|rule| rule.name == *name) {
                    return Ok(name.clone());
                }
                if self.terminals.iter().any(|text| text == name) {
                    return Ok(name.clone());
                }
                match self.grammar.tokens.iter().find(|token| token.constant == *name) {
                    Some(token) => {
                        let text = token.text.clone();
                        self.token_by_text(&text, spot)
                    }
                    None => Err(Error::at(spot, format!("`{name}` is neither a rule nor a token"))),
                }
            }
            Expr::Token(token) => {
                let text = self.grammar.tokens[*token].text.clone();
                self.token_by_text(&text, spot)
            }
            Expr::Rule(rule) => Ok(self.grammar.rules[*rule].name.clone()),
            Expr::Opt(inner) => {
                let name = self.fresh("opt");
                let inner = self.sequence(inner)?;
                self.rules.push((name.clone(), Vec::new(), spot));
                self.rules.push((name.clone(), inner, spot));
                Ok(name)
            }
            Expr::Star(inner) => {
                let name = self.fresh("star");
                let inner = self.sequence(inner)?;
                self.rules.push((name.clone(), Vec::new(), spot));
                let mut repeated = vec![name.clone()];
                repeated.extend(inner);
                self.rules.push((name.clone(), repeated, spot));
                Ok(name)
            }
            Expr::Plus(inner) => {
                let name = self.fresh("plus");
                let inner = self.sequence(inner)?;
                self.rules.push((name.clone(), inner.clone(), spot));
                let mut repeated = vec![name.clone()];
                repeated.extend(inner);
                self.rules.push((name.clone(), repeated, spot));
                Ok(name)
            }
            Expr::Sep(inner, separator, trailing) => {
                let name = self.fresh("sep");
                let items = self.sequence(inner)?;
                // The DSL says a separator is one token; it may be written as
                // `","`, as `COMMA` or (after `check`) as a token number.
                let separator = match &separator.expr {
                    Expr::Literal(text) => self.token_by_text(text, separator.spot)?,
                    Expr::Token(token) => {
                        let text = self.grammar.tokens[*token].text.clone();
                        self.token_by_text(&text, separator.spot)?
                    }
                    Expr::Name(name) => match self.grammar.tokens.iter().find(|token| token.constant == *name || token.text == *name) {
                        Some(token) => {
                            let text = token.text.clone();
                            self.token_by_text(&text, separator.spot)?
                        }
                        None => return Err(Error::at(separator.spot, format!("`{name}` is not a token, and a separator is one token"))),
                    },
                    _ => return Err(Error::at(separator.spot, "a separator is one token")),
                };
                self.rules.push((name.clone(), Vec::new(), spot));
                self.rules.push((name.clone(), items.clone(), spot));
                let mut separated = vec![name.clone(), separator.clone()];
                separated.extend(items);
                self.rules.push((name.clone(), separated, spot));
                if *trailing {
                    self.rules.push((name.clone(), vec![name.clone(), separator], spot));
                }
                Ok(name)
            }
            Expr::Seq(_) | Expr::Choice(_) => {
                let name = self.fresh("group");
                let mut alternatives = Vec::new();
                self.alternatives(node, &mut alternatives)?;
                for alternative in alternatives {
                    self.rules.push((name.clone(), alternative, spot));
                }
                Ok(name)
            }
            Expr::Pred(_) | Expr::Table(_) | Expr::Level(..) | Expr::RuleAt(..) => {
                Err(super::unsupported_in(node).unwrap_or_else(|| Error::at(spot, "this cannot be a table grammar")))
            }
        }
    }

    /// A name no rule uses, for an anonymous non-terminal. The leading `_` is
    /// what tells the runtime (and `Tables::hidden`) that it makes no node.
    fn fresh(&mut self, family: &str) -> String {
        loop {
            self.next_generated += 1;
            let candidate = format!("_gen_{family}_{}", self.next_generated);
            if !self.grammar.rules.iter().any(|rule| rule.name == candidate) {
                return candidate;
            }
        }
    }

    fn token_by_text(&self, text: &str, spot: Spot) -> Result<String, Error> {
        match self.grammar.tokens.iter().find(|token| token.text == text) {
            Some(token) if token.class == TokenClass::Trivia => Err(Error::at(spot, format!("`{}` is trivia, which rules never see", token.constant))),
            Some(token) => Ok(token.text.clone()),
            None => Err(Error::at(spot, format!("no token has the text {text:?}: declare it in `tokens`"))),
        }
    }
}
