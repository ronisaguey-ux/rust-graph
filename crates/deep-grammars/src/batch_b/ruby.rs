//! Ruby through `grammar!`: one of Field 6's four languages (Ruby, PHP, Swift,
//! Scala).
//!
//! The grammar is statement-oriented and `end`-delimited, so no layout rule is
//! needed: a line break is whitespace, and a block ends at the keyword that
//! closes it (`end`, `}`, `]`, `)`). Node names follow Tree-sitter's Ruby where
//! that costs nothing; the tree is not the same tree.
//!
//! What the DSL covers here:
//!
//! - **Blocks and bodies.** `body: _statement*` stops at whatever cannot start
//!   a statement, which is exactly the closing keyword, so `def`, `class` and
//!   `if` bodies are plain sequences of statements. `begin ... rescue ...
//!   ensure ... end` and `case ... when ... else ... end` are ordinary rules
//!   with optional clauses.
//! - **Expression statements do the assignment.** Assignment is the loosest
//!   row of one precedence table and postfix calls, members, indexes and
//!   blocks are the tightest, so `a = b = 1`, `a += 1`, `x.y = 1` and
//!   `xs.each { |x| ... }` all parse without special statement forms.
//! - **Blocks are postfix operators.** `xs.each { |x| ... }` and `xs.each do
//!   |x| ... end` are `call_with_block` rows, because a `{` after an operand
//!   is a block and at the start of a statement it is a hash.
//! - **A call is `arguments`; a command call is not.** `foo(1, a: 2)` is a
//!   `call` node. Bare `puts x` (Ruby's command call) has no separate node
//!   here: `puts x` would read as the expression `puts` followed by the
//!   expression `x`, which is the one form this grammar does not model (see
//!   the gaps below).
//! - **Strings are single tokens.** A double-quoted string's `#{...}` is
//!   scanned with its braces and inner strings balanced and stays opaque, so a
//!   field holding code is not parsed; the same holds for `%Q(...)`,
//!   `%w[...]`, backticks, `//`, and heredocs.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **Heredocs are a lexer state.** The terminator is a line that equals the
//!   delimiter, and it arrives after the token that started the heredoc. The
//!   DSL has no external scanner, so `<<~EOS` is scanned to its terminator by
//!   the step lexer, which decides heredoc against `<<` from
//!   [`Cursor::previous`]: after an operand `<<` shifts, elsewhere it starts a
//!   heredoc. That is the same trick the JavaScript grammar uses for `/`.
//! - **Regex against division.** Same problem, same answer: `/` starts a
//!   regex where an operand cannot end, and divides elsewhere. One regex is
//!   one token; its groups are not a tree.
//! - **A symbol and a label share `:`.** `:name` (no space after the colon)
//!   is one `symbol` token, while `key: value` (space or a non-name after the
//!   colon) is a label. `{a:b}` therefore reads as a symbol rather than a
//!   label, which is the one place the lexer's rule bites; Ruby's own rules
//!   here need the lexer state this DSL does not have.
//! - **Command calls.** `puts x` is a call with no parentheses; a rule for it
//!   would have to decide between "a name, then a statement" and "a name, then
//!   an argument" with no token to tell them apart, so the grammar does not
//!   have one and fixtures call with parentheses (`puts(x)`).
//! - **A decision sees three tokens.** `&(...)` predicates start the two
//!   alternatives that share a prefix: a hash/argument label (`&(_name ":")`)
//!   against an expression.
//! - **One kind per rule**, so `if` and `unless` are two rules, not one
//!   labelled alternative.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { RUBY {
    name = "ruby";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        SYMBOL = "simple_symbol",
        /// `[]`: an empty array, or the name of an index method.
        INDEX_NAME = "[]",
        INSTANCE_VAR = "instance_variable",
        CLASS_VAR = "class_variable",
        GLOBAL_VAR = "global_variable",
        REGEX = "regex",
        /// A string, symbol, regex or heredoc with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_ALIAS = "alias",
        KW_BEGIN = "begin",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CLASS = "class",
        KW_DEF = "def",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ELSIF = "elsif",
        KW_END = "end",
        KW_ENSURE = "ensure",
        KW_FALSE = "false",
        KW_FOR = "for",
        KW_IF = "if",
        /// `if` after an expression on the same line: `x = 1 if y`. The lexer
        /// tells the two apart by whether a line break came before the word,
        /// because a Ruby line break ends the statement unless a modifier
        /// follows it.
        MOD_IF = "if_modifier",
        MOD_UNLESS = "unless_modifier",
        MOD_WHILE = "while_modifier",
        MOD_UNTIL = "until_modifier",
        MOD_RESCUE = "rescue_modifier",
        KW_IN = "in",
        KW_MODULE = "module",
        KW_NEXT = "next",
        KW_NIL = "nil",
        KW_RESCUE = "rescue",
        KW_RETURN = "return",
        KW_SELF = "self",
        KW_SUPER = "super",
        KW_THEN = "then",
        KW_TRUE = "true",
        KW_UNDEF = "undef",
        KW_UNLESS = "unless",
        KW_UNTIL = "until",
        KW_WHEN = "when",
        KW_WHILE = "while",
        KW_YIELD = "yield",
        /// `and`, `or` and `not`: word operators.
        KW_AND = "and",
        KW_OR = "or",
        KW_NOT = "not",
        /// Methods that take a path: `require`, `load`, `include`.
        KW_REQUIRE = "require",
        KW_REQUIRE_RELATIVE = "require_relative",
        KW_LOAD = "load",
        KW_INCLUDE = "include",
        KW_EXTEND = "extend",
        KW_PREPEND = "prepend",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        SAFE_DOT = "&.",
        COLON = ":",
        COLONCOLON = "::",
        QUESTION = "?",
        FATARROW = "=>",
        ARROW = "->",
        ASSIGN = "=",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        STARSTAR_EQ = "**=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        ANDAND_EQ = "&&=",
        OROR_EQ = "||=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        STARSTAR = "**",
        SLASH = "/",
        PERCENT = "%",
        PIPE = "|",
        AMP = "&",
        CARET = "^",
        TILDE = "~",
        BANG = "!",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        CMP = "<=>",
        EQ = "==",
        TEQ = "===",
        NE = "!=",
        MATCH = "=~",
        NMATCH = "!~",
        ANDAND = "&&",
        OROR = "||",
        SHL = "<<",
        SHR = ">>",
        DOTDOT = "..",
        ELLIPSIS = "...",
    }

    program: _statement*,

    _statement:
        require_statement
        | alias_statement
        | undef_statement
        | include_statement
        | module_definition
        | class_definition
        | method_definition
        | if_statement
        | unless_statement
        | while_statement
        | until_statement
        | for_statement
        | case_statement
        | begin_statement
        | return_statement
        | break_statement
        | next_statement
        | ";"
        | expression_statement,

    // ---- top-level statements ----

    require_statement: ("require" | "require_relative" | "load") _expression _modifier*,
    /// `alias new old`, with both sides a name, a symbol or a global.
    alias_statement: "alias" _alias_target _alias_target,
    _alias_target: SYMBOL | GLOBAL_VAR | INSTANCE_VAR | IDENT,
    undef_statement: "undef" (_alias_target % ","),
    include_statement: ("include" | "extend" | "prepend") _constant_path ("," _constant_path)*,

    module_definition: "module" _constant_path body "end" _modifier*,
    class_definition: "class" _constant_path ["<" _constant_path] body "end" _modifier*,
    method_definition: "def" method_name [parameters] body rescue_clause* [else_clause] [ensure_clause] "end" _modifier*,
    method_name: ["self" ("." | "::")] (setter_name | index_method_name | _method_ident),
    setter_name: IDENT "=",
    index_method_name: "[]" ["="],
    /// `def +(other)`, `def <=>(other)`, `def ~`: an operator is a method name.
    _method_ident: IDENT | "~" | "+" | "-" | "*" | "/" | "%" | "**" | "==" | "===" | "!=" | "!"
        | "<" | ">" | "<=" | ">=" | "<=>" | "=~" | "!~" | "<<" | ">>" | "&" | "|" | "^",
    parameters: "(" [_parameter_list] ")",
    _parameter_list: _parameter %? ",",
    _parameter:
        splat_parameter
        | double_splat_parameter
        | block_parameter
        | destructured_parameter
        | keyword_parameter
        | _name ["=" _expression@assign],
    splat_parameter: "*" [_name],
    double_splat_parameter: "**" [_name],
    block_parameter: "&" _name,
    keyword_parameter: _name ":" [_expression],
    destructured_parameter: "(" [_parameter_list] ")",
    _constant_path: _name ("::" _name)*,

    /// Statements up to whatever closes the construct: a keyword such as `end`,
    /// or the bracket of a block.
    body: _statement*,

    if_statement: "if" _expression [then_token] body elsif_clause* [else_clause] "end",
    elsif_clause: "elsif" _expression [then_token] body,
    else_clause: "else" body,
    then_token: "then",
    unless_statement: "unless" _expression [then_token] body [else_clause] "end",
    while_statement: "while" _expression ["do"] body "end",
    until_statement: "until" _expression ["do"] body "end",
    for_statement: "for" (_name % ",") "in" _expression ["do"] body "end",

    case_statement: "case" [_expression] when_clause+ [else_clause] "end",
    when_clause: "when" (_argument % ",") [then_token] body,

    begin_statement: "begin" body rescue_clause* [else_clause] [ensure_clause] "end",
    rescue_clause: "rescue" [_exception_list] ["=>" _name] body,
    _exception_list: _constant_path ("," _constant_path)*,
    ensure_clause: "ensure" body,

    return_statement: "return" [_expression ("," _expression)*] _modifier*,
    break_statement: "break" _modifier*,
    next_statement: "next" _modifier*,
    expression_statement: _expression _modifier*,

    /// What follows a statement on its own line: `x if y`, `x while y`,
    /// `value rescue fallback`.
    _modifier: if_modifier | unless_modifier | while_modifier | until_modifier | rescue_modifier,
    if_modifier: "if_modifier" _expression,
    unless_modifier: "unless_modifier" _expression,
    while_modifier: "while_modifier" _expression,
    until_modifier: "until_modifier" _expression,
    rescue_modifier: "rescue_modifier" _expression,

    // ---- blocks ----

    brace_block: "{" [block_parameters] _statement* "}",
    do_block: "do" [block_parameters] _statement* "end",
    block_parameters: "|" [_parameter_list] "|",

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "**=" | "&=" | "|=" | "^=" | "&&=" | "||=" | "<<=" | ">>=" => assignment,
        cond: postfix "?" _expression ":" _expression@assign => conditional,
        word_or: left "or" => binary,
        word_and: left "and" => binary,
        range: left ".." | "..." => range,
        or: left "||" => binary,
        and: left "&&" => binary,
        bor: left "|" => binary,
        xor: left "^" => binary,
        band: left "&" => binary,
        eq: left "==" | "!=" | "===" | "=~" | "!~" | "<=>" => binary,
        cmp: left "<" | ">" | "<=" | ">=" => binary,
        shift: left "<<" | ">>" => binary,
        add: left "+" | "-" => binary,
        mul: left "*" | "/" | "%" => binary,
        exp: right "**" => binary,
        unary: prefix "!" | "-" | "+" | "~" | "not" => unary,
        range_from: prefix ".." | "..." => range,
        block: postfix brace_block => call_with_block,
        do_block: postfix do_block => call_with_block,
        call: postfix arguments => call,
        index: postfix subscript => element_reference,
        member: postfix "." _property => member,
        safe_member: postfix "&." _property => member,
        scope: postfix "::" _property => scope,
    },

    _primary:
        _literal
        | _variable
        | _name
        | "self"
        | parenthesized_expression
        | array
        | hash
        | super_call
        | yield_expression
        | lambda
        | absolute_constant
        | "[]",

    _literal: NUMBER | STRING | SYMBOL | REGEX | "nil" | "true" | "false",
    _name: IDENT,
    _variable: INSTANCE_VAR | CLASS_VAR | GLOBAL_VAR,
    super_call: "super" [arguments],
    yield_expression: "yield" [arguments],
    parenthesized_expression: "(" _expression ")",
    lambda: "->" [parameters] (brace_block | do_block),
    absolute_constant: "::" _constant_path,
    array: "[" [_argument %? ","] "]",
    hash: "{" [_hash_entry %? ","] "}",
    _hash_entry: &(_name ":") label_pair | double_splat_hash | arrow_pair,
    label_pair: _name ":" _expression,
    arrow_pair: _expression@bor "=>" _expression,
    double_splat_hash: "**" _expression,
    arguments: "(" [_argument %? ","] ")",
    _argument:
        splat_argument
        | double_splat_argument
        | block_argument
        | &(_expression@bor "=>") arrow_pair
        | &(_name ":") label_argument
        | _expression,
    splat_argument: "*" _expression,
    double_splat_argument: "**" _expression,
    block_argument: "&" _expression,
    label_argument: _name ":" _expression,
    subscript: "[" [_argument %? ","] "]",

    /// After `.` and `::` every keyword is a method name too: `x.class`.
    _property:
        IDENT | "class" | "then" | "begin" | "end" | "nil" | "self" | "true" | "false"
        | "not" | "and" | "or" | "in" | "do" | "case" | "when" | "if" | "unless" | "while"
        | "return" | "yield" | "def" | "module" | "require" | "super" | "alias" | "undef"
        | "break" | "next" | "rescue" | "ensure" | "for" | "else",
} }

