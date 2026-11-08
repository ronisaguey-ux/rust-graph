//! The mode stack and the indentation stack. Both are fixed arrays: pushing a
//! mode or an indent level never allocates, and an overflow is reported as
//! `false` (the lexer then keeps its current state) instead of panicking.

/// What the lexer is currently lexing. The parser's `StateId` may force one of
/// these through a table; the lexer also pushes and pops them from its own
/// rules (template literals, Python layout).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Ordinary code: operators, identifiers, numbers, comments.
    Default = 0,
    /// Inside a quoted string literal. Escapes and triple quotes apply.
    String = 1,
    /// The parser has asked for a regular-expression literal, not division.
    Regex = 2,
    /// Inside a JavaScript template literal: only text until `${` or a backtick.
    Template = 3,
    /// Inside the `${ ... }` of a template literal; braces nest.
    TemplateExpr = 4,
    /// The parser has asked for a raw string (C `R"(...)"` or equivalent).
    RawString = 5,
}

impl Mode {
    pub const COUNT: usize = 6;

    pub fn name(self) -> &'static str {
        match self {
            Mode::Default => "default",
            Mode::String => "string",
            Mode::Regex => "regex",
            Mode::Template => "template",
            Mode::TemplateExpr => "template-expr",
            Mode::RawString => "raw-string",
        }
    }
}

/// Deepest mode nesting we keep. Template literals nested 64 deep are already
/// absurd; beyond this the lexer declines the push and stays put.
pub const MODE_CAP: usize = 64;

pub struct ModeStack {
    modes: [Mode; MODE_CAP],
    /// Per-frame scratch: template brace depth for `TemplateExpr`, and whether a
    /// `Template` frame follows a substitution. Free because it is inline.
    depths: [u32; MODE_CAP],
    len: usize,
}

impl Default for ModeStack {
    fn default() -> Self {
        ModeStack::new()
    }
}

impl ModeStack {
    pub fn new() -> ModeStack {
        ModeStack {
            modes: [Mode::Default; MODE_CAP],
            depths: [0; MODE_CAP],
            len: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn depth(&self) -> usize {
        self.len
    }

    pub fn top(&self) -> Mode {
        self.modes.get(self.len - 1).copied().unwrap_or(Mode::Default)
    }

    pub fn at(&self, i: usize) -> Mode {
        self.modes.get(i).copied().unwrap_or(Mode::Default)
    }

    pub fn is_default(&self) -> bool {
        self.len == 1 && self.top() == Mode::Default
    }

    /// Push and return whether it fit. `depth` is the frame's scratch value.
    pub fn push(&mut self, mode: Mode, depth: u32) -> bool {
        if self.len >= MODE_CAP {
            return false;
        }
        self.modes[self.len] = mode;
        self.depths[self.len] = depth;
        self.len += 1;
        true
    }

    /// Pop, but never the bottom frame.
    pub fn pop(&mut self) -> bool {
        if self.len <= 1 {
            return false;
        }
        self.len -= 1;
        true
    }

    pub fn set_top(&mut self, mode: Mode) {
        if let Some(slot) = self.modes.get_mut(self.len.saturating_sub(1)) {
            *slot = mode;
        }
    }

    pub fn top_depth(&self) -> u32 {
        self.depths.get(self.len - 1).copied().unwrap_or(0)
    }

    pub fn set_top_depth(&mut self, depth: u32) {
        if let Some(slot) = self.depths.get_mut(self.len.saturating_sub(1)) {
            *slot = depth;
        }
    }

    /// Pop frames above (and including) `i`, stopping at the bottom.
    pub fn truncate(&mut self, len: usize) {
        self.len = len.max(1).min(self.len);
    }
}

/// Indentation widths in columns, bottom frame always 0.
pub const INDENT_CAP: usize = 64;

pub struct IndentStack {
    cols: [u32; INDENT_CAP],
    len: usize,
}

impl Default for IndentStack {
    fn default() -> Self {
        IndentStack::new()
    }
}

impl IndentStack {
    pub fn new() -> IndentStack {
        IndentStack {
            cols: [0; INDENT_CAP],
            len: 1,
        }
    }

    pub fn depth(&self) -> usize {
        self.len
    }

    pub fn top(&self) -> u32 {
        self.cols.get(self.len - 1).copied().unwrap_or(0)
    }

    pub fn push(&mut self, col: u32) -> bool {
        if self.len >= INDENT_CAP {
            return false;
        }
        self.cols[self.len] = col;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> bool {
        if self.len <= 1 {
            return false;
        }
        self.len -= 1;
        true
    }

    /// Pop every frame above the base, returning how many were popped.
    pub fn pop_to_base(&mut self) -> u32 {
        let n = (self.len - 1) as u32;
        self.len = 1;
        n
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.cols[..self.len]
    }
}
