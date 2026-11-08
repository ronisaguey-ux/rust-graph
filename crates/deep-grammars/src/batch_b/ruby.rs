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
        INSTANCE_VAR = "instance_variable",
        CLASS_VAR = "class_variable",
        GLOBAL_VAR = "global_variable",
        REGEX = "regex",
        /// A string, symbol, regex or heredoc with no end.
        BAD_LITERAL = "bad_literal" bad,

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

    require_statement: ("require" | "require_relative" | "load") _expression,
    include_statement: ("include" | "extend" | "prepend") _constant_path ("," _constant_path)*,

    module_definition: "module" _constant_path body "end",
    class_definition: "class" _constant_path ["<" _constant_path] body "end",
    method_definition: "def" method_name [parameters] body "end",
    method_name: ["self" "."] _method_ident,
    parameters: "(" (_parameter %? ",") ")",
    _parameter: _name ["=" _expression@assign],
    _method_ident: IDENT,
    _constant_path: _name ("::" _name)*,

    /// Statements up to whatever closes the construct: a keyword such as `end`,
    /// or the bracket of a block.
    body: _statement*,

    if_statement: "if" _expression [then_token] body elsif_clause* [else_clause] "end",
    elsif_clause: "elsif" _expression [then_token] body,
    else_clause: "else" body,
    then_token: "then",
    unless_statement: "unless" _expression [then_token] body [else_clause] "end",
    while_statement: "while" _expression body "end",
    until_statement: "until" _expression body "end",
    for_statement: "for" _name ("," _name)* "in" _expression body "end",

    case_statement: "case" [_expression] when_clause+ [else_clause] "end",
    when_clause: "when" (_expression % ",") [then_token] body,

    begin_statement: "begin" body rescue_clause* [else_clause] [ensure_clause] "end",
    rescue_clause: "rescue" [_exception_list] body,
    _exception_list: _constant_path ("," _constant_path)*,
    ensure_clause: "ensure" body,

    return_statement: "return" [_expression ("," _expression)*],
    break_statement: "break",
    next_statement: "next",
    expression_statement: _expression,

    // ---- blocks ----

    brace_block: "{" [block_parameters] _statement* "}",
    do_block: "do" [block_parameters] _statement* "end",
    block_parameters: "|" (_name %? ",") "|",

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
        | yield_expression,

    _literal: NUMBER | STRING | SYMBOL | REGEX | "nil" | "true" | "false",
    _name: IDENT,
    _variable: INSTANCE_VAR | CLASS_VAR | GLOBAL_VAR,
    super_call: "super" [arguments],
    yield_expression: "yield" [arguments],
    parenthesized_expression: "(" _expression ")",
    array: "[" (_expression %? ",") "]",
    hash: "{" (_hash_entry %? ",") "}",
    _hash_entry: &(_name ":") label_pair | arrow_pair,
    label_pair: _name ":" _expression,
    arrow_pair: _expression@bor "=>" _expression,
    arguments: "(" (_argument %? ",") ")",
    _argument: &(_name ":") label_argument | _expression,
    label_argument: _name ":" _expression,
    subscript: "[" [_expression] "]",

    /// After `.` and `::` every keyword is a method name too: `x.class`.
    _property:
        IDENT | "class" | "then" | "begin" | "end" | "nil" | "self" | "true" | "false"
        | "not" | "and" | "or" | "in" | "do" | "case" | "when" | "if" | "unless" | "while"
        | "return" | "yield" | "def" | "module" | "require" | "super",
} }