thread_local! {
    /// Whether the next token starts a line. Ruby's keywords are two-sided:
    /// `if` at a line start opens a statement, `if` after an expression on the
    /// same line modifies it. The lexer tracks the line break, so the grammar
    /// sees two different tokens and no rule has to guess.
    static LINE_START: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// One token at the cursor, with the line-start flag the keyword choice needs.
pub fn lex(c: &mut Cursor) -> Kind {
    if c.pos() == 0 {
        LINE_START.with(|flag| flag.set(true));
    }
    let kind = step(c);
    match kind {
        WHITESPACE | COMMENT => {}
        _ => LINE_START.with(|flag| flag.set(kind == SEMI)),
    }
    kind
}

/// The step lexer proper: one token, no bookkeeping.
fn step(c: &mut Cursor) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
            let start = c.pos();
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
            let blank = c.since(start);
            if blank.iter().any(|&byte| byte == b'\n') {
                LINE_START.with(|flag| flag.set(true));
            }
            WHITESPACE
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'#' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        // A backslash at a line end joins the next line: it is whitespace.
        b'\\' if matches!(c.peek_at(1), b'\n' | b'\r') => {
            c.bump_n(2);
            c.eat(b'\n');
            WHITESPACE
        }
        b'=' if c.pos() == 0 || c.behind(1) == b'\n' => {
            if c.starts_with(b"=begin") {
                block_comment(c);
                COMMENT
            } else {
                operator(c)
            }
        }
        b'"' | b'\'' | b'`' => string(c),
        b'%' if !ends_operand(c.previous()) && percent_delimiter(c.peek_at(1)) => percent_literal(c),
        b'<' if c.peek_at(1) == b'<' && !ends_operand(c.previous()) && heredoc_start(c) => heredoc(c),
        b'/' if c.peek_at(1) != b'=' && !ends_operand(c.previous()) => {
            if regex(c) {
                REGEX
            } else {
                operator(c)
            }
        }
        b':' if c.peek_at(1) != b':' && is_symbol_start(c.peek_at(1)) && (!ends_operand(c.previous()) || c.behind(1) == b' ') => symbol(c),
        b'?' if !ends_operand(c.previous()) && is_char_literal(c) => char_literal(c),
        b'@' if is_word_start(c.peek_at(1)) => variable(c),
        b'@' if c.peek_at(1) == b'@' && is_word_start(c.peek_at(2)) => variable(c),
        b'$' if is_special_global(c.peek_at(1)) => variable(c),
        b'$' if is_word_start(c.peek_at(1)) || c.peek_at(1).is_ascii_digit() => variable(c),
        b'[' if c.peek_at(1) == b']' && !ends_operand(c.previous()) => {
            c.bump_n(2);
            INDEX_NAME
        }
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() && !ends_operand(c.previous()) => number(c),
        byte if is_word_start(byte) => word(c),
        _ => operator(c),
    }
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// `?a` is a one-character string literal; `x ? y : z` is a conditional, and
/// the lexer only decides after an operand has ended or not.
fn is_char_literal(c: &Cursor) -> bool {
    let byte = c.peek_at(1);
    byte != 0 && !matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn char_literal(c: &mut Cursor) -> Kind {
    c.bump();
    if c.peek() == b'\\' {
        c.bump_n(2);
    } else {
        c.bump_char();
    }
    STRING
}

/// `$!`, `$;`, `$/`, `$\\`, `$*`, `$&`, `$?`, `$~`, `$0` and friends: the
/// punctuation globals Ruby keeps for itself.
fn is_special_global(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'@' | b';' | b',' | b'/' | b'\\' | b'*' | b'&' | b'?' | b'~' | b'+' | b'=' | b'<' | b'>'
            | b'"' | b'\'' | b'`' | b'.' | b':' | b'-' | b'0'..=b'9'
    )
}

