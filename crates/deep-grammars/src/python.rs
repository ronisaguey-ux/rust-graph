//! Python through `grammar!`: Slice 3. Layout tokens come from the runtime
//! lexer (`syntax::Layout`); expressions are one precedence table; the step
//! lexer below knows strings, f-strings and numbers.
//!
//! The tree is close to Tree-sitter's Python where that costs nothing, but it
//! is not the same tree: a plain string is one token, and a few rarely seen
//! forms nest differently. An f-string is a `f_string` node over a start token,
//! its text, one `interpolation` per replacement field and an end token, and
//! the mode stack (`modes`/`switch`) hands the bytes after `f"` to the text
//! step and the bytes inside a field back to code, so a field's expression is
//! a subtree and the field's `{` and `}` are tokens with a span each.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **Indentation is not a rule.** No `grammar!` form describes "a block is a
//!   deeper line". The `layout = NEWLINE INDENT DEDENT;` setting hands the
//!   three inferred tokens to the runtime lexer, and the rules consume them
//!   like any other token (`block: INDENT _statement* DEDENT`). A grammar that
//!   needed layout without a runtime that infers it could not be written.
//! - **An f-string's field is code again.** `modes { FSTRING = ..., FIELD =
//!   ... }` gives the mode stack a second frame: the `{` of a replacement field
//!   is `FSTRING_FIELD_OPEN`, which pushes `FIELD` (ordinary code), and the
//!   `}` that matches it at the field's own bracket depth is
//!   `FSTRING_FIELD_CLOSE`, which pops back to the text. `lex_field` keeps one
//!   `(depth, spec)` frame per open field so `f"{ {'a': 1}['a'] }"` closes at
//!   its own brace and a nested f-string or lambda cannot unbalance it; at
//!   depth 0 a `:` starts the format spec, which is text again until its `}`
//!   or a nested `{...}` field. The expression is therefore ordinary grammar
//!   nodes, and a call written in a field is an ordinary `call`.
//! - **No negative lookahead and no token beyond the third.** A choice is
//!   decided by the next one to three tokens, so `match`/`case`/`type` as soft
//!   keywords are resolved with a leading predicate (`&(...)`), not by a
//!   general lookahead. A construct that needs four tokens of context would
//!   have to be split into extra hidden rules. An assignment starts like the
//!   expression it assigns, so the statement rule asks a predicate whether the
//!   target list is followed by `=` (`&(_targets "=")`) and keeps one plain
//!   alternative.
//! - **One kind per rule.** Two alternatives that must produce different node
//!   names need two rules (`function_definition` versus `class_definition`),
//!   because naming an alternative is not part of the DSL. An assigned lambda
//!   therefore carries the name as the definition's own child, not as a second
//!   node kind.

