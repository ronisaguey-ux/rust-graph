//! POSIX shell and bash: commands, pipelines, and/or lists, redirections,
//! assignments, functions, `if`, `for`, `while`/`until`, `case`/`esac`, and
//! every form of here-document.
//!
//! The lexer decides what is a reserved word from the token before it, because
//! shell reserved words are only reserved at the start of a command: `echo if`
//! is two words, `if x; then` is a clause. A word is a keyword when it follows
//! a command boundary (start of file, newline, `;`, `;;`, `&&`, `||`, `|`, `&`,
//! `(`, `{`, `then`, `do`, `else`, `elif`), which is where bash recognises one
//! too.
//!
//! The `)` that ends a `case` pattern is the reason the DSL lets a closing
//! delimiter stand as an ordinary terminal: `case $x in a) ... ;; esac` has a
//! closer with no opener, and `find_delimiter` in the macro no longer refuses
//! it (an *opening* delimiter must still open a whole rule, because only then
//! does the parser set up the group firewall).
//!
//! # Modes
//!
//! A here-document is the reason this grammar needs lexer modes. `<<EOF` is one
//! `heredoc_start` token and the line's newline is a `heredoc_begin` token:
//! the switch on it pushes `HEREDOC`, and the body is text, not commands, until
//! a line that is the delimiter. `<<-EOF` matches its delimiter after leading
//! tabs, `<<'EOF'` and `<<"EOF"` do not expand, and several here-documents on
//! one line are a queue: each body ends with `heredoc_end_more` (the mode stays)
//! except the last, whose `heredoc_end` pops it.
//!
//! A here-document's body and a double-quoted string are not opaque: `$name`,
//! `${...}`, `` `cmd` `` and `$(cmd)` are tokens inside them, and `$( )` and
//! backticks push `CODE`, which is `lex_sh` itself. So a command substitution is
//! a `command_substitution` node wherever it appears — a line of a heredoc, a
//! piece of a string, a word of a command — and the call inside it is a
//! `simple_command` with its own span. The same applies to `<(cmd)` and
//! `>(cmd)` (process substitution), `${name:-$(cmd)}` and `$(( ... ))`.
//!
//! `[[ ... ]]` is `TEST_OPEN`/`TEST_CLOSE` with a `TEST` mode, so `<`, `>` and
//! `=` inside it are `test_operator` tokens rather than redirections.
//! `(( ... ))` and `$(( ... ))` are one `arithmetic` node over a balanced
//! `arithmetic_text`; the expression inside is not modelled (a `$(...)` there
//! stays text), which is the one gap this module still names.
//!
//! Still heuristics, and named rather than hidden:
//!
//! - **`in` is a heuristic.** It is only reserved after the `for`/`case` head,
//!   and the lexer can see one token back, so a word-shaped token before `in`
//!   makes it the keyword. `echo x in` therefore reads `in` as a keyword, which
//!   parses anyway because `_simple_item` accepts it.
//! - **A `case` clause without `;;`** is accepted only as the final clause; a
//!   missing `;;` in the middle makes the rest of the `case` one body.
//! - **`$(...)` inside `$(( ... ))`** is arithmetic text, not a node.

