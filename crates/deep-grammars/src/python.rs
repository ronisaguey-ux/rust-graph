//! Python through `grammar!`: Slice 3. Layout tokens come from the runtime
//! lexer (`syntax::Layout`); expressions are one precedence table; the step
//! lexer below knows strings, f-strings and numbers.
//!
//! The tree is close to Tree-sitter's Python where that costs nothing, but it
//! is not the same tree: strings are single tokens (f-string fields are not
//! parsed), and a few rarely seen forms nest differently.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **Indentation is not a rule.** No `grammar!` form describes "a block is a
//!   deeper line". The `layout = NEWLINE INDENT DEDENT;` setting hands the
//!   three inferred tokens to the runtime lexer, and the rules consume them
//!   like any other token (`block: INDENT _statement* DEDENT`). A grammar that
//!   needed layout without a runtime that infers it could not be written.
//! - **An f-string's `{...}` fields are not code.** Parsing them needs the
//!   lexer to switch to a string mode and back at a nested brace depth, which
//!   the DSL's one-function-per-token lexer cannot ask for, so a whole string
//!   is one `string_literal` token. `string()` in this file scans the prefix,
//!   the quotes and the escapes; the fields stay opaque.
//! - **No negative lookahead and no token beyond the third.** A choice is
//!   decided by the next one to three tokens, so `match`/`case`/`type` as soft
//!   keywords are resolved with a leading predicate (`&(...)`), not by a
//!   general lookahead. A construct that needs four tokens of context would
//!   have to be split into extra hidden rules.
//! - **One kind per rule.** Two alternatives that must produce different node
//!   names need two rules (`function_definition` versus `class_definition`),
//!   because naming an alternative is not part of the DSL.

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { PYTHON {
    name = "python";
    lex = lex;
    layout = NEWLINE INDENT DEDENT;

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
    import_from_statement: "from" (relative_import | dotted_name) "import" (wildcard_import | import_list | _import_names),
    import_list: "(" (_import_name %? ",") ")",
    wildcard_import: "*",
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

    expression_statement: _assignment_chain,
    /// `a: int = b = 1`, `a += 1`, or an expression alone.
    _assignment_chain: precedence _expressions {
        right "=" => assignment,
        right "+=" | "-=" | "*=" | "**=" | "/=" | "//=" | "%=" | "|=" | "^=" | "&=" | "<<=" | ">>=" | "@=" => augmented_assignment,
        left ":" => annotated,
    },

    _compound_statement:
        if_statement
        | for_statement
        | while_statement
        | try_statement
        | with_statement
        | function_definition
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
    function_definition: ["async"] "def" _name [type_parameters] parameters ["->" _expression] ":" _suite,
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
        b'\'' | b'"' => string(c, false),
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