use std::cell::RefCell;

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { PYTHON {
    name = "python";
    lex = lex;
    layout = NEWLINE INDENT DEDENT;

    /// After an f-string's opening quote the step lexer runs in `FSTRING`: the
    /// bytes are text, not code, until the closing quote. The start pushes the
    /// mode and the end (or a line that cannot close the string) pops it.
    ///
    /// `FIELD` is the opposite frame: the lexer pushed it at the field's `{`,
    /// so the bytes are code again until the `}` that matches that brace at
    /// its own bracket depth pops it. One switch token per transition is all
    /// the mode stack needs; the depth lives in `lex_field`'s own state.
    modes {
        FSTRING = lex_fstring_text,
        FIELD = lex_field,
    }
    switch FSTRING_START => push FSTRING;
    switch FSTRING_END => pop;
    switch FSTRING_BAD => pop FSTRING;
    switch FSTRING_FIELD_OPEN => push FIELD;
    switch FSTRING_FIELD_CLOSE => pop;

    tokens {
        /// A line break; the runtime turns these into logical line ends.
        NEWLINE = "newline",
        INDENT = "indent" open block,
        DEDENT = "dedent" close block,
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        NUMBER = "number",
        STR = "string_literal",
        /// A string with no closing quote.
        BAD_STR = "bad_string" bad,
        /// `f"` or `rf'''`: the prefix and opening quotes of an f-string.
        FSTRING_START = "fstring_start",
        /// The bytes between fields.
        FSTRING_TEXT = "fstring_text",
        /// The `{` of a replacement field: code again until its own `}`.
        FSTRING_FIELD_OPEN = "fstring_field_open" open field,
        /// The `}` that closes a replacement field at the field's own depth.
        FSTRING_FIELD_CLOSE = "fstring_field_close" close field,
        /// The bytes of a format spec between its nested fields.
        FSTRING_SPEC_TEXT = "fstring_spec_text",
        /// The closing quotes of an f-string.
        FSTRING_END = "fstring_end",
        /// An f-string that cannot close on its line.
        FSTRING_BAD = "bad_fstring" bad,

        KW_FALSE = "False",
        KW_NONE = "None",
        KW_TRUE = "True",
        KW_AND = "and",
        KW_AS = "as",
        KW_ASSERT = "assert" reset,
        KW_ASYNC = "async",
        KW_AWAIT = "await",
        KW_BREAK = "break" reset,
        KW_CLASS = "class" reset,
        KW_CONTINUE = "continue" reset,
        KW_DEF = "def" reset,
        KW_DEL = "del" reset,
        KW_ELIF = "elif" reset,
        KW_ELSE = "else",
        KW_EXCEPT = "except" reset,
        KW_FINALLY = "finally" reset,
        KW_FOR = "for",
        KW_FROM = "from",
        KW_GLOBAL = "global" reset,
        KW_IF = "if",
        KW_IMPORT = "import" reset,
        KW_IN = "in",
        KW_IS = "is",
        KW_LAMBDA = "lambda",
        KW_NONLOCAL = "nonlocal" reset,
        KW_NOT = "not",
        KW_OR = "or",
        KW_PASS = "pass" reset,
        KW_RAISE = "raise" reset,
        KW_RETURN = "return" reset,
        KW_TRY = "try" reset,
        KW_WHILE = "while" reset,
        KW_WITH = "with" reset,
        KW_YIELD = "yield",
        /// Soft keywords: names everywhere but where they start their statement.
        KW_MATCH = "match",
        KW_CASE = "case",
        KW_TYPE = "type",

        COMMA = ",",
        COLON = ":",
        SEMI = ";",
        DOT = ".",
        ELLIPSIS = "...",
        ARROW = "->",
        AT = "@",
        ASSIGN = "=",
        WALRUS = ":=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        STARSTAR = "**",
        SLASH = "/",
        SLASHSLASH = "//",
        PERCENT = "%",
        PIPE = "|",
        CARET = "^",
        AMP = "&",
        TILDE = "~",
        SHL = "<<",
        SHR = ">>",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        EQ = "==",
        NE = "!=",
        LTGT = "<>",
        BANG = "!",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        STARSTAR_EQ = "**=",
        SLASH_EQ = "/=",
        SLASHSLASH_EQ = "//=",
        PERCENT_EQ = "%=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        AMP_EQ = "&=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        AT_EQ = "@=",
    }

    module: _statement*,

    _statement:
        &("match" _expressions ":") match_statement
        // A `def` does not start with the token the ordinary statements start
        // with, so two guards pick it before the plain choice; the plain tree
        // then holds only keyword-led compound forms and the simple statements.
        // `x = lambda ...` is picked inside `_simple_statements`, where its new
        // line is the statement's.
        | &("def") function_definition
        | &("async" "def") function_definition
        | _compound_statement
        | _simple_statements,

    _simple_statements: _simple_statement [";" (_simple_statement %? ";")] NEWLINE,
    _suite: _simple_statements | NEWLINE block,
    block: INDENT _statement* DEDENT,

    _simple_statement:
        import_statement
        | import_from_statement
        | assert_statement
        | return_statement
        | delete_statement
        | raise_statement
        | pass_statement
        | break_statement
        | continue_statement
        | global_statement
        | nonlocal_statement
        | type_alias_statement
        | expression_statement,

    import_statement: "import" _import_names,
    import_from_statement: "from" (relative_import | dotted_name) "import" ("*" | import_list | _import_names),
    import_list: "(" (_import_name %? ",") ")",
    relative_import: ("." | "...")+ [dotted_name],
    _import_names: _import_name ("," _import_name)*,
    _import_name: dotted_name ["as" _name],
    dotted_name: _name ("." _name)*,
    assert_statement: "assert" _expression ["," _expression],
    return_statement: "return" [_expressions],
    delete_statement: "del" _expressions,
    raise_statement: "raise" [_expression ["from" _expression]],
    pass_statement: "pass",
    break_statement: "break",
    continue_statement: "continue",
    global_statement: "global" _name ("," _name)*,
    nonlocal_statement: "nonlocal" _name ("," _name)*,
    type_alias_statement: "type" _name [type_parameters] "=" _expression,

    expression_statement: &(_name "=" "lambda") function_definition | &(_targets ":") annotated | &(_targets "=") assignment | _assignment_chain,
    /// `a: int`, `a: int = 1`, `a: int = b = 1`: the annotation's target keeps
    /// its node, so a reader can name what is declared.
    annotated: _targets ":" _expression ["=" _assignment_rhs],
    /// `a, b = f(x), 1`: the target list keeps its own node (`pattern_list`, or
    /// `list`/`attribute`/`subscript`), so a reader can name every binding. The
    /// right side recurses, so `a = b = 1` nests one assignment per binding.
    assignment: _targets "=" _assignment_rhs,
    _assignment_rhs: &(_targets "=") assignment | _assignment_chain,
    /// `a += 1`, or an expression alone.
    _assignment_chain: precedence _expressions {
        right "+=" | "-=" | "*=" | "**=" | "/=" | "//=" | "%=" | "|=" | "^=" | "&=" | "<<=" | ">>=" | "@=" => augmented_assignment,
    },

    _compound_statement:
        if_statement
        | for_statement
        | while_statement
        | try_statement
        | with_statement
        | class_definition
        | decorated_definition,

    if_statement: "if" _expression ":" _suite elif_clause* [else_clause],
    elif_clause: "elif" _expression ":" _suite,
    else_clause: "else" ":" _suite,
    for_statement: ["async"] "for" _targets "in" _expressions ":" _suite [else_clause],
    while_statement: "while" _expression ":" _suite [else_clause],
    try_statement: "try" ":" _suite except_clause* [else_clause] [finally_clause],
    except_clause: "except" ["*"] [_expressions] ":" _suite,
    finally_clause: "finally" ":" _suite,
    with_statement: ["async"] "with" _expression ("," _expression)* ":" _suite,
    function_definition: ["async"] "def" _name [type_parameters] parameters ["->" _expression] ":" _suite | _name "=" lambda,
    class_definition: "class" _name [type_parameters] [argument_list] ":" _suite,
    decorated_definition: decorator+ (class_definition | function_definition),
    decorator: "@" _expression NEWLINE,

    parameters: "(" (_param_item %? ",") ")",
    _param_item: parameter | star_parameter | "/",
    parameter: _name [":" _expression] ["=" _expression],
    star_parameter: ("*" | "**") [_name [":" _star_expression]],
    type_parameters: "[" (type_parameter %? ",") "]",
    type_parameter: ["*" | "**"] _name [":" _expression] ["=" _expression],

    match_statement: "match" _expressions ":" NEWLINE case_block,
    case_block: INDENT case_clause* DEDENT,
    case_clause: "case" _patterns ["if" _expression] ":" _suite,
    _patterns: precedence _pattern { postfix ("," [_pattern])+ => pattern_list, },
    _pattern: list_splat | _expression@as_pattern,

    /// Targets of `for` and comprehensions: no comparison, so `in` ends them.
    _targets: precedence _target { postfix ("," [_target])+ => pattern_list, },
    _target: list_splat | _expression@bitwise_or,

    _expressions: precedence _star_expression { postfix ("," [_star_expression])+ => expression_list, },
    _star_expression: list_splat | _expression,
    list_splat: "*" _expression@bitwise_or,
    dictionary_splat: "**" _expression@bitwise_or,

    _expression: precedence _primary {
        named: right ":=" => named_expression,
        cond: right "if" _expression@disjunction "else" => conditional_expression,
        as_pattern: left "as" => as_pattern,
        disjunction: left "or" => boolean_operator,
        left "and" => boolean_operator,
        prefix "not" => not_operator,
        left "<" | ">" | "==" | ">=" | "<=" | "!=" | "<>" | "in" | "not" "in" | "is" ["not"] => comparison_operator,
        bitwise_or: left "|" => binary_operator,
        left "^" => binary_operator,
        left "&" => binary_operator,
        left "<<" | ">>" => binary_operator,
        left "+" | "-" => binary_operator,
        left "*" | "@" | "/" | "//" | "%" => binary_operator,
        prefix "+" | "-" | "~" => unary_operator,
        right "**" => binary_operator,
        prefix "await" => await,
        postfix argument_list => call,
        postfix subscript_list => subscript,
        postfix "." _name => attribute,
    },

    _primary:
        _name
        | NUMBER
        | string
        | f_string
        | "True"
        | "False"
        | "None"
        | "..."
        | parenthesized_expression
        | list
        | dictionary
        | lambda
        | yield,

    _name: IDENT | "match" | "case" | "type",
    string: (STR | BAD_STR)+,
    /// `f"a{x!r:>{w}}b"`: the opening quotes, the text between fields, one
    /// node per replacement field and the closing quotes.
    f_string: FSTRING_START (_fstring_part)* FSTRING_END,
    _fstring_part: string_content | interpolation,
    string_content: FSTRING_TEXT,
    /// `{x!r:>{w}}`: the braces keep their own tokens, and what is between
    /// them is ordinary Python — a call in a field is a `call` node like any
    /// other. `=` is the debug form (`{x=}`), `!r` the conversion, and the
    /// format spec is text with fields of its own.
    interpolation: FSTRING_FIELD_OPEN [_expressions] [_fstring_debug] [BANG _name] [COLON [_fstring_spec]] FSTRING_FIELD_CLOSE,
    _fstring_debug: ASSIGN [_expression],
    _fstring_spec: (FSTRING_SPEC_TEXT | interpolation)+,
    parenthesized_expression: "(" (_list_item %? ",") ")",
    list: "[" (_list_item %? ",") "]",
    dictionary: "{" (_dictionary_item %? ",") "}",
    _list_item: _star_expression [_comprehension],
    _dictionary_item: _key_value [_comprehension],
    _key_value: precedence _dictionary_entry { left ":" => pair, },
    _dictionary_entry: dictionary_splat | _star_expression,
    _comprehension: for_in_clause (for_in_clause | if_clause)*,
    for_in_clause: ["async"] "for" _targets "in" _expression@disjunction,
    if_clause: "if" _expression@disjunction,
    lambda: "lambda" (_lambda_item %? ",") ":" _expression,
    _lambda_item: lambda_parameter | lambda_star_parameter | "/",
    lambda_star_parameter: ("*" | "**") [_name],
    lambda_parameter: _name ["=" _expression],
    yield: "yield" ["from" _expression | _expressions],

    argument_list: "(" (_argument %? ",") ")",
    _argument: keyword_argument | list_splat | dictionary_splat | _expression [_comprehension],
    keyword_argument: _name "=" _expression,

    subscript_list: "[" (_slice_item %? ",") "]",
    _slice_item: precedence _slice_start { postfix ":" [_expression] [":" [_expression]] => slice, },
    _slice_start: open_slice | _star_expression,
    open_slice: ":" [_expression] [":" [_expression]],
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    if c.pos() == 0 {
        // A new file starts outside every mode, and whatever state a parse
        // that stopped inside an f-string left behind is not this file's.
        FIELDS.with(|fields| fields.borrow_mut().clear());
        FSTRING_QUOTES.with(|quotes| quotes.borrow_mut().clear());
    }
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0c'));
            WHITESPACE
        }
        b'\n' => {
            c.bump();
            NEWLINE
        }
        b'\r' => {
            c.bump();
            c.eat(b'\n');
            NEWLINE
        }
        b'\\' if matches!(c.peek_at(1), b'\n' | b'\r') => {
            // A joined line is one line: whitespace, not a line break.
            c.bump();
            c.eat(b'\r');
            c.eat(b'\n');
            WHITESPACE
        }
        b'#' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        0xef if c.pos() == 0 && c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'\'' | b'"' => string(c),
        b'a'..=b'z' | b'A'..=b'Z' | b'_' | 0x80..=0xff => word(c),
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() => number(c),
        _ => operator(c),
    }
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// The string prefix letters `word` spells, if it is one: whether it makes an
/// f-string (or t-string), whose fields hold code.
fn string_prefix(word: &[u8]) -> Option<bool> {
    if word.len() > 2 {
        return None;
    }
    let mut seen = 0u8;
    let mut code = false;
    for &letter in word {
        let bit = match letter.to_ascii_lowercase() {
            b'r' => 1,
            b'b' => 2,
            b'u' => 4,
            b'f' | b't' => {
                code = true;
                8
            }
            _ => return None,
        };
        if seen & bit != 0 {
            return None;
        }
        seen |= bit;
    }
    // `u` stands alone; bytes are never f-strings.
    if (seen & 4 != 0 && word.len() > 1) || seen & 10 == 10 {
        return None;
    }
    Some(code)
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    if matches!(c.peek(), b'\'' | b'"') {
        match string_prefix(text) {
            Some(true) => return fstring_start(c),
            Some(false) => return string(c),
            None => {}
        }
    }
    keyword(text)
}