use std::cell::RefCell;

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { SHELL {
    name = "shell";
    lex = lex_sh;

    /// Every mode but `CODE` is a different reading of the same bytes: a
    /// here-document's text, a string's text, a test expression, arithmetic and
    /// a parameter expansion. `CODE` is `lex_sh` again, pushed by `$(`, a
    /// backtick and `<(`, so the mode stack is what nests them.
    modes {
        CODE = lex_sh,
        HEREDOC = lex_heredoc_body,
        STRING = lex_string_text,
        TEST = lex_test,
        ARITHMETIC = lex_arithmetic,
        PARAMETER = lex_parameter,
    }
    switch SUBST_OPEN => push CODE;
    switch SUBST_END => pop;
    switch BACKTICK_OPEN => push CODE;
    switch BACKTICK_END => pop;
    switch PROCESS_OPEN => push CODE;
    switch PROCESS_CLOSE => pop;
    switch HEREDOC_BEGIN => push HEREDOC;
    switch HEREDOC_END => pop;
    switch STRING_OPEN => push STRING;
    switch STRING_CLOSE => pop;
    switch TEST_OPEN => push TEST;
    switch TEST_CLOSE => pop;
    switch ARITHMETIC_OPEN => push ARITHMETIC;
    switch ARITHMETIC_CLOSE => pop;
    switch ARITHMETIC_BAD => pop;
    switch PARAMETER_OPEN => push PARAMETER;
    switch PARAMETER_CLOSE => pop;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        SEMI = ";",
        NEWLINE = "newline",
        PIPE = "|",
        OR = "||",
        AND = "&&",
        AMP = "&",
        BANG = "!",
        /// `>file`, `2>&1`, `<<<word`: one token, the whole redirection.
        REDIRECT = "redirect",
        /// `$name`, `$1`, `$@`: one token.
        VARIABLE = "variable",
        /// `'...'` or `$'...'`: no expansion inside, so one token.
        STRING = "string",
        /// A quoted word with no closing quote.
        BAD_STRING = "bad_string" bad,
        /// `name=` (with its value, when the value is plain) at the start of a
        /// word. `x=(a b)` is this token then an `array`.
        ASSIGNMENT = "assignment",
        WORD = "word",
        /// A word directly followed by `()` at the start of a command: a
        /// function's name, which the shell's grammar does not read as an
        /// argument.
        FUNCTION_NAME = "function_name",

        /// `<<EOF`, `<<-EOF`, `<<'EOF'`, `<<"EOF"`: the whole opener.
        HEREDOC_START = "heredoc_start",
        /// The newline that ends the line the here-documents were opened on;
        /// the switch on it enters `HEREDOC`.
        HEREDOC_BEGIN = "heredoc_begin",
        /// One line of a here-document's body.
        HEREDOC_TEXT = "heredoc_text",
        /// The delimiter line of a body when another body follows it: the mode
        /// stays in `HEREDOC`.
        HEREDOC_END_MORE = "heredoc_end_more",
        /// The delimiter line of the last body: the mode pops.
        HEREDOC_END = "heredoc_end",

        /// The `$(` of a command substitution: the switch pushes `CODE`.
        SUBST_OPEN = "$(",
        /// The `)` that closes a command substitution at its own bracket depth.
        SUBST_END = "subst_end",
        /// A backtick that opens a substitution.
        BACKTICK_OPEN = "backtick_open",
        /// A backtick that closes one.
        BACKTICK_END = "backtick_end",
        /// `<(`, `>(` of a process substitution.
        PROCESS_OPEN = "<(",
        PROCESS_CLOSE = "process_close",
        /// The `"` of a double-quoted string: the switch pushes `STRING`.
        STRING_OPEN = "string_open",
        STRING_CLOSE = "string_close",
        /// The bytes of a double-quoted string between its expansions.
        STRING_TEXT = "string_text",

        TEST_OPEN = "[[",
        TEST_CLOSE = "]]",
        TEST_OPERATOR = "test_operator",

        ARITHMETIC_OPEN = "((",
        ARITHMETIC_TEXT = "arithmetic_text",
        ARITHMETIC_CLOSE = "))",
        ARITHMETIC_BAD = "bad_arithmetic" bad,

        PARAMETER_OPEN = "${",
        PARAMETER_TEXT = "parameter_text",
        PARAMETER_CLOSE = "parameter_close",

        KW_IF = "if",
        KW_THEN = "then",
        KW_ELIF = "elif",
        KW_ELSE = "else",
        KW_FI = "fi",
        KW_FOR = "for",
        KW_SELECT = "select",
        KW_IN = "in",
        KW_WHILE = "while",
        KW_UNTIL = "until",
        KW_DO = "do",
        KW_DONE = "done",
        KW_FUNCTION = "function",
        KW_CASE = "case",
        KW_ESAC = "esac",
        /// `;;`, `;&` or `;;&`: the end of one `case` clause.
        DBL_SEMI = ";;",
    }

    program: _body,

    /// A command and the line's trailing separators. A here-document's bodies
    /// are after the newline that ends the line, so they are part of the
    /// command that opened them, and the separators after the last body are the
    /// command's too.
    _command: _and_or _terminator* [heredocs] _terminator*,
    _terminator: ";" | NEWLINE | "&",
    /// `cat <<EOF` then, after the line's newline, one body per opened
    /// here-document.
    heredocs: HEREDOC_BEGIN heredoc_body+,
    /// A body: leading line breaks, then commands, each with its own trailing
    /// separators.
    _body: _terminator* _command*,

    _compound:
        if_clause
        | for_clause
        | while_clause
        | until_clause
        | case_clause
        | function_definition
        | paren_function
        | brace_group
        | subshell
        | test_expression,

    if_clause: "if" _body "then" _body elif_clause* [else_clause] "fi",
    elif_clause: "elif" _body "then" _body,
    else_clause: "else" _body,

    /// `for x in a b`, `for x; do`, `select x in ...`, or the C-style head
    /// `for ((i = 0; i < n; i++))`.
    for_clause: ("for" | "select") _for_head "do" _body "done",
    _for_head: arithmetic _terminator* | _name ["in"] _word* _terminator*,
    while_clause: "while" _body "do" _body "done",
    until_clause: "until" _body "do" _body "done",

    /// `case WORD in PAT) ... ;; esac`. The `)` after a pattern closes nothing:
    /// it is an ordinary terminal, which is what a close token may be outside a
    /// group rule. A leading `(` is allowed (`case $x in (a|b) ...`), and the
    /// final clause may omit its `;;`.
    case_clause: "case" _word "in" _terminator* case_item* "esac",
    case_item: plain_case_item | parenthesized_case_item,
    plain_case_item: _case_pattern ")" case_tail [DBL_SEMI] _terminator*,
    parenthesized_case_item: parenthesized_pattern case_tail [DBL_SEMI] _terminator*,
    parenthesized_pattern: "(" _case_pattern ")",
    case_tail: _terminator* _command*,
    _case_pattern: case_word ("|" case_word)*,
    /// One pattern: a word may be pieces run together, `*"$x"*` and `a"$b"c`.
    case_word: (_word_piece)+,
    _word_piece: WORD | VARIABLE | ASSIGNMENT | string_literal | parameter_expansion | command_substitution | backtick_substitution,

    function_definition: "function" _name [empty_parens] _terminator* _compound,
    paren_function: FUNCTION_NAME empty_parens _terminator* _compound,
    empty_parens: "(" ")",

    brace_group: "{" _body "}",
    subshell: "(" _body ")",

    _and_or: _pipeline (("&&" | "||") _terminator* _pipeline)*,
    _pipeline: ["!"] _pipeline_head _pipe_next* _redirects?,
    _pipeline_head: simple_command | _compound,
    /// A `|` at the end of a line continues onto the next one.
    _pipe_next: "|" _terminator* _pipeline_head,
    /// `while ...; done < <(cmd)`: a compound command's redirections come after
    /// the keyword that ends it.
    _redirects: REDIRECT+,
    simple_command: _simple_item+,

    _simple_item:
        WORD
        | VARIABLE
        | REDIRECT
        | "in"
        | assignment_word
        | heredoc_redirect
        | string_literal
        | command_substitution
        | backtick_substitution
        | process_substitution
        | parameter_expansion
        | arithmetic,

    _name: WORD,
    _word: WORD | VARIABLE | ASSIGNMENT | string_literal | parameter_expansion | command_substitution | backtick_substitution,

    /// `<<EOF`: the body is the next `heredoc_body`, which the command owns.
    heredoc_redirect: HEREDOC_START,
    /// The text of one body, with the expansions that still happen inside it,
    /// and the line that is the delimiter.
    heredoc_body: (_heredoc_part)* (HEREDOC_END | HEREDOC_END_MORE),
    _heredoc_part: HEREDOC_TEXT | VARIABLE | command_substitution | backtick_substitution | parameter_expansion | arithmetic,

    /// `$(cmd)`: the mode stack hands the bytes back to `CODE`, so `cmd` is a
    /// `simple_command` and not text.
    command_substitution: SUBST_OPEN _body SUBST_END,
    backtick_substitution: BACKTICK_OPEN _body BACKTICK_END,
    process_substitution: PROCESS_OPEN _body PROCESS_CLOSE,

    /// `"a $(cmd) b"`: the quotes keep their tokens and the expansions between
    /// them are nodes.
    string_literal: STRING | double_quoted_string,
    double_quoted_string: STRING_OPEN (_string_part)* STRING_CLOSE,
    _string_part: STRING_TEXT | VARIABLE | command_substitution | backtick_substitution | parameter_expansion | arithmetic,

    /// `${name:-default}`, `${#arr[@]}`, `${x/old/new}`: text with the
    /// substitutions that can nest in it.
    parameter_expansion: PARAMETER_OPEN (_parameter_part)* PARAMETER_CLOSE,
    _parameter_part: PARAMETER_TEXT | VARIABLE | command_substitution | backtick_substitution | parameter_expansion | arithmetic,

    /// `$((1 + 2))` and `((i++))`: one node over the balanced expression.
    arithmetic: ARITHMETIC_OPEN [ARITHMETIC_TEXT] (ARITHMETIC_CLOSE | ARITHMETIC_BAD),

    /// `[[ -f "$f" && $x == y ]]`. The operators keep their own kind so `<`
    /// and `>` are not read as redirections.
    test_expression: TEST_OPEN (_test_item)* TEST_CLOSE,
    _test_item: WORD | VARIABLE | TEST_OPERATOR | "&&" | "||" | "|" | "!" | test_group | string_literal | command_substitution | backtick_substitution | parameter_expansion | arithmetic,
    test_group: "(" (_test_item)* ")",

    /// `arr=(one "two" $(three))`, and the plain `x=1` that is one token.
    assignment_word: ASSIGNMENT [array],
    /// `arr=(one "two" $(three))`, with a line break allowed after each item.
    array: "(" (_nl _array_item)* _nl ")",
    _nl: NEWLINE*,
    _array_item: WORD | VARIABLE | ASSIGNMENT | REDIRECT | string_literal | command_substitution | backtick_substitution | parameter_expansion | arithmetic,
} }

