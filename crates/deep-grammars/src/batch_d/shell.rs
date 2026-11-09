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

