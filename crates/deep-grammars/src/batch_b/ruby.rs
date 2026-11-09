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
//!   with optional clauses, and a block is its own `begin` (`do ... rescue ...
//!   end`).
//! - **Expression statements do the assignment.** Assignment is the loosest
//!   row of one precedence table and postfix calls, members, indexes and
//!   blocks are the tightest, so `a = b = 1`, `a += 1`, `x.y = 1` and
//!   `xs.each { |x| ... }` all parse without special statement forms.
//!   Multiple assignment (`a, b = 1, 2`, `target, *rest = xs`, `encoding, =`)
//!   is its own statement rule, because its left side is a list.
//! - **Blocks are postfix operators.** `xs.each { |x| ... }` and `xs.each do
//!   |x| ... end` are `call_with_block` rows, because a `{` after an operand
//!   is a block and at the start of a statement it is a hash.
//! - **A call is `arguments`; a command call is a statement.** `foo(1, a: 2)`
//!   is a `call` node. A command call (`raise ArgumentError, "no"`,
//!   `FileUtils.mv a, b, **opts`) is also a `call`, taken when a comma proves
//!   the list is arguments. A single literal argument (`raise "no"`,
//!   `foo :sym`, `rand 1000`) is a postfix row, so `print ' ' * w` stays a
//!   product of a call and `w`. A bare name argument (`puts x`) is still two
//!   statements: no token tells "a name, then a statement" from "a name, then
//!   an argument".
//! - **A string's fields are code.** A double-quoted string or a backtick is
//!   an `interpolated_string` over `string_start`, its text and one
//!   `interpolation` per `#{...}` field, and a field is ordinary code again --
//!   so a call written in a field is an ordinary `call` node, a brace closes
//!   at its own depth, and a nested string opens its own frames. `%q(...)`,
//!   `%w[...]` and heredocs stay one token each.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **Line breaks are lexer state.** Ruby decides by the line break alone
//!   whether `if` opens a statement or modifies one, and whether a `(` after a
//!   method name is a parameter list or the body's own parenthesis. The step
//!   lexer keeps a line-start flag and a def-header flag and hands the parser
//!   two different tokens, so no grammar rule has to guess: `if` after an
//!   operand is `if_modifier`, at a line start it is `if`, and after `=` it is
//!   `if_value` (the same statement shape, used as a value).
//! - **Heredocs are a lexer state.** The terminator is a line that equals the
//!   delimiter, and it arrives after the token that started the heredoc. The
//!   step lexer scans `<<~EOS` to its terminator, deciding heredoc against
//!   `<<` from the delimiter's shape and from [`Cursor::previous`]: after an
//!   operand it is a heredoc only when the rest of the line is blank
//!   (`abort <<MESSAGE`), and otherwise it shifts.
//! - **Regex against division.** `/` starts a regex where an operand cannot
//!   end or a line starts, and divides elsewhere. A regex whose closing slash
//!   exists anywhere in the file may hold line breaks, which is what an
//!   extended `/x` pattern needs and what keeps a division from running away.
//!   One regex is one token; its groups are not a tree.
//! - **A symbol and a label share `:`.** `:name` is one `symbol` token, and
//!   `key: value` is a label. The lexer tells them apart by the space before
//!   the colon, so `attr_reader :name` is a symbol while `{a: 1}` is a label.
//! - **A constant is its own token.** The lexer separates `Foo` from `foo`,
//!   which is what makes `rescue Foo` an exception list and `rescue` followed
//!   by a body starting with a name an empty list.
//! - **A decision sees three tokens.** `&(...)` predicates start the
//!   alternatives that share a prefix: a hash/argument label (`&(_name ":")`)
//!   against an expression, and a multiple assignment or command call against
//!   a plain expression statement.
//! - **One kind per rule**, so `if` and `unless` are two rules, not one
//!   labelled alternative, and an operator row names a kind no rule already
//!   owns.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { RUBY {
    name = "ruby";
    lex = lex;

    /// A double-quoted string or a backtick is text with `#{...}` fields, and a
    /// field is code again. `BODY` is the text between fields; `FIELD` is the
    /// code inside one, and it ends at the `}` that matches its own `#{` at the
    /// field's bracket depth, so `"#{ {a: 1}[:a] }"` closes at its own brace
    /// and a nested string opens its own frames.
    modes {
        BODY = lex_string_body,
        FIELD = lex_interpolation,
    }
    switch STRING_START => push BODY;
    switch STRING_END => pop;
    switch BAD_STRING => pop BODY;
    switch INTERPOLATION_START => push FIELD;
    switch INTERPOLATION_END => pop;

    tokens {
        LPAREN = "(" open parens,
        /// The `(` of a `def` header: a `(` on the def's own line and after its
        /// name is the parameter list, while one on a later line belongs to the
        /// body. Ruby decides this question by the line break, and so does the
        /// lexer, which keeps the two apart as two tokens.
        PARAMETERS_OPEN = "parameters_paren",
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        /// A name that starts uppercase: a constant, and the only thing that can
        /// name an exception in `rescue Foo`. The lexer tells the two apart, so
        /// `rescue` followed by `raise` is a body, not an exception list.
        CONST = "constant",
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
        /// The opening quote of a string whose `#{...}` fields are code.
        STRING_START = "string_start",
        /// The bytes between two fields.
        STRING_CONTENT = "string_content",
        /// The `#{` that opens a field: code again until its own `}`.
        INTERPOLATION_START = "#{" open interpolation,
        /// The `}` that closes a field at the field's own bracket depth.
        INTERPOLATION_END = "interpolation_end" close interpolation,
        /// The closing quote of such a string.
        STRING_END = "string_end",
        /// A string that cannot close before the end of the file.
        BAD_STRING = "bad_string" bad,

        KW_ALIAS = "alias",
        KW_BEGIN = "begin",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CLASS = "class",
        KW_DEF = "def",
        KW_DO = "do",
        /// `defined?` is a keyword operator, not a method called `defined`.
        KW_DEFINED = "defined?",
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
        /// `if` where a value is wanted: `x = if cond ... end`, `f(if a then b end)`.
        IF_VALUE = "if_value",
        MOD_UNLESS = "unless_modifier",
        MOD_WHILE = "while_modifier",
        MOD_UNTIL = "until_modifier",
        MOD_RESCUE = "rescue_modifier",
        /// `begin ... end` as a value: `x = begin ... end`.
        MOD_BEGIN = "begin_modifier",
        /// `or return`, `and next`: a jump in operand position.
        MOD_RETURN = "return_modifier",
        MOD_BREAK = "break_modifier",
        MOD_NEXT = "next_modifier",
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
        /// `load` only where a statement starts: `x = load(...)` and
        /// `load.setup` are method calls, and `load "f.rb"` is a require.
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
        &(_left_target ",") multiple_assignment
        | &("*" _name "=") splat_assignment
        | &(_callable _command_argument ",") command_call
        | require_statement
        | alias_statement
        | undef_statement
        | include_statement
        | singleton_class_definition
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
    _alias_target: SYMBOL | GLOBAL_VAR | INSTANCE_VAR | setter_name | _method_ident,
    undef_statement: "undef" (_alias_target % ","),
    include_statement: ("include" | "extend" | "prepend") (_constant_path | parenthesized_expression) ("," _constant_path)*,

    /// `class << self`: reopening the singleton class.
    singleton_class_definition: "class" "<<" _expression body "end" _modifier*,
    module_definition: "module" _constant_path body "end" _modifier*,
    class_definition: "class" _constant_path ["<" _expression] body "end" _modifier*,
    method_definition: "def" method_name [parameters] body rescue_clause* [else_clause] [ensure_clause] "end" _modifier*,
    /// `def foo`, `def self.foo` and `def IRB.start`: the receiver is taken only
    /// when a dot really follows it, so a plain `def foo` never reads the body
    /// as a receiver.
    method_name:
        "self" ("." | "::") _method_name_tail
        | &(_name ("." | "::")) _name ("." | "::") _method_name_tail
        | _method_name_tail,
    _method_name_tail: setter_name | index_method_name | _method_ident,
    setter_name: IDENT "=",
    index_method_name: "[]" ["="],
    /// `def +(other)`, `def <=>(other)`, `def ~`: an operator is a method name.
    _method_ident: IDENT | CONST | "~" | "+" | "-" | "*" | "/" | "%" | "**" | "==" | "===" | "!=" | "!"
        | "<" | ">" | "<=" | ">=" | "<=>" | "=~" | "!~" | "<<" | ">>" | "&" | "|" | "^",
    parameters: PARAMETERS_OPEN [_parameter_list] ")",
    _parameter_list: _parameter %? ",",
    _parameter:
        double_splat_parameter
        | splat_parameter
        | block_parameter
        | destructured_parameter
        | keyword_parameter
        | _name ["=" _expression@assign],
    splat_parameter: "*" [_name],
    double_splat_parameter: "**" [_name | "nil"],
    block_parameter: "&" _name,
    keyword_parameter: _name ":" [_expression],
    destructured_parameter: "(" [_parameter_list] ")",
    _constant_path: CONST ("::" _name)*,

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

    return_statement: "return" [_return_arguments] _modifier*,
    _return_arguments: (_return_argument ("," _return_argument)*),
    _return_argument: splat_argument | _expression,
    break_statement: "break" _modifier*,
    next_statement: "next" _modifier*,
    expression_statement: _expression _modifier*,

    /// `a, b = 1, 2`, `a, *rest = xs`, `encoding, = split(:)`: one `=` with a
    /// list on each side. The comma in the left list is what tells it apart
    /// from a plain assignment, so the two rules cannot both match.
    multiple_assignment: _left_list "=" _right_list,
    splat_assignment: splat_argument "=" _right_list,
    _right_list: (splat_argument | _expression) ("," (splat_argument | _expression))*,
    splat_argument: "*" _expression,
    _left_list: _left_target ("," [_left_target | splat_target])+,
    splat_target: "*" [_left_target],
    _left_target: precedence _primary {
        index: postfix subscript => element_reference,
        member: postfix "." _property => member,
        safe_member: postfix "&." _property => member,
        scope: postfix "::" _property => scope,
    },

    /// A call without parentheses: `raise ArgumentError, "no"`, `load(a, **opts)`.
    /// It takes the comma list as its arguments, which is what tells it apart
    /// from two statements on one line; a single argument stays an ordinary
    /// expression statement, which costs the tree a `call` node and no error.
    command_call: _callable _command_arguments _modifier*,
    /// A call with one literal argument and no comma: `raise "no"`, `foo :sym`.
    _bare_argument: STRING | interpolated_string | SYMBOL | NUMBER,
    _callable: _name ("." _property | "::" _property)*,
    _command_arguments: _command_argument ("," _command_argument)+,
    _command_argument:
        double_splat_argument
        | splat_argument
        | block_argument
        | &(_expression@bor "=>") arrow_pair
        | &(_name ":") label_argument
        | _expression,

    /// What follows a statement on its own line: `x if y`, `x while y`,
    /// `value rescue fallback`.
    _modifier: if_modifier | unless_modifier | while_modifier | until_modifier | rescue_modifier,
    if_modifier: "if_modifier" _expression,
    unless_modifier: "unless_modifier" _expression,
    while_modifier: "while_modifier" _expression,
    until_modifier: "until_modifier" _expression,
    rescue_modifier: "rescue_modifier" _expression,

    // ---- blocks ----

    brace_block: "{" [block_parameters] _block_body "}",
    do_block: "do" [block_parameters] _block_body "end",
    /// A block is its own `begin`: `do ... rescue ... ensure ... end`.
    _block_body: _statement* rescue_clause* [else_clause] [ensure_clause],
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
        call_through: postfix "." arguments => call,
        bare_call: postfix _bare_argument => call,
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
        | interpolated_string
        | conditional_value
        | begin_value
        | jump_expression
        | defined_expression
        | "[]",

    _literal: NUMBER | STRING | SYMBOL | REGEX | "nil" | "true" | "false",
    /// `x = begin ... end`: the same clauses as `begin`, in operand position.
    begin_value: "begin_modifier" body rescue_clause* [else_clause] [ensure_clause] "end",
    /// `block_given? or return enum_for(...)`: a jump where a value is wanted.
    jump_expression: "return_modifier" [_return_arguments] | "break_modifier" | "next_modifier",
    /// `defined?(x)` and `defined? x`, which are keywords in Ruby.
    defined_expression: "defined?" [defined_parentheses] [_expression],
    defined_parentheses: "(" _expression ")",
    /// `x = if cond then 1 else 2 end`: an `if` that is not at a line start is
    /// a value, and its `end` closes it rather than a statement.
    conditional_value: "if_value" _expression [then_token] body elsif_clause* [else_clause] "end",
    /// A double-quoted string or a backtick: its fields are expressions.
    interpolated_string: STRING_START _string_body STRING_END,
    _string_body: (STRING_CONTENT | interpolation)*,
    interpolation: "#{" _expression _modifier* INTERPOLATION_END,
    _name: IDENT | CONST,
    _variable: INSTANCE_VAR | CLASS_VAR | GLOBAL_VAR,
    super_call: "super" [arguments],
    yield_expression: "yield" [arguments],
    parenthesized_expression: "(" _expression _modifier* ")",
    lambda: "->" [lambda_parameters | parameters] (brace_block | do_block),
    lambda_parameters: "(" [_parameter_list] ")",
    absolute_constant: "::" _constant_path,
    array: "[" [_argument %? ","] "]",
    hash: "{" [_hash_entry %? ","] "}",
    _hash_entry: &(_name ":") label_pair | double_splat_hash | arrow_pair,
    label_pair: _name ":" _expression,
    arrow_pair: _expression@bor "=>" _expression,
    double_splat_hash: "**" _expression,
    arguments: "(" [_argument %? ","] ")",
    _argument:
        double_splat_argument
        | splat_argument
        | block_argument
        | &(_expression@bor "=>") arrow_pair
        | &(_name ":") label_argument
        | _expression,
    double_splat_argument: "**" _expression,
    block_argument: "&" _expression,
    label_argument: _name ":" _expression,
    subscript: "[" [_argument %? ","] "]",

    /// After `.` and `::` every keyword is a method name too: `x.class`.
    _property:
        IDENT | CONST | "class" | "then" | "begin" | "end" | "nil" | "self" | "true" | "false"
        | "not" | "and" | "or" | "in" | "do" | "case" | "when" | "if" | "unless" | "while"
        | "return" | "yield" | "def" | "module" | "require" | "super" | "alias" | "undef"
        | "break" | "next" | "rescue" | "ensure" | "for" | "else" | "elsif"
        | "begin_modifier" | "return_modifier" | "break_modifier" | "next_modifier"
        | "if_modifier" | "unless_modifier" | "while_modifier" | "until_modifier" | "rescue_modifier"
        | "==" | "===" | "!=" | "=~" | "!~" | "<=>" | "<" | ">" | "<=" | ">=" | "+" | "-"
        | "*" | "/" | "%" | "**" | "<<" | ">>" | "&" | "|" | "^" | "~" | "!" | "[]",
} }