/// What one open frame is: a code context that a `)` or a backtick closes, or a
/// mode's own state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameKind {
    /// `$( ... )`: the `)` at depth 0 closes it.
    Substitution,
    /// `` ` ... ` ``: the closing backtick closes it.
    Backtick,
    /// `<( ... )` / `>( ... )`.
    Process,
    Heredoc,
    Test,
    Arithmetic,
    Parameter,
    /// A double-quoted string.
    String,
}

impl FrameKind {
    fn is_code(self) -> bool {
        matches!(self, FrameKind::Substitution | FrameKind::Backtick | FrameKind::Process)
    }
}

/// One frame of the lexer's own stack. The mode stack the macro maintains says
/// *which step function* runs; this says what the bytes mean to it: the depth a
/// `$(` closes at, or a heredoc's delimiter.
struct Frame {
    kind: FrameKind,
    /// Openers minus closers inside a code frame.
    depth: u32,
    /// A heredoc's delimiter line, and whether `<<-` allows leading tabs and
    /// whether the body expands (`<<EOF` does, `<<'EOF'` does not).
    delimiter: Vec<u8>,
    strip_tabs: bool,
    expand: bool,
}

impl Frame {
    fn new(kind: FrameKind) -> Self {
        Frame { kind, depth: 0, delimiter: Vec::new(), strip_tabs: false, expand: true }
    }
}

thread_local! {
    /// The open frames, innermost last. Cleared at byte 0 of every file, like
    /// the macro's mode stack, because a parse that stopped inside a heredoc
    /// must not lend its state to the next file.
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    /// Here-documents opened on the current line whose bodies have not started.
    /// One line can open several (`cat <<A <<B`), and the bodies come in order
    /// after the line's newline.
    static PENDING: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

fn with_frames<T>(action: impl FnOnce(&mut Vec<Frame>) -> T) -> T {
    FRAMES.with(|frames| action(&mut frames.borrow_mut()))
}

fn push_frame(kind: FrameKind) {
    with_frames(|frames| frames.push(Frame::new(kind)));
}

fn pop_frame() {
    with_frames(|frames| {
        frames.pop();
    });
}

fn top_kind() -> Option<FrameKind> {
    with_frames(|frames| frames.last().map(|frame| frame.kind))
}

fn top_depth() -> u32 {
    with_frames(|frames| frames.last().map_or(0, |frame| frame.depth))
}

/// An opener or closer inside a `$( ... )` counts only for that frame.
fn adjust_depth(change: i32) {
    with_frames(|frames| {
        if let Some(frame) = frames.last_mut() {
            if frame.kind.is_code() {
                frame.depth = if change < 0 { frame.depth.saturating_sub(1) } else { frame.depth + 1 };
            }
        }
    });
}

/// The tokens after which a word is in command position, and so a keyword.
fn command_start(previous: Kind) -> bool {
    matches!(
        previous,
        crate::deep::syntax::UNKNOWN
            | NEWLINE
            | SEMI
            | PIPE
            | OR
            | AND
            | AMP
            | LPAREN
            | RPAREN
            | LBRACE
            | KW_THEN
            | KW_DO
            | KW_ELSE
            | KW_ELIF
            | KW_FUNCTION
            | DBL_SEMI
            | HEREDOC_BEGIN
            | HEREDOC_END
            | HEREDOC_END_MORE
            | SUBST_OPEN
            | BACKTICK_OPEN
            | PROCESS_OPEN
    )
}

/// The tokens a `for`/`case` head can end with, after which `in` is reserved.
fn head_end(previous: Kind) -> bool {
    matches!(previous, WORD | VARIABLE | STRING | ASSIGNMENT | STRING_CLOSE | SUBST_END | BACKTICK_END | PARAMETER_CLOSE)
}

pub fn lex_sh(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, WHITESPACE};
    if c.pos() == 0 {
        // A new file starts outside every mode, and whatever a parse that
        // stopped inside a heredoc left behind is not this file's.
        with_frames(|frames| frames.clear());
        PENDING.with(|pending| pending.borrow_mut().clear());
    }
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\x0c'));
            WHITESPACE
        }
        b'\\' if c.peek_at(1) == b'\n' => {
            c.bump_n(2);
            WHITESPACE
        }
        b'\\' if c.peek_at(1) == b'\r' => {
            c.bump_n(2);
            c.eat(b'\n');
            WHITESPACE
        }
        b'\n' => {
            c.bump();
            if start_heredoc() {
                HEREDOC_BEGIN
            } else {
                NEWLINE
            }
        }
        b'#' => {
            lex::line_comment(c);
            COMMENT
        }
        b'\'' => single_quoted(c),
        b'"' => string_open(c, 1),
        b'`' => backtick(c),
        b'$' => dollar(c),
        b'(' => {
            if c.peek_at(1) == b'(' {
                open_arithmetic(c, 2, 2)
            } else {
                c.bump();
                adjust_depth(1);
                LPAREN
            }
        }
        b')' => match top_kind() {
            Some(FrameKind::Substitution) if top_depth() == 0 => {
                c.bump();
                pop_frame();
                SUBST_END
            }
            Some(FrameKind::Process) if top_depth() == 0 => {
                c.bump();
                pop_frame();
                PROCESS_CLOSE
            }
            _ => {
                c.bump();
                adjust_depth(-1);
                RPAREN
            }
        },
        b'{' if (command_start(c.previous()) || c.previous() == WORD) && matches!(c.peek_at(1), b' ' | b'\t' | b'\n' | b'\r') => {
            c.bump();
            LBRACE
        }
        b'}' if command_start(c.previous()) && matches!(c.peek_at(1), b' ' | b'\t' | b'\n' | b'\r' | b';') => {
            c.bump();
            RBRACE
        }
        b'[' if c.peek_at(1) == b'[' => {
            c.bump_n(2);
            push_frame(FrameKind::Test);
            TEST_OPEN
        }
        b'<' if c.peek_at(1) == b'(' => {
            c.bump_n(2);
            push_frame(FrameKind::Process);
            PROCESS_OPEN
        }
        b'>' if c.peek_at(1) == b'(' => {
            c.bump_n(2);
            push_frame(FrameKind::Process);
            PROCESS_OPEN
        }
        b'<' if c.peek_at(1) == b'<' => heredoc_or_herestring(c),
        b';' if c.peek_at(1) == b';' => {
            c.bump_n(2);
            c.eat(b'&');
            DBL_SEMI
        }
        b';' if c.peek_at(1) == b'&' => {
            c.bump_n(2);
            DBL_SEMI
        }
        b';' => {
            c.bump();
            SEMI
        }
        b'|' if c.peek_at(1) == b'|' => {
            c.bump_n(2);
            OR
        }
        b'|' => {
            c.bump();
            PIPE
        }
        b'&' if c.peek_at(1) == b'&' => {
            c.bump_n(2);
            AND
        }
        b'&' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            if c.peek() == b'>' {
                c.bump();
            }
            redirect_target(c);
            REDIRECT
        }
        b'&' => {
            c.bump();
            AMP
        }
        b'!' if !c.peek_at(1).is_ascii_alphanumeric() && c.peek_at(1) != b'=' => {
            c.bump();
            BANG
        }
        b'>' | b'<' => redirect(c),
        b'0'..=b'9'
            if matches!(c.peek_at(1), b'>' | b'<')
                || (c.peek_at(1).is_ascii_digit() && matches!(c.peek_at(2), b'>' | b'<')) =>
        {
            c.eat_while(|byte| byte.is_ascii_digit());
            c.bump();
            if c.peek() == b'>' && c.behind(1) == b'>' {
                c.bump();
            }
            if c.peek() == b'&' {
                c.bump();
            }
            redirect_target(c);
            REDIRECT
        }
        _ => word(c),
    }
}