thread_local! {
    /// The quotes of every f-string the `FSTRING` mode is inside, innermost
    /// last: a nested f-string restores the outer one's quote on the way out,
    /// as the heredoc's delimiter lives in its own step function.
    static FSTRING_QUOTES: RefCell<Vec<(u8, bool)>> = const { RefCell::new(Vec::new()) };
    /// One frame per open replacement field: the brackets opened inside it
    /// and whether its format spec has started. The frame is pushed when the
    /// lexer emits `FSTRING_FIELD_OPEN` and popped on the matching close, so
    /// `f"{ {'a': 1}['a'] }"` closes at its own brace.
    static FIELDS: RefCell<Vec<Field>> = const { RefCell::new(Vec::new()) };
}

/// What one open replacement field holds.
#[derive(Clone, Copy)]
struct Field {
    /// Openers minus closers inside the field.
    depth: u32,
    /// A `:` at depth 0 began the format spec: the bytes are text again.
    spec: bool,
    /// The f-string the field was opened in, as the quote stack's depth. A
    /// broken string abandons every field opened at that depth, however many.
    string: usize,
}

/// `f"`, `rf'''`, `t"`: the prefix was consumed by `word`; this takes the
/// opening quotes and remembers how the body closes.
fn fstring_start(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    let triple = c.peek_at(1) == quote && c.peek_at(2) == quote;
    c.bump_n(if triple { 3 } else { 1 });
    FSTRING_QUOTES.with(|quotes| quotes.borrow_mut().push((quote, triple)));
    FSTRING_START
}

