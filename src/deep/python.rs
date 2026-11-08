//! Python through `grammar!`: Slice 3. Layout tokens come from the runtime
//! lexer (`syntax::Layout`); expressions are one precedence table; the step
//! lexer below knows strings, f-strings and numbers.
//!
//! The tree is close to Tree-sitter's Python where that costs nothing, but it
//! is not the same tree: strings are single tokens (f-string fields are not
//! parsed), and a few rarely seen forms nest differently.

use super::lexer::Cursor;
use super::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

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
        if let Some(code) = string_prefix(text) {
            return string(c, code);
        }
    }
    keyword(text)
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
/// does not close there; a triple-quoted one runs to the end of the file.
///
/// Fields of a one-line f-string may span lines (Python 3.12). Such a string
/// is taken whole only if it then closes; otherwise it is lexed again with
/// fields ending at the line, so a broken one costs one line.
fn string(c: &mut Cursor, code: bool) -> Kind {
    let quote = c.peek();
    let triple = c.peek_at(1) == quote && c.peek_at(2) == quote;
    let open = if triple { 3 } else { 1 };
    if code && !triple {
        let mut trial = Cursor::new(c.rest());
        trial.bump_n(open);
        if string_body(&mut trial, quote, false, true, true) {
            c.bump_n(trial.pos());
            return STR;
        }
    }
    c.bump_n(open);
    if string_body(c, quote, triple, code, false) { STR } else { BAD_STR }
}

/// The rest of a string after its opening quotes. True if it closed.
fn string_body(c: &mut Cursor, quote: u8, triple: bool, code: bool, spanning: bool) -> bool {
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            // A backslash never escapes a brace: `rf"\{{"` is a backslash and `{`.
            b'\\' if code && matches!(c.peek_at(1), b'{' | b'}') => c.bump(),
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
            b'{' if code => {
                if c.peek_at(1) == b'{' {
                    c.bump_n(2);
                } else {
                    c.bump();
                    field(c, quote, triple || spanning);
                }
            }
            _ => c.bump(),
        }
    }
}

/// An f-string replacement field after its `{`, through its `}`. Strings
/// inside may reuse the outer quote (Python 3.12). In a one-line string a line
/// break ends the field, so a broken field cannot swallow the file.
fn field(c: &mut Cursor, quote: u8, triple: bool) {
    let mut depth = 0u32;
    loop {
        if c.at_end() {
            return;
        }
        match c.peek() {
            b'\'' | b'"' => {
                let inner = c.peek();
                let inner_triple = c.peek_at(1) == inner && c.peek_at(2) == inner;
                if !inner_triple && !triple && inner == quote && depth == 0 && matches!(c.peek_at(1), b'\n' | b'\r' | 0) {
                    // `f"{x"` then the line ends: the outer string's end.
                    return;
                }
                string(c, false);
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = c.rest();
                let length = c.eat_while(is_word_byte);
                if matches!(c.peek(), b'\'' | b'"') {
                    if let Some(code) = string_prefix(&start[..length]) {
                        string(c, code);
                    }
                }
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                c.bump();
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                c.bump();
            }
            b'}' => {
                c.bump();
                if depth == 0 {
                    return;
                }
                depth -= 1;
            }
            b':' if depth == 0 => {
                c.bump();
                return spec(c, quote, triple);
            }
            b'\n' | b'\r' if !triple => return,
            _ => c.bump(),
        }
    }
}