/// `'...'`: no expansion and no escaping inside, and a line break is allowed,
/// which is what makes `awk '...'` and `sed '...'` across lines one token.
fn single_quoted(c: &mut Cursor) -> Kind {
    c.bump();
    while !c.at_end() {
        if c.peek() == b'\'' {
            c.bump();
            return STRING;
        }
        c.bump_char();
    }
    BAD_STRING
}

/// `"..."`, and the `$"..."` locale form: the opening quote is one token and
/// the switch enters `STRING`, so the pieces between the expansions are text.
fn string_open(c: &mut Cursor, width: usize) -> Kind {
    c.bump_n(width);
    push_frame(FrameKind::String);
    STRING_OPEN
}

/// `$name`, `$1`, `$?`, `$(`/`$((`, `${`.
fn dollar(c: &mut Cursor) -> Kind {
    match c.peek_at(1) {
        b'(' => {
            if c.peek_at(2) == b'(' {
                open_arithmetic(c, 3, 2)
            } else {
                c.bump_n(2);
                push_frame(FrameKind::Substitution);
                SUBST_OPEN
            }
        }
        b'{' => {
            c.bump_n(2);
            push_frame(FrameKind::Parameter);
            PARAMETER_OPEN
        }
        b'\'' => {
            c.bump();
            single_quoted(c)
        }
        b'"' => string_open(c, 2),
        _ => variable(c),
    }
}

/// A backtick: it closes the innermost backtick frame, or opens one.
fn backtick(c: &mut Cursor) -> Kind {
    c.bump();
    if top_kind() == Some(FrameKind::Backtick) && top_depth() == 0 {
        pop_frame();
        BACKTICK_END
    } else {
        push_frame(FrameKind::Backtick);
        BACKTICK_OPEN
    }
}

/// `$(( ... ))` and `(( ... ))`: the opener keeps its own token, and the text
/// is scanned to the `))` that balances it.
fn open_arithmetic(c: &mut Cursor, width: usize, depth: u32) -> Kind {
    c.bump_n(width);
    with_frames(|frames| {
        let mut frame = Frame::new(FrameKind::Arithmetic);
        frame.depth = depth;
        frames.push(frame);
    });
    ARITHMETIC_OPEN
}

/// `>file`, `>>file`, `>&2`, `2>&1`, `<<<word`: one token.
fn redirect(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    c.bump();
    if c.peek() == byte {
        c.bump();
    }
    if c.peek() == b'&' {
        c.bump();
    }
    redirect_target(c);
    REDIRECT
}

