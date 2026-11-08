//! Julia: modules, functions, structs, control flow and typed expressions.
//!
//! A newline separates statements, and like R the expression parser stops where
//! the next token cannot continue, so the statement list ends itself. `;` is an
//! explicit separator. Blocks (`function`, `if`, `for`, `while`, `begin`,
//! `struct`, `module`, `try`, `let`) all end with `end`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A keyword argument is an assignment.** `f(x = 1)` parses `x = 1` through
//!   the assignment row, not as a separate argument node.
//! - **Macros** (`@time`, `@assert`, string macros) are one `macro_name` token,
//!   not a call with a parsed argument.
//! - **`using X: a, b`** names are parsed as a plain dotted list; the `:` list is
//!   one `using` statement either way.
//! - **Broadcasting dots** (`f.(x)`, `.+`) are their own tokens but only `.+`,
//!   `.-`, `.*`, `./` are accepted as operators; a general dotted operator is
//!   an error.
//! - **Comprehensions** are modelled as a bracketed expression with a `for`
//!   clause inside, not as their own node.
//! - **`where` clauses** take a type parameter list, not full expressions.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { JULIA {
    name = "julia";
    lex = lex_julia;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        COMMA = ",",
        SEMI = ";",
        DOT = ".",
        COLON = ":",
        DOUBLE_COLON = "::",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        IDENTICAL = "===",
        NOT_IDENTICAL = "!==",
        LE = "<=",
        GE = ">=",
        LT = "<",
        SUBTYPE = "<:",
        GT = ">",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        SLASHSLASH = "//",
        PERCENT = "%",
        CARET = "^",
        AMP = "&",
        PIPE = "|",
        BANG = "!",
        TILDE = "~",
        AND = "&&",
        OR = "||",
        ARROW = "->",
        FAT_ARROW = "=>",
        PIPE_GT = "|>",
        ELLIPSIS = "...",
        DOLLAR = "$",
        AT = "@",
        MACRO_NAME = "macro_name",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        CHAR = "char_literal",
        BAD_STRING = "bad_string" bad,
        COMMAND = "command_literal",
        QUOTE_SIGN = "'",

        KW_MODULE = "module",
        KW_BAREMODULE = "baremodule",
        KW_FUNCTION = "function",
        KW_STRUCT = "struct",
        KW_MUTABLE = "mutable",
        KW_ABSTRACT = "abstract",
        KW_TYPE = "type",
        KW_END = "end",
        KW_IF = "if",
        KW_ELSEIF = "elseif",
        KW_ELSE = "else",
        KW_FOR = "for",
        KW_WHILE = "while",
        KW_IN = "in",
        KW_BEGIN = "begin",
        KW_LET = "let",
        KW_TRY = "try",
        KW_CATCH = "catch",
        KW_FINALLY = "finally",
        KW_RETURN = "return",
        KW_BREAK = "break",
        KW_CONTINUE = "continue",
        KW_USING = "using",
        KW_IMPORT = "import",
        KW_EXPORT = "export",
        KW_CONST = "const",
        KW_GLOBAL = "global",
        KW_LOCAL = "local",
        KW_MACRO = "macro",
        KW_DO = "do",
        KW_WHERE = "where",
        KW_TRUE = "true",
        KW_FALSE = "false",
        KW_NOTHING = "nothing",
    }

    program: _statement*,

    _statement: _statement_kind [";"],

    _statement_kind:
        module_definition
        | function_definition
        | struct_definition
        | macro_definition
        | if_statement
        | for_statement
        | while_statement
        | let_statement
        | begin_block
        | try_statement
        | abstract_type
        | using_statement
        | import_statement
        | export_statement
        | const_declaration
        | binding_declaration
        | return_statement
        | break_statement
        | continue_statement
        | expression_statement,

    body: statements "end",
    statements: _statement*,

    module_definition: ["baremodule"] "module" IDENT body,
    function_definition: "function" signature function_body,
    signature: IDENT parameter_list ["::" _expression] [where_clause],
    where_clause: "where" type_parameter ("," type_parameter)*,
    type_parameter: IDENT ["<:" _expression],

    function_body: body | "=" _expression,
    parameter_list: "(" [parameter (( "," | ";") parameter)*] ")",
    parameter: IDENT ["::" _expression] ["=" _expression] | "..." | "::" _expression,

    abstract_type: "abstract" "type" IDENT body,
    struct_definition: ["mutable"] "struct" IDENT [supertype] body,
    supertype: "<:" _expression,
    macro_definition: "macro" IDENT parameter_list body,

    if_statement: "if" _expression statements elseif_clause* [else_clause] "end",
    elseif_clause: "elseif" _expression statements,
    else_clause: "else" statements,
    for_statement: "for" iteration ("," iteration)* body,
    iteration: IDENT ["=" | "in"] _expression,
    while_statement: "while" _expression body,
    let_statement: "let" statements "end",
    begin_block: "begin" statements "end",
    try_statement: "try" statements [catch_clause] [finally_clause] "end",
    catch_clause: "catch" [IDENT] statements,
    finally_clause: "finally" statements,

    using_statement: "using" module_path ("," module_path)*,
    import_statement: "import" module_path ("," module_path)*,
    export_statement: "export" IDENT ("," IDENT)*,
    module_path: IDENT ("." IDENT)*,
    const_declaration: "const" IDENT "=" _expression,
    binding_declaration: ("global" | "local") IDENT ["=" _expression],

    return_statement: "return" [_expression],
    break_statement: "break",
    continue_statement: "continue",
    expression_statement: _expression,

    _expression: precedence _primary {
        right "=" => assignment,
        left "||" => logical_expression,
        left "&&" => logical_expression,
        left "==" | "!=" | "===" | "!==" | "<" | ">" | "<=" | ">=" => comparison,
        left "|>" => pipe_expression,
        left "=>" => pair,
        left "->" => anonymous_function,
        left "+" | "-" => arithmetic,
        left "*" | "/" | "//" | "%" => arithmetic,
        prefix "!" | "-" | "+" | "~" => unary_expression,
        right "^" => arithmetic,
        left "&" | "|" => bitwise_expression,
        postfix call_arguments => call,
        postfix index_arguments => index,
        postfix "." IDENT => member,
        postfix "::" _expression => type_assertion,
        postfix type_arguments => parameterized_type,
        postfix "'" => adjoint,
    },

    call_arguments: "(" [(argument %? ",")] [keyword_arguments] [for_clause] ")",
    /// `;` starts the keyword arguments in a call: `f(x; a = 1)`.
    keyword_arguments: ";" [(argument %? ",")],
    index_arguments: "[" [(argument %? ",")] "]",
    argument: _expression,

    _primary:
        IDENT
        | NUMBER
        | STRING
        | CHAR
        | COMMAND
        | "true"
        | "false"
        | "nothing"
        | MACRO_NAME [call_arguments]
        | paren_or_tuple
        | vector_literal,

    paren_or_tuple: "(" [(_expression %? ",")] ")",
    vector_literal: "[" [(_expression %? ",")] [for_clause] "]",
    type_arguments: "{" [(_expression %? ",")] "}",
    for_clause: "for" iteration ("," iteration)*,
} }