/// The quotes of the innermost f-string, if the mode stack and this state
/// still agree.
fn fstring_quotes() -> Option<(u8, bool)> {
    FSTRING_QUOTES.with(|quotes| quotes.borrow().last().copied())
}

fn pop_fstring() {
    FSTRING_QUOTES.with(|quotes| {
        quotes.borrow_mut().pop();
    });
}

/// The `FSTRING` mode's step: text between fields, one `{` that opens a
/// replacement field, or the closing quotes. In a one-line string a line
/// break ends it badly, so a broken string costs its line and the mode pops.
pub fn lex_fstring_text(c: &mut Cursor) -> Kind {
    if c.at_end() {
        pop_fstring();
        return FSTRING_BAD;
    }
    let Some((quote, triple)) = fstring_quotes() else {
        return FSTRING_BAD;
    };
    let closes = if triple { c.peek() == quote && c.peek_at(1) == quote && c.peek_at(2) == quote } else { c.peek() == quote };
    if closes {
        c.bump_n(if triple { 3 } else { 1 });
        pop_fstring();
        return FSTRING_END;
    }
    if !triple && matches!(c.peek(), b'\n' | b'\r') {
        c.bump();
        c.eat(b'\n');
        pop_fstring();
        return FSTRING_BAD;
    }
    if c.peek() == b'{' && c.peek_at(1) != b'{' {
        c.bump();
        // The field's expression is code: the mode stack returns to code here
        // and `lex_field` closes it at the brace that matches this one.
        open_field();
        return FSTRING_FIELD_OPEN;
    }
    // Text up to a field, the closing quotes, or a line break in a one-line
    // string. `{{` and `}}` are escaped braces and stay text.
    loop {
        if c.at_end() {
            return FSTRING_TEXT;
        }
        let byte = c.peek();
        if triple {
            if byte == quote && c.peek_at(1) == quote && c.peek_at(2) == quote {
                return FSTRING_TEXT;
            }
        } else if byte == quote || matches!(byte, b'\n' | b'\r') {
            return FSTRING_TEXT;
        }
        if byte == b'{' && c.peek_at(1) != b'{' {
            return FSTRING_TEXT;
        }
        match byte {
            b'\\' => c.bump_n(2),
            b'{' | b'}' if c.peek_at(1) == byte => c.bump_n(2),
            _ => c.bump(),
        }
    }
}