/// `@name`, `@@name` or `$name`: one variable token.
fn variable(c: &mut Cursor) -> Kind {
    let sigil = c.peek();
    c.bump();
    if sigil == b'@' && c.peek() == b'@' {
        c.bump();
        c.eat_while(is_word_byte);
        return CLASS_VAR;
    }
    if sigil == b'$' {
        // `$-w` and the punctuation globals: one sigil plus whatever follows.
        if c.peek() == b'-' {
            c.bump();
            c.eat_while(is_word_byte);
            return GLOBAL_VAR;
        }
        if !is_word_byte(c.peek()) {
            c.bump_char();
            return GLOBAL_VAR;
        }
        c.eat_while(is_word_byte);
        return GLOBAL_VAR;
    }
    c.eat_while(is_word_byte);
    INSTANCE_VAR
}

/// A method name may end with `?` or `!`, as `empty?` and `save!` do. The
/// suffix only joins when nothing that could continue an expression follows,
/// so `x? 1 : 2` is still a conditional.
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let suffix = matches!(c.peek(), b'?' | b'!') && c.peek_at(1) != b'=' && !is_word_byte(c.peek_at(1));
    if suffix {
        c.bump();
        return IDENT;
    }
    keyword(&start[..length])
}

/// The modifier form of a keyword, or the plain one at a line start.
fn mundane(kind: Kind, modifier: Kind) -> Kind {
    if LINE_START.with(|flag| flag.get()) { kind } else { modifier }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"if" => mundane(KW_IF, MOD_IF),
        b"unless" => mundane(KW_UNLESS, MOD_UNLESS),
        b"while" => mundane(KW_WHILE, MOD_WHILE),
        b"until" => mundane(KW_UNTIL, MOD_UNTIL),
        b"rescue" => mundane(KW_RESCUE, MOD_RESCUE),
        b"alias" => KW_ALIAS,
        b"begin" => KW_BEGIN,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"class" => KW_CLASS,
        b"def" => KW_DEF,
        b"do" => KW_DO,
        b"else" => KW_ELSE,
        b"elsif" => KW_ELSIF,
        b"end" => KW_END,
        b"ensure" => KW_ENSURE,
        b"false" => KW_FALSE,
        b"for" => KW_FOR,
        b"in" => KW_IN,
        b"module" => KW_MODULE,
        b"next" => KW_NEXT,
        b"nil" => KW_NIL,
        b"return" => KW_RETURN,
        b"self" => KW_SELF,
        b"super" => KW_SUPER,
        b"then" => KW_THEN,
        b"true" => KW_TRUE,
        b"undef" => KW_UNDEF,
        b"when" => KW_WHEN,
        b"yield" => KW_YIELD,
        b"and" => KW_AND,
        b"or" => KW_OR,
        b"not" => KW_NOT,
        b"require" => KW_REQUIRE,
        b"require_relative" => KW_REQUIRE_RELATIVE,
        b"load" => KW_LOAD,
        b"include" => KW_INCLUDE,
        b"extend" => KW_EXTEND,
        b"prepend" => KW_PREPEND,
        _ => IDENT,
    }
}

