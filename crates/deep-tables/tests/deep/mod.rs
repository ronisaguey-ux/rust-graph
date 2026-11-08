//! The old parser runtime and the hand-written JSON lexer, included by path
//! exactly as `deep-grammars` includes them: the equivalence test compares the
//! two code paths over one lexer and one runtime, so neither may be a copy.

#[path = "../../../../src/deep/budget.rs"]
pub mod budget;
#[path = "../../../../src/deep/lexer.rs"]
pub mod lexer;
#[path = "../../../../src/deep/pairs.rs"]
pub mod pairs;
#[path = "../../../../src/deep/parser.rs"]
pub mod parser;
#[path = "../../../../src/deep/syntax.rs"]
pub mod syntax;
#[path = "../../../../src/deep/tree.rs"]
pub mod tree;
/// The hand-written JSON lexer and kind table.
#[path = "../../../../src/deep/json.rs"]
pub mod json;

/// `json::lex` is `pub(super)`; this is the name the macro is given.
pub fn lex(cursor: &mut lexer::Cursor) -> syntax::Kind {
    json::lex(cursor)
}
