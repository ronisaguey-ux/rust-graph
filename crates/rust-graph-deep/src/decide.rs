//! LL(k) decisions: which alternative to take, on the shortest run of leading
//! tokens that tells them apart, up to [`LOOKAHEAD`] tokens.
//!
//! The tokens an alternative can start with are found by taking derivatives of
//! the grammar: a configuration is a stack of what is left to match, and
//! consuming a token yields every configuration that can follow. An
//! alternative that may already be complete continues with the tokens that may
//! follow the whole choice, and after those with anything. No left recursion
//! (checked) means expanding a configuration always reaches a token.
//!
//! The result is a decision tree that the emitter writes as nested `match`es on
//! `p.current()`, `p.nth(1)`, ... A choice commits as soon as one alternative is
//! left, so `def 1():` is a function with an error inside, not an error.

use std::collections::BTreeMap;

use crate::check::{Checked, Set};
use crate::read::{Expr, Node};
use crate::{Error, Spot};

/// How many tokens a decision may look at.
pub const LOOKAHEAD: usize = 3;

/// Configurations explored per decision before giving up: a grammar this
/// ambiguous should be restructured, not waited for.
const BUDGET: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Take alternative `alt`. Its first `matched` tokens were checked on the way here.
    Take { alt: usize, matched: usize },
    /// Look at token `depth` (0 is the current one).
    Branch { depth: usize, arms: Vec<(Vec<usize>, Decision)>, default: Option<Box<Decision>> },
}