/// A format spec after its `:`, through the field's `}`. It may hold fields.
fn spec(c: &mut Cursor, quote: u8, triple: bool) {
    loop {
        if c.at_end() {
            return;
        }
        match c.peek() {
            b'{' => {
                c.bump();
                field(c, quote, triple);
            }
            b'}' => {
                c.bump();
                return;
            }
            byte if byte == quote => return,
            b'\n' | b'\r' if !triple => return,
            b'\\' => c.bump_n(2),
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

#[cfg(test)]
mod tests {
    use super::super::budget::Budget;
    use super::super::parser::Workspace;
    use super::super::syntax::{Kind, ERROR};
    use super::super::tests::{check, mutate, on_big_stack, Rng};
    use super::super::tree::HAS_ERROR;
    use super::*;

    fn parse(src: &str) -> Workspace {
        let mut workspace = Workspace::default();
        let outcome = workspace.parse(src.as_bytes(), &PYTHON, &Budget::default()).unwrap();
        assert!(outcome.complete(), "{src:?}: {}", outcome.describe());
        check(&workspace, src.as_bytes(), &PYTHON);
        workspace
    }

    fn named(src: &str) -> String {
        parse(src).tree.sexp(&PYTHON, true)
    }

    fn clean(src: &str) -> bool {
        parse(src).tree.root().flags & HAS_ERROR == 0
    }

    /// The tokens the grammar sees, layout included, by name.
    fn significant(src: &str) -> Vec<&'static str> {
        let workspace = parse(src);
        let tokens = &workspace.tokens;
        (0..tokens.len()).filter(|&at| !PYTHON.is_trivia(tokens.kind[at])).map(|at| PYTHON.name(tokens.kind[at])).collect()
    }

    #[test]
    fn a_bad_function_name_is_an_error_inside_the_function() {
        // #5910: the function survives with the error inside it, and so does
        // everything after it.
        assert_eq!(
            named("def 1():\n    pass\nx = 1\n"),
            "(module (function_definition def (ERROR number) (parameters) newline (block indent (pass_statement pass) newline dedent)) (expression_statement (assignment identifier number)) newline)"
        );
        let tree = named("class 1:\n    x = 2\n");
        assert!(tree.starts_with("(module (class_definition class (ERROR number) newline (block"), "{tree}");
        let tree = named("if :\n    pass\nelse:\n    pass\n");
        assert!(tree.starts_with("(module (if_statement if (ERROR MISSING) newline (block"), "{tree}");
        assert!(tree.contains("(else_clause"), "{tree}");
        let tree = named("for in x:\n    pass\n");
        assert!(tree.starts_with("(module (for_statement for (ERROR MISSING) in identifier newline (block"), "{tree}");
    }

    #[test]
    fn operators_nest_by_precedence() {
        let cases = [
            ("a + b * c ** -d\n", "(binary_operator identifier (binary_operator identifier (binary_operator identifier (unary_operator identifier))))"),
            ("-a ** 2\n", "(unary_operator (binary_operator identifier number))"),
            ("a ** b ** c\n", "(binary_operator identifier (binary_operator identifier identifier))"),
            ("a - b - c\n", "(binary_operator (binary_operator identifier identifier) identifier)"),
            (
                "not a == b or c and d\n",
                "(boolean_operator (not_operator not (comparison_operator identifier identifier)) or (boolean_operator identifier and identifier))",
            ),
            ("a is not b\n", "(comparison_operator identifier is not identifier)"),
            ("a not in b\n", "(comparison_operator identifier not in identifier)"),
            ("a | b ^ c & d << e\n", "(binary_operator identifier (binary_operator identifier (binary_operator identifier (binary_operator identifier identifier))))"),
            (
                "x if c else y if d else z\n",
                "(conditional_expression identifier if identifier else (conditional_expression identifier if identifier else identifier))",
            ),
            ("a.b(c)[d]\n", "(subscript (call (attribute identifier identifier) (argument_list identifier)) (subscript_list identifier))"),
            ("await f() + 1\n", "(binary_operator (await await (call identifier (argument_list))) number)"),
            ("(y := a or b)\n", "(parenthesized_expression (named_expression identifier (boolean_operator identifier or identifier)))"),
            ("lambda x, *a, k=1: x + k\n", "(lambda lambda (lambda_parameter identifier) (lambda_star_parameter identifier) (lambda_parameter identifier number) (binary_operator identifier identifier))"),
        ];
        for (src, want) in cases {
            assert_eq!(named(src), format!("(module (expression_statement {want}) newline)"), "{src:?}");
        }
    }

    #[test]
    fn statements_and_their_parts() {
        let tree = named("a, *b = c: int = f(x, *y, k=v, **w)\n");
        assert_eq!(
            tree,
            "(module (expression_statement (assignment (expression_list identifier (list_splat identifier)) (assignment (annotated identifier identifier) (call identifier (argument_list identifier (list_splat identifier) (keyword_argument identifier identifier) (dictionary_splat identifier)))))) newline)"
        );
        assert_eq!(named("x[1:2, ::3, a]\n"), "(module (expression_statement (subscript identifier (subscript_list (slice number number) (open_slice number) identifier))) newline)");
        assert_eq!(named("{k: v for k, v in d if k}\n"), "(module (expression_statement (dictionary (pair identifier identifier) (for_in_clause for (pattern_list identifier identifier) in identifier) (if_clause if identifier))) newline)");
        assert_eq!(named("from .. import (a as b, c,)\n"), "(module (import_from_statement from (relative_import) import (import_list (dotted_name identifier) as identifier (dotted_name identifier))) newline)");
        assert_eq!(named("type T[*Ts] = tuple[*Ts]\n"), "(module (type_alias_statement type identifier (type_parameters (type_parameter identifier)) (subscript identifier (subscript_list (list_splat identifier)))) newline)");
        // Soft keywords stay names where they cannot start their statement.
        assert!(clean("match = 1\nmatch(x)\ntype = 2\ncase = match.case\n"));
        assert!(clean("match x:\n    case [1, *rest] if rest:\n        pass\n    case {\"k\": v, **kw} | Point(x=0) as p:\n        pass\n"));
        assert!(clean("@decorator(1)\nasync def f[T](a, /, b: int = 2, *c: *Ts, d, **e) -> T:\n    async with a as b, c:\n        yield from b\n    async for x in y: await x\n"));
        assert!(clean("try:\n    pass\nexcept* (A, B) as e:\n    raise X from e\nelse:\n    pass\nfinally:\n    del a, b\n"));
        assert!(clean("while x: x -= 1; continue\nelse: global a, b\n"));
        assert!(clean("print(f\"{x!r:>{width}} {d[\"k\"]} {'{'}\", rb'\\d', f'''{\n  x\n}''', 0x1F, 1_000.5e-3j)\n"));
    }

    #[test]
    fn layout_becomes_tokens() {
        assert_eq!(
            significant("if a:\n    b\n\n    # c\n    c\nd\n"),
            ["if", "identifier", ":", "newline", "indent", "identifier", "newline", "identifier", "newline", "dedent", "identifier", "newline"]
        );
        // Inside brackets and after a backslash a line break is whitespace.
        assert_eq!(significant("x = (1,\n  2) + \\\n  3\n"), ["identifier", "=", "(", "number", ",", "number", ")", "+", "number", "newline"]);
        // The end of the file ends the line and closes every block.
        assert_eq!(
            significant("def f():\n  if a:\n    b"),
            ["def", "identifier", "(", ")", ":", "newline", "indent", "if", "identifier", ":", "newline", "indent", "identifier", "newline", "dedent", "dedent"]
        );
        // A newline and the dedents sit right after the last token, before the
        // comment and blank lines that follow it.
        let src = "if a:\n    b  # note\n\nc\n";
        let workspace = parse(src);
        let tokens = &workspace.tokens;
        let at = (0..tokens.len()).find(|&at| tokens.kind[at] == DEDENT).unwrap();
        assert_eq!(tokens.start(at) as usize, src.find("b").unwrap() + 1);
        assert_eq!(tokens.kind[at - 1], NEWLINE);
        // A dedent to a column no outer line used opens a block of its own.
        let tree = named("if a:\n        b\n    c\n");
        assert!(tree.contains("ERROR"), "{tree}");
    }

    #[test]
    fn a_statement_keyword_ends_an_unclosed_bracket() {
        let src = "foo(1,\ndef g():\n    return 2\nx = [\nclass C:\n    pass\n";
        let tree = named(src);
        assert!(tree.contains("(function_definition def identifier (parameters) newline (block indent (return_statement return number) newline dedent))"), "{tree}");
        assert!(tree.contains("(class_definition class identifier newline (block indent (pass_statement pass) newline dedent))"), "{tree}");
        assert!(tree.contains("ERROR") || tree.contains("UNCLOSED"), "{tree}");
    }

    #[test]
    fn errors_stay_in_their_statement() {
        let tree = named("x = = 1\ny = 2\nif x\n    pass\nz = 3\n");
        assert!(tree.ends_with("(expression_statement (assignment identifier number)) newline)"), "{tree}");
        assert!(tree.contains("(expression_statement (assignment identifier number)) newline (if_statement"), "{tree}");
        assert!(!clean("x = (1, 2\n"));
        assert!(!clean("f(a for)\n"));
        assert!(!clean("return return\n"));
        assert!(!clean("x = 'open\n"));
    }

    #[test]
    fn deep_input_stays_within_the_budget() {
        on_big_stack(|| {
            let deep = [
                "(".repeat(100_000),
                format!("x = {}y\n", "-".repeat(100_000)),
                format!("x = {}y\n", "not ".repeat(100_000)),
                format!("x = {}y\n", "a ** ".repeat(100_000)),
                format!("x = {}y\n", "a if b else ".repeat(100_000)),
                format!("x = {}y\n", "lambda: ".repeat(100_000)),
                (0..3_000).map(|level| format!("{}if x:\n", " ".repeat(level))).collect::<String>() + &" ".repeat(3_000) + "pass\n",
            ];
            for src in deep {
                let mut workspace = Workspace::default();
                let outcome = workspace.parse(src.as_bytes(), &PYTHON, &Budget::default()).unwrap();
                assert!(outcome.exhausted.is_none(), "{}", outcome.describe());
                check(&workspace, src.as_bytes(), &PYTHON);
                assert_eq!(workspace.tree.root().kind, MODULE);
            }
        });
    }

    const SEEDS: &[&str] = &[
        "import os, sys\nfrom . import (a, b)\n\nclass C(Base, metaclass=M):\n    \"\"\"Doc.\"\"\"\n    x: int = 1\n\n    @property\n    def f(self, a, *b, c=2, **d) -> int:\n        if a and not b:\n            return [i ** 2 for i in range(10) if i % 2]\n        elif c:\n            raise ValueError(f\"{a!r} {b:>{c}}\")\n        else:\n            with open(a) as f, g:\n                yield from f\n        try:\n            x = {k: v for k, v in d.items()}\n        except (A, B) as e:\n            pass\n        finally:\n            del x\n",
        "match command.split():\n    case [action]:\n        pass\n    case [\"go\", direction] | [\"move\", direction]:\n        go(direction)\n    case Point(x=0, y=0) if ok:\n        print('origin')\n    case _:\n        pass\n",
        "async def main():\n    async with lock:\n        await asyncio.sleep(1)\n    async for item in aiter():\n        x = lambda a, b=1: a + b\n        y = a if b else c\n        z = (yield)\n        w = x[1:2, ::3]\n        type Alias[T] = list[T]\n\nwhile True: x += 1; break\n",
    ];

    const PYTHON_BYTES: &[u8] = b"()[]{}:,;.=+-*/%<>!@&|^~\"'\\#0123456789 \n\n\n\t    xyfdefclassifelsereturnlambdanotandor\xc3\xa9";

    #[test]
    fn fuzzed_python_terminates_and_keeps_every_invariant() {
        on_big_stack(|| {
            let mut rng = Rng(0x5910_0003_1337_beef);
            let mut workspace = Workspace::default();
            let budget = Budget::default();
            let mut erroneous = 0;
            const ROUNDS: usize = 3000;
            for round in 0..ROUNDS {
                let mut src = SEEDS[round % SEEDS.len()].as_bytes().to_vec();
                for _ in 0..1 + rng.below(6) {
                    let length = src.len();
                    mutate(&mut rng, &mut src, PYTHON_BYTES, b"", 0, length);
                }
                let outcome = workspace.parse(&src, &PYTHON, &budget).unwrap();
                assert!(outcome.exhausted.is_none(), "budget ran out on {:?}", String::from_utf8_lossy(&src));
                check(&workspace, &src, &PYTHON);
                assert_eq!(workspace.tree.root().kind, MODULE);
                if workspace.tree.root().flags & HAS_ERROR != 0 {
                    erroneous += 1;
                }
            }
            // Python is forgiving (most edits to a name leave a name), but at
            // least half of random edits must still be reported.
            assert!(erroneous > ROUNDS / 2, "{erroneous} of {ROUNDS}");
        });
        for seed in SEEDS {
            assert!(clean(seed), "{seed}");
        }
    }

    #[test]
    fn every_error_kind_is_the_runtime_one() {
        let workspace = parse("def (:\n");
        let errors: Vec<Kind> = workspace.tree.nodes.iter().map(|node| node.kind).filter(|&kind| kind == ERROR).collect();
        assert!(!errors.is_empty());
    }
}