fn open_field() {
    let string = FSTRING_QUOTES.with(|quotes| quotes.borrow().len());
    FIELDS.with(|fields| fields.borrow_mut().push(Field { depth: 0, spec: false, string }));
}

fn close_field() {
    FIELDS.with(|fields| {
        fields.borrow_mut().pop();
    });
}

/// A field that cannot close ends its whole f-string: every field opened in
/// that string loses its frame and the string loses its quote. The mode stack
/// then pops through `FSTRING` on the one bad token.
fn abandon_string() {
    let string = FSTRING_QUOTES.with(|quotes| quotes.borrow().len());
    FIELDS.with(|fields| {
        let mut fields = fields.borrow_mut();
        while fields.last().is_some_and(|field| field.string == string) {
            fields.pop();
        }
    });
    pop_fstring();
}

/// Change the innermost field's frame; a no-op when none is open.
fn with_field(change: impl FnOnce(&mut Field)) {
    FIELDS.with(|fields| {
        if let Some(field) = fields.borrow_mut().last_mut() {
            change(field);
        }
    });
}

/// The `FIELD` mode's step: ordinary code, with the two tokens that only exist
/// inside a replacement field. At the field's own depth a `}` closes it and a
/// `:` starts the format spec; deeper, they are a dict's brace and a slice's
/// colon, handled by `lex`. A line break at depth 0 in a one-line string ends
/// the field badly, as it ends the string itself.
pub fn lex_field(c: &mut Cursor) -> Kind {
    let Some(field) = FIELDS.with(|fields| fields.borrow().last().copied()) else {
        // No field is open, so this frame and the mode stack disagree: lexing
        // code cannot lose a token, and the parser still gets a tree.
        return lex(c);
    };
    if c.at_end() {
        abandon_string();
        return FSTRING_BAD;
    }
    if field.spec {
        // A format spec: text until the field's `}`, except that a single `{`
        // opens one more field and `{{` is an escaped brace.
        if c.peek() == b'}' {
            c.bump();
            close_field();
            return FSTRING_FIELD_CLOSE;
        }
        if c.peek() == b'{' && c.peek_at(1) != b'{' {
            c.bump();
            open_field();
            return FSTRING_FIELD_OPEN;
        }
        let triple = fstring_quotes().is_some_and(|(_, triple)| triple);
        let start = c.pos();
        while !c.at_end() {
            match c.peek() {
                b'}' => break,
                b'{' if c.peek_at(1) == b'{' => c.bump_n(2),
                b'{' => break,
                b'\n' | b'\r' if !triple => break,
                _ => c.bump(),
            }
        }
        if matches!(c.peek(), b'\n' | b'\r') && !triple {
            // A spec does not outlive its line in a one-line f-string either.
            c.bump();
            c.eat(b'\n');
            abandon_string();
            return FSTRING_BAD;
        }
        if c.pos() == start {
            // A brace the checks above did not take: consume it so the loop
            // always advances, and leave the rest to the parser.
            c.bump();
        }
        return FSTRING_SPEC_TEXT;
    }
    if field.depth == 0 {
        match c.peek() {
            b'}' => {
                c.bump();
                close_field();
                return FSTRING_FIELD_CLOSE;
            }
            // `:=` is the walrus, not a format spec.
            b':' if c.peek_at(1) != b'=' => {
                c.bump();
                with_field(|field| field.spec = true);
                return COLON;
            }
            b'\n' | b'\r' if !fstring_quotes().is_some_and(|(_, triple)| triple) => {
                // A one-line f-string cannot hold this field: the string ends
                // badly at its line, as it does between fields.
                c.bump();
                c.eat(b'\n');
                abandon_string();
                return FSTRING_BAD;
            }
            _ => {}
        }
    }
    let kind = lex(c);
    match kind {
        LPAREN | LBRACKET | LBRACE => with_field(|field| field.depth += 1),
        RPAREN | RBRACKET | RBRACE => with_field(|field| field.depth = field.depth.saturating_sub(1)),
        _ => {}
    }
    kind
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"False" => KW_FALSE,
        b"None" => KW_NONE,
        b"True" => KW_TRUE,
        b"and" => KW_AND,
        b"as" => KW_AS,
        b"assert" => KW_ASSERT,
        b"async" => KW_ASYNC,
        b"await" => KW_AWAIT,
        b"break" => KW_BREAK,
        b"class" => KW_CLASS,
        b"continue" => KW_CONTINUE,
        b"def" => KW_DEF,
        b"del" => KW_DEL,
        b"elif" => KW_ELIF,
        b"else" => KW_ELSE,
        b"except" => KW_EXCEPT,
        b"finally" => KW_FINALLY,
        b"for" => KW_FOR,
        b"from" => KW_FROM,
        b"global" => KW_GLOBAL,
        b"if" => KW_IF,
        b"import" => KW_IMPORT,
        b"in" => KW_IN,
        b"is" => KW_IS,
        b"lambda" => KW_LAMBDA,
        b"nonlocal" => KW_NONLOCAL,
        b"not" => KW_NOT,
        b"or" => KW_OR,
        b"pass" => KW_PASS,
        b"raise" => KW_RAISE,
        b"return" => KW_RETURN,
        b"try" => KW_TRY,
        b"while" => KW_WHILE,
        b"with" => KW_WITH,
        b"yield" => KW_YIELD,
        b"match" => KW_MATCH,
        b"case" => KW_CASE,
        b"type" => KW_TYPE,
        _ => IDENT,
    }
}