/// Whether a token of this kind can end an operand, so that `/` divides, `<<`
/// shifts, `%` is a modulo and `:x` cannot start a symbol.
fn ends_operand(kind: Kind) -> bool {
    matches!(
        kind,
        IDENT | NUMBER | STRING | SYMBOL | REGEX | INDEX_NAME | RPAREN | RBRACKET | RBRACE | KW_SELF
            | KW_NIL | KW_TRUE | KW_FALSE | KW_END
    )
}

/// `=begin ... =end` at the start of a line.
fn block_comment(c: &mut Cursor) {
    while !c.at_end() {
        if c.starts_with(b"=end") {
            c.bump_n(4);
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            return;
        }
        c.eat_while(|byte| byte != b'\n');
        c.eat(b'\n');
    }
}

/// A string from its quote, with `#{...}` scanned whole in a double-quoted one.
fn string(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    c.bump();
    loop {
        match c.peek() {
            0 if c.at_end() => return BAD_LITERAL,
            b'\\' => c.bump_n(2),
            b'\n' | b'\r' if quote == b'\'' || quote == b'`' => return BAD_LITERAL,
            b'#' if quote == b'"' && c.peek_at(1) == b'{' => {
                c.bump_n(2);
                braces(c);
            }
            byte if byte == quote => {
                c.bump();
                return STRING;
            }
            _ => c.bump(),
        }
    }
}

