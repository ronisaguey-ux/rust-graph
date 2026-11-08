//! Python layout: physical lines become logical lines.
//!
//! Kept out of the main state machine so the rules — indentation stack,
//! suppressed breaks inside brackets and after a backslash, blank and
//! comment-only lines — can be read on their own.

use crate::cursor::{is_ascii_space, is_line_terminator};
use crate::python;
use crate::{DeepLexer, Kind, Language, Mode, Span, StateId, Token};

impl<'a, S: crate::ExternalScanner> DeepLexer<'a, S> {
    pub(crate) fn apply_state_mode(&mut self, state: StateId) {
        if self.state_modes.is_empty() {
            return;
        }
        match self.state_modes.get(state as usize).copied() {
            Some(m) => {
                if self.hint_on {
                    self.modes.set_top(m);
                } else if m != Mode::Default && self.modes.push(m, 0) {
                    self.hint_on = true;
                }
            }
            None => {
                if self.hint_on {
                    self.modes.pop();
                    self.hint_on = false;
                }
            }
        }
    }

    /// Map a kind through the symbol table and update the context that later
    /// tokens depend on. No allocation.
    pub(crate) fn skip_trivia(&mut self) -> Option<Token> {
        loop {
            match self.cur.peek() {
                None => return None,
                Some(b) if is_ascii_space(b) => {
                    self.cur.skip_while(is_ascii_space);
                }
                Some(b) if is_line_terminator(b) => {
                    let at = self.cur.pos();
                    let layout = self.layout_on();
                    python::eat_terminator(&mut self.cur);
                    self.line_has_token = false;
                    if layout {
                        // Only a token ends a logical line, so only then does
                        // line-start bookkeeping begin. A suppressed break
                        // (inside brackets, non-layout language) is plain
                        // whitespace and must not arm `begin_line`.
                        self.line_start = true;
                        return Some(self.layout_token_at(Kind::Newline, at));
                    }
                }
                Some(b'\\') => {
                    // A backslash immediately before a line break joins the
                    // lines: consume both and no layout is emitted.
                    let mut probe = self.cur;
                    probe.bump();
                    if python::terminator_len(&probe) > 0 {
                        self.cur.bump();
                        python::eat_terminator(&mut self.cur);
                        self.line_start = false;
                    } else {
                        return None; // a stray backslash is an operator
                    }
                }
                Some(b'#') if self.lang == Language::Python => {
                    let at = self.cur.pos();
                    python::eat_line_comment(&mut self.cur);
                    if self.comments_as_tokens {
                        return Some(self.emit(Kind::LineComment, Span::new(at, self.cur.pos())));
                    }
                }
                Some(b'/') if self.cur.starts_with(b"//") && self.lang != Language::Python => {
                    let at = self.cur.pos();
                    self.cur.skip_while(|b| !is_line_terminator(b));
                    if self.comments_as_tokens {
                        return Some(self.emit(Kind::LineComment, Span::new(at, self.cur.pos())));
                    }
                }
                Some(b'/') if self.cur.starts_with(b"/*") => {
                    let at = self.cur.pos();
                    self.cur.advance(2);
                    loop {
                        match self.cur.peek() {
                            None => break,
                            Some(b'*') if self.cur.peek_n(1) == Some(b'/') => {
                                self.cur.advance(2);
                                break;
                            }
                            Some(_) => {
                                self.cur.bump();
                            }
                        }
                    }
                    if self.comments_as_tokens {
                        return Some(self.emit(Kind::BlockComment, Span::new(at, self.cur.pos())));
                    }
                }
                Some(_) => return None,
            }
        }
    }

    // ---- Python layout ----------------------------------------------------

    /// At the start of a physical line: skip blank and comment-only lines,
    /// measure indentation and queue the INDENT/DEDENT tokens it implies.
    pub(crate) fn begin_line(&mut self) -> Option<Token> {
        self.line_start = false;
        if !self.lang.has_layout() || self.modes.top() != Mode::Default || self.suppress > 0 {
            return None;
        }
        let first = std::mem::replace(&mut self.first_line, false);

        let mut indent;
        loop {
            indent = python::measure_indent(&mut self.cur, self.tab_width);
            match self.cur.peek() {
                None => return None,
                Some(b) if is_line_terminator(b) => {
                    python::eat_terminator(&mut self.cur);
                    continue;
                }
                Some(b'#') => {
                    python::eat_line_comment(&mut self.cur);
                    python::eat_terminator(&mut self.cur);
                    continue;
                }
                Some(_) => break,
            }
        }

        // The first line's indentation is the base, not an INDENT.
        if first {
            return None;
        }

        self.layout_pos = self.cur.pos();
        let top = self.indents.top();
        if indent > top {
            if self.indents.push(indent) {
                self.pending_indent = true;
            }
        } else if indent < top {
            let mut n = 0u32;
            while self.indents.depth() > 1 && self.indents.top() > indent {
                self.indents.pop();
                n += 1;
            }
            self.pending_dedents = n;
        }

        if self.pending_dedents > 0 {
            self.pending_dedents -= 1;
            return Some(self.layout_token_at(Kind::Dedent, self.layout_pos));
        }
        if self.pending_indent {
            self.pending_indent = false;
            return Some(self.layout_token_at(Kind::Indent, self.layout_pos));
        }
        None
    }

    /// The synthetic line end and the closing DEDENTs an otherwise unfinished
    /// file still owes.
    pub(crate) fn eof_layout(&mut self) -> Option<Token> {
        if !self.lang.has_layout() || self.modes.top() != Mode::Default {
            return None;
        }
        let at = self.cur.pos();
        if self.line_has_token && !self.eof_newline_done {
            self.eof_newline_done = true;
            self.line_has_token = false;
            return Some(self.layout_token_at(Kind::Newline, at));
        }
        if self.indents.depth() > 1 && !self.eof_dedent_done {
            self.eof_dedent_done = true;
            self.layout_pos = at;
            self.pending_dedents = self.indents.pop_to_base();
        }
        None
    }
}