/// `<<EOF`, `<<-EOF`, `<<'EOF'`, `<<"EOF"`, and `<<<word` (a here-string).
fn heredoc_or_herestring(c: &mut Cursor) -> Kind {
    if c.peek_at(2) == b'<' {
        c.bump_n(3);
        redirect_target(c);
        return REDIRECT;
    }
    c.bump_n(2);
    let mut strip_tabs = false;
    if c.peek() == b'-' {
        c.bump();
        strip_tabs = true;
    }
    // `<< EOF` is legal: blanks may sit between the operator and the delimiter.
    c.eat_while(|byte| byte == b' ' || byte == b'\t');
    let mut expand = true;
    // A quoted delimiter keeps its body text: `<<'EOF'`, `<<"EOF"`, `<<\EOF`.
    // The token still spans the quotes; only the delimiter is unquoted.
    let delimiter: Vec<u8> = match c.peek() {
        b'\'' | b'"' => {
            expand = false;
            let quote = c.peek();
            c.bump();
            let inner = c.pos();
            let _ = lex::quoted(c, quote);
            let mut text = c.since(inner).to_vec();
            if text.last() == Some(&quote) {
                text.pop();
            }
            text
        }
        b'\\' => {
            expand = false;
            c.bump();
            let inner = c.pos();
            c.eat_while(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>'));
            c.since(inner).to_vec()
        }
        _ => {
            let inner = c.pos();
            c.eat_while(|byte| {
                !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>' | b'$' | b'`')
            });
            c.since(inner).to_vec()
        }
    };
    if delimiter.is_empty() {
        // No delimiter: not a here-document after all. The word scan took
        // nothing, so the caller's expectation is the only thing lost.
        return REDIRECT;
    }
    let mut frame = Frame::new(FrameKind::Heredoc);
    frame.delimiter = delimiter;
    frame.strip_tabs = strip_tabs;
    frame.expand = expand;
    PENDING.with(|pending| pending.borrow_mut().push(frame));
    HEREDOC_START
}

/// The line's newline started the first pending body: the frame joins the
/// lexer's stack and `HEREDOC` takes over.
fn start_heredoc() -> bool {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.is_empty() {
            return false;
        }
        let frame = pending.remove(0);
        with_frames(|frames| frames.push(frame));
        true
    })
}

/// The target of a redirection: a word, or an `&` and a file descriptor.
fn redirect_target(c: &mut Cursor) {
    if c.peek() == b'&' {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'-');
        return;
    }
    c.eat_while(|byte| {
        !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>')
    });
}

/// A `$` form: `$name`, `$1`, `$@`, `$?`.
fn variable(c: &mut Cursor) -> Kind {
    c.bump();
    c.eat_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@' | b'*' | b'#' | b'?' | b'$' | b'!' | b'-'));
    VARIABLE
}

/// Whether the `$` at the cursor starts something rather than standing alone.
fn dollar_starts(c: &Cursor) -> bool {
    let next = c.peek_at(1);
    next.is_ascii_alphanumeric()
        || matches!(next, b'_' | b'@' | b'*' | b'#' | b'?' | b'$' | b'!' | b'-' | b'(' | b'{')
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    // A backslash escapes the byte after it, so `\;`, `\"` and `foo\ bar` are
    // one word: `find -exec ... {} \;` and `printf "a\"b"` depend on it.
    loop {
        match c.peek() {
            b'\\' if !c.at_end() => c.bump_n(2),
            b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>' | b'\'' | b'"' | b'#' | b'$' | b'`' | 0 => break,
            _ => c.bump_char(),
        }
    }
    let length = start.len() - c.rest().len();
    let text = &start[..length];

    // `name=value` is one assignment word, and `name=(a b)` is one followed by
    // an array literal. `name+=value` assigns by appending.
    if let Some(eq) = text.iter().position(|&byte| byte == b'=') {
        let mut name = &text[..eq];
        if name.last() == Some(&b'+') {
            name = &name[..name.len() - 1];
        }
        let name_shaped = !name.is_empty()
            && name.iter().all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && !name[0].is_ascii_digit();
        if name_shaped {
            return ASSIGNMENT;
        }
    }

    if text.is_empty() {
        // A character the word scan stops on with nothing taken: one byte, so
        // the lexer always advances.
        c.bump_char();
        return WORD;
    }

    if command_start(c.previous()) && parens_ahead(c) {
        return FUNCTION_NAME;
    }

    if command_start(c.previous()) {
        match text {
            b"if" => return KW_IF,
            b"then" => return KW_THEN,
            b"elif" => return KW_ELIF,
            b"else" => return KW_ELSE,
            b"fi" => return KW_FI,
            b"for" => return KW_FOR,
            b"select" => return KW_SELECT,
            b"while" => return KW_WHILE,
            b"until" => return KW_UNTIL,
            b"do" => return KW_DO,
            b"done" => return KW_DONE,
            b"function" => return KW_FUNCTION,
            b"case" => return KW_CASE,
            b"esac" => return KW_ESAC,
            _ => {}
        }
    }
    // `in` after the loop or case word, which is all the context there is. The
    // subject may be a word, a variable or a quoted string (`case "$1" in`), so
    // the last token of a `for`/`case` head is any word-shaped one.
    if text == b"in" && head_end(c.previous()) {
        return KW_IN;
    }
    WORD
}

/// Whether the cursor sits at `(`, with only blanks between: `foo () {`.
fn parens_ahead(c: &Cursor) -> bool {
    let mut at = 0;
    while matches!(c.peek_at(at), b' ' | b'\t') {
        at += 1;
    }
    c.peek_at(at) == b'(' && c.peek_at(at + 1) == b')'
}

/// The `STRING` mode's step: text between expansions, one expansion, or the
/// closing quote.
pub fn lex_string_text(c: &mut Cursor) -> Kind {
    if top_kind() != Some(FrameKind::String) {
        // The mode stack and this stack disagree; lexing code cannot lose a
        // token, and the parser still gets a tree.
        return lex_sh(c);
    }
    if c.at_end() {
        pop_frame();
        return STRING_CLOSE;
    }
    match c.peek() {
        b'"' => {
            c.bump();
            pop_frame();
            STRING_CLOSE
        }
        b'`' => {
            c.bump();
            push_frame(FrameKind::Backtick);
            BACKTICK_OPEN
        }
        b'$' if c.peek_at(1) == b'(' && c.peek_at(2) == b'(' => open_arithmetic(c, 3, 2),
        b'$' if c.peek_at(1) == b'(' => {
            c.bump_n(2);
            push_frame(FrameKind::Substitution);
            SUBST_OPEN
        }
        b'$' if c.peek_at(1) == b'{' => {
            c.bump_n(2);
            push_frame(FrameKind::Parameter);
            PARAMETER_OPEN
        }
        b'$' if dollar_starts(c) => variable(c),
        _ => string_text(c),
    }
}

/// The bytes of a string between its expansions. A backslash escapes the next
/// byte, so `\"` does not close it.
fn string_text(c: &mut Cursor) -> Kind {
    let start = c.pos();
    while !c.at_end() {
        match c.peek() {
            b'"' => break,
            b'`' => break,
            b'\\' => c.bump_n(2),
            b'$' if dollar_starts(c) => break,
            _ => c.bump_char(),
        }
    }
    if c.pos() == start {
        // A `$` the checks above did not take: consume it so the mode advances.
        c.bump_char();
    }
    STRING_TEXT
}

/// The `HEREDOC` mode's step: one line of the body, one expansion, or the
/// delimiter line that ends it.
pub fn lex_heredoc_body(c: &mut Cursor) -> Kind {
    let Some(frame) = with_frames(|frames| frames.last().filter(|frame| frame.kind == FrameKind::Heredoc).map(|frame| (frame.delimiter.clone(), frame.strip_tabs, frame.expand))) else {
        return lex_sh(c);
    };
    let (delimiter, strip_tabs, expand) = frame;
    if c.at_end() {
        pop_frame();
        return HEREDOC_END;
    }
    if delimiter_line(c, &delimiter, strip_tabs) {
        consume_line(c);
        pop_frame();
        // Another body opened on the same line: its text follows immediately,
        // in the same mode.
        if next_body() {
            return HEREDOC_END_MORE;
        }
        return HEREDOC_END;
    }
    if expand {
        match c.peek() {
            b'`' => {
                c.bump();
                push_frame(FrameKind::Backtick);
                return BACKTICK_OPEN;
            }
            b'$' if c.peek_at(1) == b'(' && c.peek_at(2) == b'(' => return open_arithmetic(c, 3, 2),
            b'$' if c.peek_at(1) == b'(' => {
                c.bump_n(2);
                push_frame(FrameKind::Substitution);
                return SUBST_OPEN;
            }
            b'$' if c.peek_at(1) == b'{' => {
                c.bump_n(2);
                push_frame(FrameKind::Parameter);
                return PARAMETER_OPEN;
            }
            b'$' if dollar_starts(c) => {
                variable(c);
                return VARIABLE;
            }
            _ => {}
        }
    }
    // One line, up to the next expansion or the line end. Keeping the token on
    // the line is what lets the delimiter be recognised at a line start.
    let start = c.pos();
    while !c.at_end() {
        match c.peek() {
            b'\n' => {
                c.bump();
                break;
            }
            b'`' if expand => break,
            b'$' if expand && dollar_starts(c) => break,
            _ => c.bump_char(),
        }
    }
    if c.pos() == start {
        c.bump_char();
    }
    HEREDOC_TEXT
}

/// Whether the cursor is at a line that is the delimiter of this body.
fn delimiter_line(c: &Cursor, delimiter: &[u8], strip_tabs: bool) -> bool {
    if c.pos() != 0 && c.behind(1) != b'\n' {
        return false;
    }
    let mut at = 0;
    if strip_tabs {
        while c.peek_at(at) == b'\t' {
            at += 1;
        }
    }
    for &byte in delimiter {
        if c.peek_at(at) != byte {
            return false;
        }
        at += 1;
    }
    matches!(c.peek_at(at), b'\n' | b'\r' | 0)
}

/// The delimiter line, with its line break: the body's last token.
fn consume_line(c: &mut Cursor) {
    c.eat_while(|byte| byte != b'\n' && byte != b'\r');
    c.eat(b'\r');
    c.eat(b'\n');
}

/// Start the next queued body, if the line opened more than one.
fn next_body() -> bool {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending.is_empty() {
            return false;
        }
        let frame = pending.remove(0);
        with_frames(|frames| frames.push(frame));
        true
    })
}

/// The `PARAMETER` mode's step: the text of `${...}`, a nested expansion, or the
/// `}` that closes it.
pub fn lex_parameter(c: &mut Cursor) -> Kind {
    if top_kind() != Some(FrameKind::Parameter) {
        return lex_sh(c);
    }
    if c.at_end() {
        pop_frame();
        return PARAMETER_TEXT;
    }
    match c.peek() {
        b'}' => {
            c.bump();
            pop_frame();
            PARAMETER_CLOSE
        }
        b'`' => {
            c.bump();
            push_frame(FrameKind::Backtick);
            BACKTICK_OPEN
        }
        b'$' if c.peek_at(1) == b'(' && c.peek_at(2) == b'(' => open_arithmetic(c, 3, 2),
        b'$' if c.peek_at(1) == b'(' => {
            c.bump_n(2);
            push_frame(FrameKind::Substitution);
            SUBST_OPEN
        }
        b'$' if c.peek_at(1) == b'{' => {
            c.bump_n(2);
            push_frame(FrameKind::Parameter);
            PARAMETER_OPEN
        }
        b'$' if dollar_starts(c) => {
            variable(c);
            VARIABLE
        }
        _ => {
            let start = c.pos();
            while !c.at_end() {
                match c.peek() {
                    b'}' => break,
                    b'`' => break,
                    b'$' if dollar_starts(c) => break,
                    _ => c.bump_char(),
                }
            }
            if c.pos() == start {
                c.bump_char();
            }
            PARAMETER_TEXT
        }
    }
}

/// The `ARITHMETIC` mode's step: the balanced text, then the `))`.
pub fn lex_arithmetic(c: &mut Cursor) -> Kind {
    if top_kind() != Some(FrameKind::Arithmetic) {
        return lex_sh(c);
    }
    if c.at_end() {
        pop_frame();
        return ARITHMETIC_BAD;
    }
    if c.peek() == b')' && c.peek_at(1) == b')' {
        c.bump_n(2);
        pop_frame();
        return ARITHMETIC_CLOSE;
    }
    // The text, up to the `))` that balances the opener. A `(` opens one more
    // and a `)` closes one; the pair that would close the whole expression is
    // left for the next step, so one token always advances.
    let mut depth = top_depth();
    let start = c.pos();
    while !c.at_end() {
        if c.peek() == b')' && c.peek_at(1) == b')' && depth <= 2 {
            break;
        }
        match c.peek() {
            b'(' => {
                depth += 1;
                c.bump();
            }
            b')' => {
                if depth > 2 {
                    depth -= 1;
                }
                c.bump();
            }
            _ => c.bump_char(),
        }
    }
    if c.pos() == start {
        c.bump_char();
    }
    ARITHMETIC_TEXT
}

/// The `TEST` mode's step: `[[ ... ]]` with `<`, `>` and `=` as operators.
pub fn lex_test(c: &mut Cursor) -> Kind {
    if top_kind() != Some(FrameKind::Test) {
        return lex_sh(c);
    }
    if c.starts_with(b"]]") {
        c.bump_n(2);
        pop_frame();
        return TEST_CLOSE;
    }
    match c.peek() {
        b'<' | b'>' | b'=' => {
            c.bump();
            if c.peek() == b'=' {
                c.bump();
            }
            if c.peek() == b'~' {
                c.bump();
            }
            TEST_OPERATOR
        }
        b'!' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            if c.peek() == b'~' {
                c.bump();
            }
            TEST_OPERATOR
        }
        _ => lex_sh(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::deep::syntax::ERROR;
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    /// Every valid fixture and its top-level node kinds.
    const VALID: &[(&str, &[&str])] = &[
        (
            "shell_constructs.sh",
            &[
                "simple_command",
                "simple_command",
                "simple_command",
                "simple_command",
                "case_clause",
                "if_clause",
                "while_clause",
                "simple_command",
                "simple_command",
                "simple_command",
                "for_clause",
                "simple_command",
                "simple_command",
                "simple_command",
                "simple_command",
            ],
        ),
        (
            "shell_heredocs.sh",
            &[
                "simple_command",
                "simple_command",
                "heredocs",
                "if_clause",
                "simple_command",
                "heredocs",
                "simple_command",
                "heredocs",
                "simple_command",
                "heredocs",
                "simple_command",
                "heredocs",
                "function_definition",
                "for_clause",
                "simple_command",
                "heredocs",
            ],
        ),
        (
            "shell_pipeline.sh",
            &[
                "simple_command",
                "simple_command",
                "simple_command",
                "simple_command",
                "simple_command",
                "paren_function",
                "paren_function",
                "paren_function",
                "paren_function",
                "paren_function",
                "simple_command",
            ],
        ),
    ];

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
    }

    pub(super) fn parse(src: &[u8]) -> Workspace {
        let mut workspace = Workspace::default();
        workspace
            .parse(src, &SHELL, &Budget::default())
            .unwrap_or_else(|_| panic!("shell: input too large"));
        workspace
    }

    pub(super) fn errors(workspace: &Workspace) -> usize {
        let nodes = &workspace.tree.nodes;
        (0..nodes.len())
            .filter(|&index| nodes[index].kind == ERROR || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0)
            .count()
    }

    fn top(workspace: &Workspace) -> Vec<&'static str> {
        let nodes = &workspace.tree.nodes;
        workspace
            .tree
            .children(0)
            .map(|child| &nodes[child as usize])
            .filter(|node| node.flags & TOKEN == 0)
            .map(|node| SHELL.name(node.kind))
            .collect()
    }

    fn find<'w>(workspace: &'w Workspace, name: &str) -> Vec<&'w crate::deep::tree::Node> {
        workspace.tree.nodes.iter().filter(|node| SHELL.name(node.kind) == name).collect()
    }

    fn sexp(src: &[u8]) -> String {
        parse(src).tree.sexp(&SHELL, true)
    }

    #[test]
    fn fixtures_parse_cleanly() {
        for (name, expected) in VALID {
            let path = fixture(name);
            let src = fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            assert!(!src.is_empty(), "{name}: an empty fixture proves nothing");
            let workspace = parse(&src);
            assert_eq!(SHELL.name(workspace.tree.root().kind), "program", "{name}: the root's kind changed");
            assert_eq!(errors(&workspace), 0, "{name}: {} error nodes in a valid file:\n{}", errors(&workspace), workspace.tree.sexp(&SHELL, true));
            assert_eq!(&top(&workspace), expected, "{name}: top-level kinds:\n{}", workspace.tree.sexp(&SHELL, true));
        }
    }

    #[test]
    fn a_command_substitution_in_a_heredoc_is_a_node_with_its_span() {
        let src: &[u8] = b"#!/bin/sh\ncat <<EOF\nstart $(date +%s) end\n`whoami`\nEOF\n";
        let workspace = parse(src);
        assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&SHELL, true));

        let substitutions = find(&workspace, "command_substitution");
        assert_eq!(substitutions.len(), 1, "one $( ) in the body, one node");
        let node = substitutions[0];
        let text = &src[node.start as usize..node.end as usize];
        assert_eq!(text, b"$(date +%s)", "the substitution's span is exactly the substitution");

        // The call inside is its own command node, inside that span.
        let calls = find(&workspace, "simple_command");
        assert!(
            calls.iter().any(|call| call.start >= node.start
                && call.end <= node.end
                && &src[call.start as usize..call.end as usize] == b"date +%s"),
            "no simple_command for `date +%s` inside the substitution:\n{}",
            workspace.tree.sexp(&SHELL, true)
        );

        let ticks = find(&workspace, "backtick_substitution");
        assert_eq!(ticks.len(), 1, "a backtick substitution in a body is a node too");
        let tick = ticks[0];
        assert_eq!(&src[tick.start as usize..tick.end as usize], b"`whoami`");
    }

    #[test]
    fn a_heredoc_body_is_one_node_per_body() {
        let src: &[u8] = b"cat <<A <<B\nfirst\nA\nsecond\nB\necho done\n";
        let workspace = parse(src);
        assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&SHELL, true));
        let bodies = find(&workspace, "heredoc_body");
        assert_eq!(bodies.len(), 2, "two here-documents, two bodies:\n{}", workspace.tree.sexp(&SHELL, true));
        assert_eq!(&src[bodies[0].start as usize..bodies[0].end as usize], b"first\nA\n");
        assert_eq!(&src[bodies[1].start as usize..bodies[1].end as usize], b"second\nB\n");
    }

    #[test]
    fn a_stripped_heredoc_ends_at_an_indented_delimiter() {
        let src: &[u8] = b"if true; then\n\tcat <<-EOF\n\t\tindented body\n\tEOF\nfi\n";
        let workspace = parse(src);
        assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&SHELL, true));
        let bodies = find(&workspace, "heredoc_body");
        assert_eq!(bodies.len(), 1);
        assert_eq!(&src[bodies[0].start as usize..bodies[0].end as usize], b"\t\tindented body\n\tEOF\n");
    }

    #[test]
    fn a_quoted_heredoc_does_not_expand() {
        let src: &[u8] = b"cat <<'EOF'\n$HOME $(date)\nEOF\n";
        let workspace = parse(src);
        assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&SHELL, true));
        assert!(find(&workspace, "command_substitution").is_empty(), "a quoted delimiter keeps its body text");
        assert!(find(&workspace, "heredoc_text").len() == 1);
    }

    #[test]
    fn a_string_keeps_its_substitution_as_a_node() {
        let src: &[u8] = b"echo \"version $(git describe --tags) on ${HOST:-local}\"\n";
        let workspace = parse(src);
        assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&SHELL, true));
        let substitutions = find(&workspace, "command_substitution");
        assert_eq!(substitutions.len(), 1);
        let node = substitutions[0];
        assert_eq!(&src[node.start as usize..node.end as usize], b"$(git describe --tags)");
        let expansions = find(&workspace, "parameter_expansion");
        assert_eq!(expansions.len(), 1);
        assert_eq!(&src[expansions[0].start as usize..expansions[0].end as usize], b"${HOST:-local}");
    }

    /// Truncated and broken input must still produce a tree: never a panic and
    /// never a hang. The error lives inside the root.
    #[test]
    fn truncated_input_still_produces_a_tree() {
        let cases: &[&[u8]] = &[
            b"if true; then\n",
            b"cat <<EOF\nbody\n",
            b"cat <<EOF\n",
            b"cat <<-EOF\n\tbody\n",
            b"cat <<A <<B\nfirst\nA\n",
            b"case $x in\na) echo a;;\n",
            b"echo \"$(date\"\n",
            b"echo \"unterminated\n",
            b"echo ${x:-\n",
            b"echo ${x\n",
            b"echo $((1 + \n",
            b"echo `date\n",
            b"echo <(ls\n",
            b"[[ -f x\n",
            b"[[ -f\n",
            b"for ((i = 0; i < 3; i++))\n",
            b"function f {\n",
            b"while read x; do\n",
            b"echo )\n",
            b"case x in\n",
            b"#!/bin/sh\n(\n",
        ];
        for src in cases {
            let workspace = parse(src);
            assert_eq!(SHELL.name(workspace.tree.root().kind), "program", "{:?}: the root did not survive", String::from_utf8_lossy(src));
            assert_eq!(workspace.tree.root().end as usize, src.len(), "{:?}: the root did not span the file", String::from_utf8_lossy(src));
        }
    }

    #[test]
    fn every_heredoc_form_parses() {
        let src: &[u8] = concat!(
            "cat <<EOF\na\nEOF\n",
            "cat <<-EOF\n\tb\n\tEOF\n",
            "cat <<'EOF'\n$c\nEOF\n",
            "cat <<\"EOF\"\n$d\nEOF\n",
            "cat <<\\EOF\n$e\nEOF\n",
            "cat <<EOF | grep x\n$(f) EOF_ON_A_LINE\nEOF\n",
            "for x in a b; do\n\tcat <<EOF\n\t$((x + 1))\nEOF\ndone\n",
            "f() {\n\tcat <<EOF\n${g:-$(h)}\nEOF\n}\n",
        )
        .as_bytes();
        let workspace = parse(src);
        assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&SHELL, true));
        assert_eq!(find(&workspace, "heredoc_body").len(), 8);
    }

    #[test]
    fn a_case_array_and_arithmetic_are_nodes() {
        let src: &[u8] = concat!(
            "arr=(one \"two\" $(three))\n",
            "case $1 in\n  (a|b) echo ab ;;\n  c) echo c ;;\n  *) echo other\n",
            "esac\n",
            "[[ -f \"$f\" && $x =~ ^(a|b)$ ]]\n",
            "n=$((1 + 2 * (3 + 4)))\n",
            "for ((i = 0; i < 3; i++)); do echo $i; done\n",
            "diff <(sort a) <(sort b)\n",
        )
        .as_bytes();
        let workspace = parse(src);
        assert_eq!(errors(&workspace), 0, "{}", workspace.tree.sexp(&SHELL, true));
        assert_eq!(find(&workspace, "array").len(), 1);
        assert_eq!(find(&workspace, "case_item").len(), 3);
        assert_eq!(find(&workspace, "test_expression").len(), 1);
        assert_eq!(find(&workspace, "arithmetic").len(), 2);
        assert_eq!(find(&workspace, "process_substitution").len(), 2);
    }

    /// Prints the top-level kinds of every fixture, so the table above is
    /// written from what the grammar produced rather than guessed.
    #[test]
    #[ignore]
    fn dump_top_level_kinds() {
        for (name, _) in VALID {
            let src = fs::read(fixture(name)).unwrap();
            let workspace = parse(&src);
            println!(
                "{name}: complete={} errors={} top={}",
                workspace.tree.nodes.len() > 0 && errors(&workspace) == 0,
                errors(&workspace),
                top(&workspace).join(" ")
            );
            if errors(&workspace) > 0 {
                println!("{}", workspace.tree.sexp(&SHELL, true));
            }
        }
    }

    #[test]
    #[ignore]
    fn dump_one_script() {
        let cases: &[&[u8]] = &[
            b"cleanup()\n{\n\trm -f ${perfdata}\n\ttrap - EXIT TERM INT\n\texit $glb_err\n}\n",
            b"cleanup()\n{\n\techo hi\n}\n",
            b"f() { echo hi; }\n",
            b"get_canonical_version()\n{\n\tIFS=.\n\tset -- $1\n\techo $((100000 * $1 + 100 * $2 + $3))\n}\n",
        ];
        for case in cases {
            let workspace = parse(case);
            println!("ONE {:?} => errors {} {}", String::from_utf8_lossy(case), errors(&workspace), workspace.tree.sexp(&SHELL, true));
        }
    }

    #[test]
    #[ignore]
    fn dump_error_context() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/corpus-sh");
        let mut names: Vec<PathBuf> = fs::read_dir(&directory).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.is_file()).collect();
        names.sort();
        let mut shown = 0;
        for path in &names {
            let src = fs::read(path).unwrap();
            let workspace = parse(&src);
            if errors(&workspace) == 0 {
                continue;
            }
            let at = workspace.tree.nodes.iter().find(|node| node.kind == ERROR).map_or(0, |node| node.start as usize);
            let line_start = src[..at.min(src.len())].iter().rposition(|&byte| byte == b'\n').map_or(0, |index| index + 1);
            let line_end = src[at.min(src.len())..].iter().position(|&byte| byte == b'\n').map_or(src.len(), |index| index + at);
            println!("FILE {} at {}", path.file_name().unwrap().to_string_lossy(), at);
            println!("LINE {:?}", String::from_utf8_lossy(&src[line_start..line_end.min(src.len())]));
            let from = line_start.saturating_sub(200);
            let to = (line_end + 120).min(src.len());
            println!("CONTEXT {:?}", String::from_utf8_lossy(&src[from..to]));
            shown += 1;
            if shown == 6 {
                break;
            }
        }
    }

    /// The measured corpus: every `.sh` under the directory the environment
    /// names is parsed and counted. Real scripts are the only evidence that the
    /// grammar is not shaped around its own fixtures.
    #[test]
    #[ignore]
    fn real_shell_corpus() {
        let directory = std::env::var("SH_CORPUS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/corpus-sh"));
        let mut names: Vec<PathBuf> = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{directory:?}: {error}"))
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .collect();
        names.sort();
        let mut clean = 0;
        let mut total = 0;
        let mut broken: Vec<(String, usize, String)> = Vec::new();
        for path in &names {
            let src = fs::read(path).unwrap();
            if src.len() > 1 << 20 {
                continue;
            }
            total += 1;
            let workspace = parse(&src);
            let count = errors(&workspace);
            if count == 0 {
                clean += 1;
            } else {
                let at = workspace
                    .tree
                    .nodes
                    .iter()
                    .find(|node| node.kind == ERROR)
                    .map_or(0, |node| node.start as usize);
                let line = src[..at.min(src.len())].iter().filter(|&&byte| byte == b'\n').count() + 1;
                let snippet = String::from_utf8_lossy(&src[at.saturating_sub(40).min(src.len())..(at + 60).min(src.len())]).into_owned();
                broken.push((path.file_name().unwrap().to_string_lossy().into_owned(), line, snippet.replace('\n', "\\n")));
            }
        }
        broken.sort_by_key(|(_, _, snippet)| snippet.clone());
        println!("{clean} of {total} shell files parse with no error node");
        for (name, line, snippet) in broken.iter().take(12) {
            println!("  {name}:{line}: {snippet}");
        }
    }
}