/// The code of a `#{...}` field, up to its matching `}`, with strings and
/// nested braces balanced. Never fails: an unclosed field runs to the end.
fn braces(c: &mut Cursor) {
    let mut depth = 1u32;
    while !c.at_end() {
        match c.peek() {
            b'{' => {
                depth += 1;
                c.bump();
            }
            b'}' => {
                depth -= 1;
                c.bump();
                if depth == 0 {
                    return;
                }
            }
            b'"' | b'\'' | b'`' => {
                string(c);
            }
            b'#' if c.peek_at(1) == b'{' => {
                c.bump_n(2);
                braces(c);
            }
            _ => c.bump(),
        }
    }
}

fn percent_delimiter(byte: u8) -> bool {
    matches!(byte, b'q' | b'Q' | b'w' | b'W' | b'i' | b'I' | b'r' | b's' | b'x')
}

/// `%q(...)`, `%Q{...}`, `%w[...]` and `%r{...}`: the opening delimiter pairs
/// with its close, and bracket delimiters nest.
fn percent_literal(c: &mut Cursor) -> Kind {
    c.bump();
    let letter = c.peek().to_ascii_lowercase();
    c.bump();
    let open = c.peek();
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        other => other,
    };
    if !open.is_ascii() || open.is_ascii_alphanumeric() || open == b' ' || open == b'\n' {
        return BAD_LITERAL;
    }
    c.bump();
    let nested = matches!(open, b'(' | b'[' | b'{' | b'<');
    let mut depth = 1u32;
    loop {
        match c.peek() {
            0 if c.at_end() => return BAD_LITERAL,
            b'\\' => c.bump_n(2),
            byte if byte == open && nested => {
                depth += 1;
                c.bump();
            }
            byte if byte == close => {
                depth -= 1;
                c.bump();
                if depth == 0 {
                    if letter == b'r' {
                        c.eat_while(|byte| byte.is_ascii_alphabetic());
                    }
                    return STRING;
                }
            }
            _ => c.bump(),
        }
    }
}