pub fn lex_julia(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'#' => {
            if c.peek_at(1) == b'=' {
                lex::block_comment(c, b"#=", b"=#", true);
            } else {
                lex::line_comment(c);
            }
            COMMENT
        }
        b'"' if c.peek_at(1) == b'"' && c.peek_at(2) == b'"' => {
            c.bump_n(3);
            loop {
                if c.at_end() {
                    return BAD_STRING;
                }
                if c.starts_with(b"\"\"\"") {
                    c.bump_n(3);
                    return STRING;
                }
                c.bump_char();
            }
        }
        b'"' => if lex::quoted(c, b'"') { STRING } else { BAD_STRING },
        b'\'' => {
            if c.peek_at(1) == b'\\' {
                c.bump();
                c.bump();
                c.eat_while(|byte| byte != b'\'' && byte != b'\n');
                c.eat(b'\'');
                CHAR
            } else if c.peek_at(2) == b'\'' {
                c.bump_n(3);
                CHAR
            } else {
                // A transpose is one token; the grammar takes it after an operand.
                c.bump();
                QUOTE_SIGN
            }
        }
        b'`' => {
            c.bump();
            loop {
                if c.at_end() {
                    return BAD_STRING;
                }
                if c.peek() == b'`' {
                    c.bump();
                    return COMMAND;
                }
                c.bump_char();
            }
        }
        b'@' => {
            c.bump();
            c.eat_while(lex::is_word_byte);
            MACRO_NAME
        }
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'.' if c.peek_at(1).is_ascii_digit() => { c.bump(); lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            let start = c.rest();
            let mut length = c.eat_while(lex::is_word_byte);
            // `append!` and `haskey?` are one name in Julia.
            if matches!(c.peek(), b'!' | b'?') && !matches!(c.peek_at(1), b'=') {
                c.bump();
                length += 1;
            }
            keyword(&start[..length])
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b'{' => { c.bump(); LBRACE }
        b'}' => { c.bump(); RBRACE }
        b',' => { c.bump(); COMMA }
        b';' => { c.bump(); SEMI }
        b'.' if c.peek_at(1) == b'.' && c.peek_at(2) == b'.' => { c.bump_n(3); ELLIPSIS }
        b'.' if matches!(c.peek_at(1), b'+' | b'-' | b'*' | b'/') => {
            c.bump_n(2);
            match c.behind(1) {
                b'+' => PLUS,
                b'-' => MINUS,
                b'*' => STAR,
                _ => SLASH,
            }
        }
        b'.' => { c.bump(); DOT }
        b':' if c.peek_at(1) == b':' => { c.bump_n(2); DOUBLE_COLON }
        b':' => { c.bump(); COLON }
        b'=' if c.peek_at(1) == b'=' && c.peek_at(2) == b'=' => { c.bump_n(3); IDENTICAL }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' if c.peek_at(1) == b'>' => { c.bump_n(2); FAT_ARROW }
        b'=' => { c.bump(); ASSIGN }
        b'!' if c.peek_at(1) == b'=' && c.peek_at(2) == b'=' => { c.bump_n(3); NOT_IDENTICAL }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' => { c.bump(); BANG }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b':' => { c.bump_n(2); SUBTYPE }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' => { c.bump(); GT }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' if c.peek_at(1) == b'/' => { c.bump_n(2); SLASHSLASH }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'^' => { c.bump(); CARET }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'&' => { c.bump(); AMP }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); OR }
        b'|' if c.peek_at(1) == b'>' => { c.bump_n(2); PIPE_GT }
        b'|' => { c.bump(); PIPE }
        b'~' => { c.bump(); TILDE }
        b'$' => { c.bump(); DOLLAR }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"module" => KW_MODULE,
        b"baremodule" => KW_BAREMODULE,
        b"function" => KW_FUNCTION,
        b"struct" => KW_STRUCT,
        b"mutable" => KW_MUTABLE,
        b"abstract" => KW_ABSTRACT,
        b"type" => KW_TYPE,
        b"end" => KW_END,
        b"if" => KW_IF,
        b"elseif" => KW_ELSEIF,
        b"else" => KW_ELSE,
        b"for" => KW_FOR,
        b"while" => KW_WHILE,
        b"in" => KW_IN,
        b"begin" => KW_BEGIN,
        b"let" => KW_LET,
        b"try" => KW_TRY,
        b"catch" => KW_CATCH,
        b"finally" => KW_FINALLY,
        b"return" => KW_RETURN,
        b"break" => KW_BREAK,
        b"continue" => KW_CONTINUE,
        b"using" => KW_USING,
        b"import" => KW_IMPORT,
        b"export" => KW_EXPORT,
        b"const" => KW_CONST,
        b"global" => KW_GLOBAL,
        b"local" => KW_LOCAL,
        b"macro" => KW_MACRO,
        b"do" => KW_DO,
        b"where" => KW_WHERE,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        b"nothing" => KW_NOTHING,
        _ => IDENT,
    }
}