/// A string from its opening quote. A one-line string ends at its line if it
/// does not close there; a triple-quoted one runs to the end of the file. An
/// f-string never comes here: its quotes and its fields have tokens of their
/// own, so the expression in a field is parsed.
fn string(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    let triple = c.peek_at(1) == quote && c.peek_at(2) == quote;
    c.bump_n(if triple { 3 } else { 1 });
    if string_body(c, quote, triple) { STR } else { BAD_STR }
}

/// The rest of a string after its opening quotes. True if it closed.
fn string_body(c: &mut Cursor, quote: u8, triple: bool) -> bool {
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'\\' => c.bump_n(2),
            byte if byte == quote => {
                if !triple {
                    c.bump();
                    return true;
                }
                if c.peek_at(1) == quote && c.peek_at(2) == quote {
                    c.bump_n(3);
                    return true;
                }
                c.bump();
            }
            b'\n' | b'\r' if !triple => return false,
            _ => c.bump(),
        }
    }
}

fn number(c: &mut Cursor) -> Kind {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return NUMBER;
    }
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.peek() == b'.' {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'e' | b'E') {
        let sign = usize::from(matches!(c.peek_at(1), b'+' | b'-'));
        if c.peek_at(1 + sign).is_ascii_digit() {
            c.bump_n(1 + sign);
            c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
        }
    }
    if matches!(c.peek(), b'j' | b'J' | b'l' | b'L') {
        c.bump();
    }
    NUMBER
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'*', b'*', b'=') => (STARSTAR_EQ, 3),
        (b'/', b'/', b'=') => (SLASHSLASH_EQ, 3),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'*', b'*', _) => (STARSTAR, 2),
        (b'/', b'/', _) => (SLASHSLASH, 2),
        (b'<', b'<', _) => (SHL, 2),
        (b'>', b'>', _) => (SHR, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'<', b'>', _) => (LTGT, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b':', b'=', _) => (WALRUS, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'@', b'=', _) => (AT_EQ, 2),
        (b'(', ..) => (LPAREN, 1),
        (b')', ..) => (RPAREN, 1),
        (b'[', ..) => (LBRACKET, 1),
        (b']', ..) => (RBRACKET, 1),
        (b'{', ..) => (LBRACE, 1),
        (b'}', ..) => (RBRACE, 1),
        (b',', ..) => (COMMA, 1),
        (b':', ..) => (COLON, 1),
        (b';', ..) => (SEMI, 1),
        (b'.', ..) => (DOT, 1),
        (b'@', ..) => (AT, 1),
        (b'=', ..) => (ASSIGN, 1),
        (b'+', ..) => (PLUS, 1),
        (b'-', ..) => (MINUS, 1),
        (b'*', ..) => (STAR, 1),
        (b'/', ..) => (SLASH, 1),
        (b'%', ..) => (PERCENT, 1),
        (b'|', ..) => (PIPE, 1),
        (b'^', ..) => (CARET, 1),
        (b'&', ..) => (AMP, 1),
        (b'~', ..) => (TILDE, 1),
        (b'<', ..) => (LT, 1),
        (b'>', ..) => (GT, 1),
        (b'!', ..) => (BANG, 1),
        _ => return UNKNOWN,
    };
    c.bump_n(width);
    kind
}