/// Whether `<<` starts a heredoc: `<<EOS`, `<<~EOS`, `<<-EOS` or a quoted
/// delimiter, never `<< b`.
fn heredoc_start(c: &Cursor) -> bool {
    let mut at = 2;
    if matches!(c.peek_at(at), b'~' | b'-') {
        at += 1;
    }
    is_word_start(c.peek_at(at)) || matches!(c.peek_at(at), b'"' | b'\'')
}

/// A heredoc from `<<` to its terminator line. `~` and `-` allow the
/// terminator to be indented; a plain one must start the line.
fn heredoc(c: &mut Cursor) -> Kind {
    c.bump_n(2);
    let indented = matches!(c.peek(), b'~' | b'-');
    if indented {
        c.bump();
    }
    let quote = c.peek();
    let quoted = matches!(quote, b'"' | b'\'');
    if quoted {
        c.bump();
    }
    let rest = c.rest();
    let delimiters = c.eat_while(is_word_byte);
    let delimiter = &rest[..delimiters];
    if quoted {
        c.eat(quote);
    }
    if delimiter.is_empty() {
        return BAD_LITERAL;
    }
    // The rest of the opening line, then the lines of the body.
    c.eat_while(|byte| byte != b'\n');
    if c.at_end() {
        return BAD_LITERAL;
    }
    c.eat(b'\n');
    while !c.at_end() {
        let line_start = c.pos();
        c.eat_while(|byte| byte != b'\n');
        let mut line = c.since(line_start);
        if indented {
            line = trim_start(line);
        }
        let line = trim_end(line);
        let closed = line == delimiter;
        if c.at_end() {
            return if closed { STRING } else { BAD_LITERAL };
        }
        c.eat(b'\n');
        if closed {
            return STRING;
        }
    }
    BAD_LITERAL
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = bytes {
        if matches!(first, b' ' | b'\t') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while let [rest @ .., last] = bytes {
        if matches!(last, b' ' | b'\t' | b'\r') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

/// Whether a `:` starts a symbol: a name right after it, or one of the operator
/// symbols Ruby allows (`:+`, `:[]`, `:<=>`).
fn is_symbol_start(byte: u8) -> bool {
    is_word_start(byte) || matches!(byte, b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'~' | b'&' | b'|' | b'^' | b'[' | b'@' | b'$')
}

fn symbol(c: &mut Cursor) -> Kind {
    c.bump();
    // `:@name` and `:$name`: a symbol for a variable.
    if matches!(c.peek(), b'@' | b'$') {
        variable(c);
        return SYMBOL;
    }
    if is_word_start(c.peek()) {
        c.eat_while(is_word_byte);
        if matches!(c.peek(), b'?' | b'!' | b'=') && c.peek_at(1) != b'=' && !is_word_byte(c.peek_at(1)) {
            c.bump();
        }
        return SYMBOL;
    }
    // A one- or two-character operator symbol.
    c.bump_n(operator_width(c));
    SYMBOL
}

fn operator_width(c: &Cursor) -> usize {
    let (b, d) = (c.peek_at(1), c.peek_at(2));
    match (c.peek(), b, d) {
        (b'<', b'=', b'>') => 3,
        (b'=', b'=', b'=') => 3,
        (b'=', b'=', _) | (b'!', b'=', _) | (b'<', b'=', _) | (b'>', b'=', _) | (b'<', b'<', _) | (b'>', b'>', _) => 2,
        _ => 1,
    }
}

/// A regular expression from its `/`, to its closing `/` plus flags. False if
/// the line ends first, so a division is not swallowed.
fn regex(c: &mut Cursor) -> bool {
    c.bump();
    let mut class = false;
    loop {
        match c.peek() {
            0 if c.at_end() => return false,
            b'\n' | b'\r' => return false,
            b'\\' => c.bump_n(2),
            b'[' => {
                class = true;
                c.bump();
            }
            b']' => {
                class = false;
                c.bump();
            }
            b'/' if !class => {
                c.bump();
                c.eat_while(|byte| byte.is_ascii_alphabetic());
                return true;
            }
            _ => c.bump(),
        }
    }
}

fn number(c: &mut Cursor) -> Kind {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return NUMBER;
    }
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.peek() == b'.' && c.peek_at(1).is_ascii_digit() {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'e' | b'E') && (c.peek_at(1).is_ascii_digit() || (matches!(c.peek_at(1), b'+' | b'-') && c.peek_at(2).is_ascii_digit())) {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'r' | b'i') {
        c.bump();
    }
    NUMBER
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'&', b'&', b'=') => (ANDAND_EQ, 3),
        (b'|', b'|', b'=') => (OROR_EQ, 3),
        (b'*', b'*', b'=') => (STARSTAR_EQ, 3),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'<', b'=', b'>') => (CMP, 3),
        (b'=', b'=', b'=') => (TEQ, 3),
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'*', b'*', _) => (STARSTAR, 2),
        (b'<', b'<', _) => (SHL, 2),
        (b'>', b'>', _) => (SHR, 2),
        (b'&', b'.', _) => (SAFE_DOT, 2),
        (b'=', b'~', _) => (MATCH, 2),
        (b'!', b'~', _) => (NMATCH, 2),
        (b'=', b'>', _) => (FATARROW, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b':', b':', _) => (COLONCOLON, 2),
        (b'.', b'.', _) => (DOTDOT, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'(', ..) => (LPAREN, 1),
        (b')', ..) => (RPAREN, 1),
        (b'[', ..) => (LBRACKET, 1),
        (b']', ..) => (RBRACKET, 1),
        (b'{', ..) => (LBRACE, 1),
        (b'}', ..) => (RBRACE, 1),
        (b';', ..) => (SEMI, 1),
        (b',', ..) => (COMMA, 1),
        (b'.', ..) => (DOT, 1),
        (b':', ..) => (COLON, 1),
        (b'?', ..) => (QUESTION, 1),
        (b'=', ..) => (ASSIGN, 1),
        (b'+', ..) => (PLUS, 1),
        (b'-', ..) => (MINUS, 1),
        (b'*', ..) => (STAR, 1),
        (b'/', ..) => (SLASH, 1),
        (b'%', ..) => (PERCENT, 1),
        (b'|', ..) => (PIPE, 1),
        (b'&', ..) => (AMP, 1),
        (b'^', ..) => (CARET, 1),
        (b'~', ..) => (TILDE, 1),
        (b'!', ..) => (BANG, 1),
        (b'<', ..) => (LT, 1),
        (b'>', ..) => (GT, 1),
        _ => {
            c.bump_char();
            return UNKNOWN;
        }
    };
    c.bump_n(width);
    kind
}
