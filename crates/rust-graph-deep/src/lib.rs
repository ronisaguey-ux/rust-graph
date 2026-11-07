//! `grammar!` for the deep scan, on the std `proc_macro` API alone.
//!
//! Slice 0 reads the shape of a grammar and lists its rules. Rule bodies are
//! kept for Slice 2, which turns them into parser functions.

use proc_macro::{Delimiter, Spacing, TokenStream, TokenTree};

/// `grammar! { Name { rule: body, ... } }` defines `struct Name` with
/// `Name::NAME` and `Name::RULES`, the rules in source order.
#[proc_macro]
pub fn grammar(input: TokenStream) -> TokenStream {
    let code = match read(input) {
        Ok((name, rules)) => emit(&name, &rules),
        Err(message) => format!("compile_error!({message:?});"),
    };
    code.parse().expect("generated code is valid Rust")
}

fn read(input: TokenStream) -> Result<(String, Vec<String>), String> {
    let mut tokens = input.into_iter();
    let name = match tokens.next() {
        Some(TokenTree::Ident(name)) => name.to_string(),
        _ => return Err("grammar!: expected a grammar name".into()),
    };
    let body = match tokens.next() {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => group.stream(),
        _ => return Err(format!("grammar!: expected `{{ rules }}` after `{name}`")),
    };
    if tokens.next().is_some() {
        return Err(format!("grammar!: unexpected tokens after the body of `{name}`"));
    }
    Ok((name, rules(body)?))
}

fn rules(body: TokenStream) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = Vec::new();
    let mut tokens = body.into_iter().peekable();
    while let Some(first) = tokens.next() {
        let TokenTree::Ident(rule) = first else {
            return Err(format!("grammar!: expected a rule name, found `{first}`"));
        };
        let rule = rule.to_string();
        if !matches!(tokens.next(), Some(TokenTree::Punct(colon)) if colon.as_char() == ':' && colon.spacing() == Spacing::Alone) {
            return Err(format!("grammar!: expected `:` after rule `{rule}`"));
        }
        if names.contains(&rule) {
            return Err(format!("grammar!: rule `{rule}` is defined twice"));
        }
        let mut empty = true;
        loop {
            match tokens.next() {
                None => break,
                Some(TokenTree::Punct(comma)) if comma.as_char() == ',' => break,
                Some(_) => empty = false,
            }
        }
        if empty {
            return Err(format!("grammar!: rule `{rule}` has an empty body"));
        }
        names.push(rule);
    }
    if names.is_empty() {
        return Err("grammar!: a grammar needs at least one rule".into());
    }
    Ok(names)
}

fn emit(name: &str, rules: &[String]) -> String {
    format!(
        "pub struct {name}; impl {name} {{ \
         pub const NAME: &'static str = {name:?}; \
         pub const RULES: &'static [&'static str] = &{rules:?}; }}"
    )
}