thread_local! {
    /// Whether the next token starts a line. Ruby's keywords are two-sided:
    /// `if` at a line start opens a statement, `if` after an expression on the
    /// same line modifies it. The lexer tracks the line break, so the grammar
    /// sees two different tokens and no rule has to guess.
    static LINE_START: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Whether the tokens so far belong to a `def` header on one line, which
    /// is what makes a `(` the parameter list rather than a body's.
    static IN_DEF_HEADER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One token at the cursor, with the line-start flag the keyword choice needs.
pub fn lex(c: &mut Cursor) -> Kind {
    if c.pos() == 0 {
        LINE_START.with(|flag| flag.set(true));
    }
    let mut kind = step(c);
    let line_start = LINE_START.with(|flag| flag.get());
    match kind {
        WHITESPACE | COMMENT => return kind,
        KW_DEF => IN_DEF_HEADER.with(|flag| flag.set(true)),
        LPAREN if IN_DEF_HEADER.with(|flag| flag.get()) && !line_start => {
            IN_DEF_HEADER.with(|flag| flag.set(false));
            kind = PARAMETERS_OPEN;
        }
        SEMI => IN_DEF_HEADER.with(|flag| flag.set(false)),
        _ => {
            // The header lives only on the `def` line: a token that starts a
            // later line belongs to the body.
            if line_start {
                IN_DEF_HEADER.with(|flag| flag.set(false));
            }
        }
    }
    // `if` has three positions: a statement at a line start, a modifier after
    // an operand, and a value after `=`, `(` or `,`. `return if x` is the
    // modifier, so a jump keyword counts as the end of an operand here.
    if kind == MOD_IF && !ends_operand(c.previous()) && !matches!(c.previous(), KW_RETURN | KW_BREAK | KW_NEXT) {
        kind = IF_VALUE;
    }
    LINE_START.with(|flag| flag.set(kind == SEMI));
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
                // A def header never outlives its line.
                IN_DEF_HEADER.with(|flag| flag.set(false));
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
        b'"' | b'`' => string_start(c),
        b'\'' => string(c),
        b'%' if !after_operand(c) && (percent_delimiter(c.peek_at(1)) || percent_open(c.peek_at(1))) => percent_literal(c),
        b'<' if c.peek_at(1) == b'<' && heredoc_start(c) && (!after_operand(c) || heredoc_tail_blank(c)) => heredoc(c),
        b'/' if !after_operand(c) => {
            if regex(c) {
                REGEX
            } else {
                operator(c)
            }
        }
        b':' if c.peek_at(1) != b':' && is_symbol_start(c.peek_at(1)) && (!after_operand(c) || c.behind(1) == b' ') => symbol(c),
        b'?' if !after_operand(c) && is_char_literal(c) => char_literal(c),
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
        b'!' | b'@' | b'$' | b';' | b',' | b'/' | b'\\' | b'*' | b'&' | b'?' | b'~' | b'+' | b'=' | b'<' | b'>'
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
    let text = &start[..length];
    if text == b"defined" && c.peek() == b'?' {
        c.bump();
        return KW_DEFINED;
    }
    let suffix = matches!(c.peek(), b'?' | b'!') && c.peek_at(1) != b'=' && !is_word_byte(c.peek_at(1));
    if suffix {
        c.bump();
        return IDENT;
    }
    let kind = keyword(text);
    // `load.setup` and `load: x` are not the statement keyword.
    if kind == KW_LOAD && matches!(c.peek(), b'.' | b':') {
        return IDENT;
    }
    match kind {
        IDENT if text.first().is_some_and(|byte| byte.is_ascii_uppercase()) => CONST,
        _ => kind,
    }
}

/// The modifier form of a keyword, or the plain one at a line start.
fn mundane(kind: Kind, modifier: Kind) -> Kind {
    if LINE_START.with(|flag| flag.get()) { kind } else { modifier }
}

fn keyword(text: &[u8]) -> Kind {
    // `def load`, `def require`: after `def` every word is a method name.
    if IN_DEF_HEADER.with(|flag| flag.get()) {
        return IDENT;
    }
    match text {
        b"if" => mundane(KW_IF, MOD_IF),
        b"unless" => mundane(KW_UNLESS, MOD_UNLESS),
        b"while" => mundane(KW_WHILE, MOD_WHILE),
        b"until" => mundane(KW_UNTIL, MOD_UNTIL),
        b"rescue" => mundane(KW_RESCUE, MOD_RESCUE),
        b"begin" => mundane(KW_BEGIN, MOD_BEGIN),
        b"return" => mundane(KW_RETURN, MOD_RETURN),
        b"break" => mundane(KW_BREAK, MOD_BREAK),
        b"next" => mundane(KW_NEXT, MOD_NEXT),
        b"alias" => KW_ALIAS,
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
        b"nil" => KW_NIL,
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
        b"load" => mundane(KW_LOAD, IDENT),
        b"require" => KW_REQUIRE,
        b"require_relative" => KW_REQUIRE_RELATIVE,
        b"include" => KW_INCLUDE,
        b"extend" => KW_EXTEND,
        b"prepend" => KW_PREPEND,
        _ => IDENT,
    }
}

/// Whether the cursor sits right after an operand on the same line: there a
/// `/` divides, a `%` is a modulo, a `:` is a label and a `<<` shifts. A line
/// break starts a fresh statement, so none of those hold there.
fn after_operand(c: &Cursor) -> bool {
    ends_operand(c.previous()) && !LINE_START.with(|flag| flag.get())
}

/// Whether the heredoc's opening line ends right after its delimiter, which is
/// what `abort <<MESSAGE` looks like; a shift takes an operand after it.
fn heredoc_tail_blank(c: &Cursor) -> bool {
    let mut at = 2;
    if matches!(c.peek_at(at), b'~' | b'-') {
        at += 1;
    }
    let quote = c.peek_at(at);
    if matches!(quote, b'"' | b'\'') {
        at += 1;
    }
    while is_word_byte(c.peek_at(at)) {
        at += 1;
    }
    if matches!(quote, b'"' | b'\'') && c.peek_at(at) == quote {
        at += 1;
    }
    let rest = c.rest();
    let tail = &rest[at.min(rest.len())..];
    tail.iter().take_while(|&&byte| byte != b'\n').all(|&byte| matches!(byte, b' ' | b'\t' | b'\r'))
}

/// Whether a token of this kind can end an operand, so that `/` divides, `<<`
/// shifts, `%` is a modulo and `:x` cannot start a symbol.
fn ends_operand(kind: Kind) -> bool {
    matches!(
        kind,
        IDENT | CONST | NUMBER | STRING | SYMBOL | REGEX | INDEX_NAME | RPAREN | RBRACKET | RBRACE
            | STRING_END | INTERPOLATION_END | KW_SELF | KW_NIL | KW_TRUE | KW_FALSE | KW_END
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

thread_local! {
    /// The quotes of every string body the `BODY` mode is inside, innermost
    /// last: a nested string in a field restores the outer one on the way out.
    static STRING_QUOTES: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    /// One frame per open `#{...}` field: the brackets opened inside it and how
    /// many string bodies were open when it began, so a broken string abandons
    /// every field it opened.
    static FIELDS: std::cell::RefCell<Vec<Field>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What one open interpolation field holds.
#[derive(Clone, Copy)]
struct Field {
    /// Openers minus closers inside the field.
    depth: u32,
    /// The string the field was opened in, as the quote stack's depth.
    string: usize,
}

/// A single-quoted string: no fields, one token.
fn string(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    c.bump();
    loop {
        match c.peek() {
            0 if c.at_end() => return BAD_LITERAL,
            b'\\' => c.bump_n(2),
            b'\n' | b'\r' if quote == b'\'' => return BAD_LITERAL,
            byte if byte == quote => {
                c.bump();
                return STRING;
            }
            _ => c.bump(),
        }
    }
}

/// Whether a string that opens here has a closing quote anywhere before the
/// end of the file. A quote that does not is lexed as one bad token at its
/// line, so a broken string costs its line and the rest of the file still
/// parses; a real multi-line string closes and takes the mode path.
fn closes_later(c: &Cursor, quote: u8) -> bool {
    let rest = c.rest();
    let mut at = 1;
    while at < rest.len() {
        match rest[at] {
            b'\\' => at += 2,
            byte if byte == quote => return true,
            _ => at += 1,
        }
    }
    false
}

/// `"` or a backtick: the body is text with fields, so the mode stack takes
/// over until the closing quote.
fn string_start(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    if !closes_later(c, quote) {
        c.bump();
        c.eat_while(|byte| byte != b'\n' && byte != b'\r');
        return BAD_LITERAL;
    }
    c.bump();
    STRING_QUOTES.with(|quotes| quotes.borrow_mut().push(quote));
    STRING_START
}

fn pop_string() {
    STRING_QUOTES.with(|quotes| {
        quotes.borrow_mut().pop();
    });
}

fn open_field() {
    let string = STRING_QUOTES.with(|quotes| quotes.borrow().len());
    FIELDS.with(|fields| fields.borrow_mut().push(Field { depth: 0, string }));
}

fn close_field() {
    FIELDS.with(|fields| {
        fields.borrow_mut().pop();
    });
}

/// A string that cannot close ends here: the fields it opened lose their
/// frames and the mode stack pops through `BODY` on the one bad token.
fn abandon_string() {
    let string = STRING_QUOTES.with(|quotes| quotes.borrow().len());
    FIELDS.with(|fields| {
        let mut fields = fields.borrow_mut();
        while fields.last().is_some_and(|field| field.string >= string) {
            fields.pop();
        }
    });
    pop_string();
}

fn with_field(change: impl FnOnce(&mut Field)) {
    FIELDS.with(|fields| {
        if let Some(field) = fields.borrow_mut().last_mut() {
            change(field);
        }
    });
}

/// The `BODY` mode: the text of a string, with one `#{` opening a field and the
/// quote closing the string. Never fails on an unbalanced field: the field's
/// own `}` is the one at its bracket depth.
pub fn lex_string_body(c: &mut Cursor) -> Kind {
    if c.at_end() {
        abandon_string();
        return BAD_STRING;
    }
    let Some(quote) = STRING_QUOTES.with(|quotes| quotes.borrow().last().copied()) else {
        // The mode stack and this state disagree: lex code, which cannot lose
        // a token, and let the parser recover.
        return step(c);
    };
    if c.peek() == quote {
        c.bump();
        pop_string();
        return STRING_END;
    }
    if c.peek() == b'#' && c.peek_at(1) == b'{' {
        c.bump_n(2);
        open_field();
        return INTERPOLATION_START;
    }
    loop {
        if c.at_end() {
            return STRING_CONTENT;
        }
        let byte = c.peek();
        if byte == quote || (byte == b'#' && c.peek_at(1) == b'{') {
            return STRING_CONTENT;
        }
        if byte == b'\\' {
            c.bump_n(2);
        } else {
            c.bump();
        }
    }
}

/// The `FIELD` mode: ordinary code, except that a `}` at the field's own depth
/// closes it. Deeper, a brace belongs to a hash or a block and is `lex`'s.
pub fn lex_interpolation(c: &mut Cursor) -> Kind {
    let Some(field) = FIELDS.with(|fields| fields.borrow().last().copied()) else {
        return step(c);
    };
    if c.at_end() {
        abandon_string();
        return BAD_STRING;
    }
    if field.depth == 0 && c.peek() == b'}' {
        c.bump();
        close_field();
        return INTERPOLATION_END;
    }
    let kind = lex(c);
    match kind {
        LPAREN | LBRACKET | LBRACE => with_field(|field| field.depth += 1),
        RPAREN | RBRACKET | RBRACE => with_field(|field| field.depth = field.depth.saturating_sub(1)),
        _ => {}
    }
    kind
}

/// A quoted name from its quote to its closing quote, with `#{...}` fields and
/// nested strings skipped: a symbol's name is one token, not a subtree.
fn skip_quoted(c: &mut Cursor, quote: u8) {
    c.bump();
    let mut depth = 0u32;
    while !c.at_end() {
        match c.peek() {
            b'\\' => c.bump_n(2),
            b'#' if c.peek_at(1) == b'{' => {
                c.bump_n(2);
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                c.bump();
            }
            byte if byte == quote && depth == 0 => {
                c.bump();
                return;
            }
            _ => c.bump(),
        }
    }
}

/// Whether a byte can open a `%` literal directly: `%{`, `%(`, `%[`, `%<`.
fn percent_open(byte: u8) -> bool {
    matches!(byte, b'{' | b'(' | b'[' | b'<' | b'|' | b'!' | b'/' | b'^' | b'~')
}

fn percent_delimiter(byte: u8) -> bool {
    matches!(byte, b'q' | b'Q' | b'w' | b'W' | b'i' | b'I' | b'r' | b's' | b'x')
}

/// `%q(...)`, `%Q{...}`, `%w[...]` and `%r{...}`: the opening delimiter pairs
/// with its close, and bracket delimiters nest.
fn percent_literal(c: &mut Cursor) -> Kind {
    c.bump();
    // `%{...}` and `%(...)` are `%Q` without the letter.
    let letter = c.peek().to_ascii_lowercase();
    if percent_delimiter(c.peek()) {
        c.bump();
    }
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
    is_word_start(byte) || matches!(byte, b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'~' | b'&' | b'|' | b'^' | b'[' | b'@' | b'$' | b'"' | b'\'')
}

fn symbol(c: &mut Cursor) -> Kind {
    c.bump();
    // `:"#{name}"`: a symbol whose name is a quoted string, fields and all.
    if matches!(c.peek(), b'"' | b'\'') {
        skip_quoted(c, c.peek());
        return SYMBOL;
    }
    // `:@name` and `:$name`: a symbol for a variable.
    if matches!(c.peek(), b'@' | b'$') {
        variable(c);
        return SYMBOL;
    }
    if c.peek() == b'[' && c.peek_at(1) == b']' {
        c.bump_n(2);
        c.eat(b'=');
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
    // An extended regex spans lines, so a line break only ends it when the
    // rest of the file holds no closing slash: a division cannot run away.
    let multiline = closes_later(c, b'/');
    c.bump();
    let mut class = false;
    loop {
        match c.peek() {
            0 if c.at_end() => return false,
            b'\n' | b'\r' if !multiline => return false,
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
