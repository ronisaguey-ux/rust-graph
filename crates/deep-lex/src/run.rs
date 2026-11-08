//! The `deep_abi::Lexer` implementation: apply the parser's mode hint, then
//! walk the pending layout tokens, the mode stack and the scanners until one
//! token can be returned.

use crate::{DeepLexer, ExternalScanner, Kind, Lexer, Mode, StateId, Token};

impl<'a, S: ExternalScanner> Lexer for DeepLexer<'a, S> {
    fn next_token(&mut self, state: StateId) -> Option<Token> {
        if self.done {
            return None;
        }
        self.apply_state_mode(state);
        loop {
            if self.pending_dedents > 0 {
                self.pending_dedents -= 1;
                return Some(self.layout_token_at(Kind::Dedent, self.layout_pos));
            }
            if self.pending_indent {
                self.pending_indent = false;
                return Some(self.layout_token_at(Kind::Indent, self.layout_pos));
            }
            if self.line_start {
                if let Some(token) = self.begin_line() {
                    return Some(token);
                }
            }
            if self.cur.eof() {
                if let Some(token) = self.eof_layout() {
                    return Some(token);
                }
                if self.pending_dedents > 0 {
                    continue;
                }
                self.done = true;
                return None;
            }
            if self.modes.top() == Mode::Template {
                let at = self.cur.pos();
                return Some(self.lex_template(at));
            }
            if let Some(token) = self.skip_trivia() {
                return Some(token);
            }
            if self.cur.eof() {
                continue;
            }
            let ctx = self.ctx();
            if self.scanner.start(&self.cur, ctx) {
                if let Some((kind, span)) = self.scanner.scan(&mut self.cur, ctx) {
                    return Some(self.emit(kind, span));
                }
            }
            return Some(self.lex_code());
        }
    }

    fn mode(&self) -> &'static str {
        self.modes.top().name()
    }
}