/// What is left to match, top last.
#[derive(Clone)]
enum Frame<'g> {
    Node(&'g Node),
    /// `x*` after an `x`: another, or done.
    Again(&'g Node),
    /// `x % sep` after an `x`: a separator and another, or done.
    SepNext(&'g Node, &'g Node, bool),
    /// `x % sep` after a separator: an `x`, or done if trailing is allowed.
    SepItem(&'g Node, &'g Node, bool),
    /// The end of the alternative: a token that may follow the choice.
    After,
    /// Past that: any token.
    Any,
}

type Config<'g> = Vec<Frame<'g>>;

struct Search<'g, 'a> {
    checked: &'g Checked,
    after: &'a Set,
    budget: usize,
}

impl<'g> Search<'g, '_> {
    /// Every way to consume one token from `stack`: pushes `(token, rest)` to
    /// `out`, and sets `ended` if the alternative may be complete here.
    fn expand(&mut self, mut stack: Config<'g>, out: &mut Vec<(usize, Config<'g>)>, ended: &mut bool) -> bool {
        if self.budget == 0 {
            return false;
        }
        self.budget -= 1;
        let Some(top) = stack.pop() else { return true };
        match top {
            Frame::Any => {
                *ended = true;
                for token in 0..self.checked.size() {
                    out.push((token, vec![Frame::Any]));
                }
                true
            }
            Frame::After => {
                *ended = true;
                for token in self.after.iter() {
                    out.push((token, vec![Frame::Any]));
                }
                true
            }
            Frame::Again(inner) => {
                if !self.expand(stack.clone(), out, ended) {
                    return false;
                }
                stack.push(Frame::Again(inner));
                stack.push(Frame::Node(inner));
                self.expand(stack, out, ended)
            }
            Frame::SepNext(inner, sep, trailing) => {
                if !self.expand(stack.clone(), out, ended) {
                    return false;
                }
                stack.push(Frame::SepItem(inner, sep, trailing));
                stack.push(Frame::Node(sep));
                self.expand(stack, out, ended)
            }
            Frame::SepItem(inner, sep, trailing) => {
                if trailing && !self.expand(stack.clone(), out, ended) {
                    return false;
                }
                stack.push(Frame::SepNext(inner, sep, trailing));
                stack.push(Frame::Node(inner));
                self.expand(stack, out, ended)
            }
            Frame::Node(node) => match &node.expr {
                Expr::Token(token) => {
                    out.push((*token, stack));
                    true
                }
                Expr::Rule(rule) | Expr::RuleAt(rule, _) => {
                    stack.push(Frame::Node(&self.checked.grammar.rules[*rule].body));
                    self.expand(stack, out, ended)
                }
                Expr::Seq(items) => {
                    stack.extend(items.iter().rev().map(Frame::Node));
                    self.expand(stack, out, ended)
                }
                Expr::Choice(items) => items.iter().all(|item| {
                    let mut branch = stack.clone();
                    branch.push(Frame::Node(item));
                    self.expand(branch, out, ended)
                }),
                Expr::Opt(inner) => {
                    if !self.expand(stack.clone(), out, ended) {
                        return false;
                    }
                    stack.push(Frame::Node(inner));
                    self.expand(stack, out, ended)
                }
                Expr::Star(inner) => {
                    stack.push(Frame::Again(inner));
                    self.expand(stack, out, ended)
                }
                Expr::Plus(inner) => {
                    stack.push(Frame::Again(inner));
                    stack.push(Frame::Node(inner));
                    self.expand(stack, out, ended)
                }
                Expr::Sep(inner, sep, trailing) => {
                    if !self.expand(stack.clone(), out, ended) {
                        return false;
                    }
                    stack.push(Frame::SepNext(inner, sep, *trailing));
                    stack.push(Frame::Node(inner));
                    self.expand(stack, out, ended)
                }
                Expr::Pred(_) => self.expand(stack, out, ended),
                Expr::Literal(_) | Expr::Name(_) | Expr::Level(..) | Expr::Table(_) => unreachable!("resolved"),
            },
        }
    }

    /// `candidates`: each alternative and its configurations after `path`.
    fn decide(&mut self, candidates: Vec<(usize, Vec<Config<'g>>)>, path: &mut Vec<usize>, default_last: bool) -> Result<Decision, Conflict> {
        let depth = path.len();
        let mut by_token: BTreeMap<usize, Vec<(usize, Vec<Config<'g>>)>> = BTreeMap::new();
        let mut ends = Vec::new();
        for (alt, configs) in &candidates {
            let mut next = Vec::new();
            let mut ended = false;
            for config in configs {
                if !self.expand(config.clone(), &mut next, &mut ended) {
                    return Err(Conflict::TooBig);
                }
            }
            if ended {
                ends.push(*alt);
            }
            for (token, rest) in next {
                let entry = by_token.entry(token).or_default();
                match entry.last_mut() {
                    Some((last, configs)) if last == alt => configs.push(rest),
                    _ => entry.push((*alt, vec![rest])),
                }
            }
        }
        let mut arms: Vec<(Vec<usize>, Decision)> = Vec::new();
        for (token, alternatives) in by_token {
            let decision = if alternatives.len() == 1 {
                Decision::Take { alt: alternatives[0].0, matched: depth + 1 }
            } else if depth + 1 >= LOOKAHEAD {
                path.push(token);
                return Err(Conflict::Ambiguous(path.clone(), alternatives.iter().map(|(alt, _)| *alt).collect()));
            } else {
                path.push(token);
                let decision = self.decide(alternatives, path, default_last)?;
                path.pop();
                decision
            };
            match arms.last_mut() {
                Some((tokens, last)) if *last == decision && matches!(decision, Decision::Take { .. }) => tokens.push(token),
                _ => arms.push((vec![token], decision)),
            }
        }
        let default = if depth == 0 {
            None
        } else if ends.len() == 1 {
            // The one alternative that may already be complete takes the rest.
            Some(Box::new(Decision::Take { alt: ends[0], matched: depth }))
        } else if default_last {
            Some(Box::new(Decision::Take { alt: candidates.last().expect("two or more").0, matched: depth }))
        } else {
            None
        };
        Ok(Decision::Branch { depth, arms, default })
    }
}

enum Conflict {
    TooBig,
    /// The tokens read, and the alternatives still possible after them.
    Ambiguous(Vec<usize>, Vec<usize>),
}

impl Checked {
    /// The decision tree for choosing among `alternatives`, followed by `after`.
    /// With `default_last`, a run of tokens that matches none of the remaining
    /// alternatives past the first token takes the last of them (or the one
    /// that may already be complete); without it, it takes none.
    pub fn decide(&self, alternatives: &[&Node], after: &Set, spot: Spot, default_last: bool) -> Result<Decision, Error> {
        let mut search = Search { checked: self, after, budget: BUDGET };
        let candidates = alternatives.iter().enumerate().map(|(alt, node)| (alt, vec![vec![Frame::After, Frame::Node(node)]])).collect();
        match search.decide(candidates, &mut Vec::new(), default_last) {
            Ok(decision) => Ok(decision),
            Err(Conflict::TooBig) => Err(Error::at(spot, "these alternatives are too ambiguous to decide: restructure them")),
            Err(Conflict::Ambiguous(path, alts)) => {
                let text: Vec<String> = path.iter().map(|token| format!("`{}`", self.grammar.tokens[*token].text)).collect();
                let spot = alts.get(1).map(|alt| alternatives[*alt].spot).unwrap_or(spot);
                Err(Error::at(
                    spot,
                    format!("alternatives cannot be told apart within {LOOKAHEAD} tokens: more than one can start {}; use a predicate `&(...)`", text.join(" ")),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::toks;

    fn checked(rules: &str) -> Checked {
        let src = format!(
            r#"G {{ lex = lex;
               tokens {{ A = "a", B = "b", C = "c", D = "d", E = "e", EQ = "=", }}
               {rules} }}"#
        );
        crate::check::check(crate::read::grammar(toks(&src)).unwrap()).unwrap()
    }

    fn decide(g: &Checked, rule: usize, after: &[usize]) -> Result<Decision, String> {
        let Expr::Choice(items) = &g.grammar.rules[rule].body.expr else { panic!("not a choice") };
        let mut set = Set::new(g.size());
        after.iter().for_each(|token| set.insert(*token));
        g.decide(&items.iter().collect::<Vec<_>>(), &set, None, true).map_err(|error| error.message)
    }

    #[test]
    fn one_token_decides_when_it_can() {
        let g = checked("top: x, x: A B | C,");
        let want = Decision::Branch {
            depth: 0,
            arms: vec![(vec![0], Decision::Take { alt: 0, matched: 1 }), (vec![2], Decision::Take { alt: 1, matched: 1 })],
            default: None,
        };
        assert_eq!(decide(&g, 1, &[]).unwrap(), want);
    }

    #[test]
    fn a_shared_first_token_is_decided_by_the_next() {
        // `a =` is a keyword argument; `a` then anything else an expression.
        let g = checked("top: x, x: A \"=\" B | A C* | D,");
        let Decision::Branch { arms, .. } = decide(&g, 1, &[]).unwrap() else { panic!() };
        let Decision::Branch { depth: 1, arms: inner, default } = &arms[0].1 else { panic!("{arms:?}") };
        assert_eq!(inner, &vec![(vec![2], Decision::Take { alt: 1, matched: 2 }), (vec![5], Decision::Take { alt: 0, matched: 2 })]);
        assert_eq!(default.as_deref(), Some(&Decision::Take { alt: 1, matched: 1 }), "the one that may be complete");
    }

    #[test]
    fn three_tokens_is_the_limit() {
        let g = checked("top: x, x: A B C D | A B C E,");
        let error = decide(&g, 1, &[]).unwrap_err();
        assert!(error.contains("within 3 tokens") && error.contains("`a` `b` `c`"), "{error}");
        let g = checked("top: x, x: A B D | A B E,");
        assert!(decide(&g, 1, &[]).is_ok());
    }

    #[test]
    fn what_follows_the_choice_counts() {
        // `a` alone, or `a b`: told apart by whether `b` follows, unless `b` may
        // follow the whole choice too.
        let g = checked("top: x, x: A | A B,");
        assert!(decide(&g, 1, &[2]).is_ok());
        assert!(decide(&g, 1, &[1]).unwrap_err().contains("`a` `b`"));
    }
}
