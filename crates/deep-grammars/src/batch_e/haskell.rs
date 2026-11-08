//! Haskell: the module header, imports, `data`/`newtype`/`type`/`class`/
//! `instance` declarations, type signatures, function bindings with guards and
//! `where` blocks, and the expressions and `do` notation a scanner reads.
//!
//! Haskell is indentation-scoped, so the grammar uses the runtime's layout pass
//! (`layout = NEWLINE INDENT DEDENT`, the same machinery Python uses). That is
//! what makes `do` blocks, `case` alternatives, `class` bodies and indented
//! `where` blocks ordinary rules instead of token soup.
//!
//! # What is modelled
//!
//! - **Modules.** A `module ... where` header with an export list, `import`
//!   (qualified, `as`, `hiding`, an import list), and `foreign import`.
//! - **Declarations.** `data`, `newtype`, `type` (with record fields and
//!   `deriving`), `class` and `instance` with indented bodies, `infixl`-style
//!   fixity declarations, type signatures and function bindings with `_pattern`
//!   arguments, guards and an indented `where` block.
//! - **Expressions.** One precedence table: `$`, `||`, `&&`, comparisons,
//!   `:`/`++`, `+`/`-`, `*`/`/`/`div`/`mod`, `^`, unary minus, application and
//!   record construction; `if`/`then`/`else`, `case ... of`, `do`, `let ... in`,
//!   `\x -> e`, literals, tuples, lists and ranges.
//! - **Types.** `->`, `=>` and type application, with lists, tuples, the unit
//!   type and type variables.
//! - **The lexer** knows `--` and nested `{- -}` comments, `"..."` with escapes
//!   and string gaps, character literals, decimal/hex/octal/binary numbers with
//!   `_`, and — the part Haskell makes unusual — a run of symbol bytes as one
//!   operator token, so `>>=`, `<*>` and `<>` are single tokens.
//!
//! # What is missing, named here rather than accepted silently
//!
//! - **List comprehensions.** `[x | x <- xs]` is not modelled; the `|` inside
//!   the brackets is a guard byte, not a separator.
//! - **`where` at the same indentation as its binding.** The clause is read
//!   from an indent, so `f x = y where y = 1` on one line is not; the indented
//!   form and the inline form after a `=` are.
//! - **Operator sections** (`(+ 1)`, `(1 +)`) are parenthesised expressions with
//!   an operator, which the expression table does not start with.
//! - **Pattern synonyms, type families, GADTs, `MultiWayIf`, arrows** and
//!   `{-# LANGUAGE #-}` pragmas beyond one trivia token are not modelled.
//! - **Qualified names are one token** (`Data.List.sort`), so the module part
//!   is not a node of its own.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { HASKELL {
    name = "haskell";
    lex = lex;
    layout = NEWLINE INDENT DEDENT;

    tokens {
        NEWLINE = "newline",
        INDENT = "indent" open block,
        DEDENT = "dedent" close block,

        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,

        IDENT = "variable",
        CONID = "constructor",
        QUALIFIED = "qualified_name",
        NUMBER = "number",
        STRING = "string",
        CHAR = "char",
        OP = "operator",
        BAD_LITERAL = "bad_literal" bad,

        KW_AS = "as",
        KW_CASE = "case",
        KW_CLASS = "class",
        KW_DATA = "data",
        KW_DERIVING = "deriving",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_FALSE = "False",
        KW_FOREIGN = "foreign",
        KW_HIDING = "hiding",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_IN = "in",
        KW_INFIX = "infix",
        KW_INFIXL = "infixl",
        KW_INFIXR = "infixr",
        KW_INSTANCE = "instance",
        KW_LET = "let",
        KW_MODULE = "module",
        KW_NEWTYPE = "newtype",
        KW_OF = "of",
        KW_QUALIFIED = "qualified",
        KW_THEN = "then",
        KW_TRUE = "True",
        KW_TYPE = "type",
        KW_WHERE = "where",
        KW_DEFAULT = "default",
        KW_DIV = "div",
        KW_MOD = "mod",
        KW_QUOT = "quot",
        KW_REM = "rem",
        KW_NOT = "not",

        COMMA = ",",
        SEMI = ";",
        COLON = ":",
        COLONCOLON = "::",
        ARROW = "->",
        LARROW = "<-",
        FATARROW = "=>",
        ASSIGN = "=",
        PIPE = "|",
        AT = "@",
        TILDE = "~",
        BACKSLASH = "\\",
        DOT = ".",
        DOTDOT = "..",
        DOLLAR = "$",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        EQUALS = "==",
        NOTEQUALS = "/=",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        ANDAND = "&&",
        OROR = "||",
        PLUSPLUS = "++",
        DIAMOND = "<>",
        CARET = "^",
        BANG = "!",
        PERCENT = "%",
        AMP = "&",
        SHR = ">>",
        SHL = "<<",
        UNDERSCORE = "_",
    }

    program: [module_header] [NEWLINE] declaration*,

    module_header: "module" module_name [module_exports_and_where],
    module_exports_and_where: "where" | export_list "where" | NEWLINE indented_header,
    indented_header: INDENT export_list [NEWLINE] "where" [NEWLINE] DEDENT,
    module_name: QUALIFIED | CONID,
    declaration_name: _name | CONID,
    export_list: "(" (export_item %? ",") ")",
    export_item: declaration_name [export_constructors] | "module" module_name,
    export_constructors: "(" (".." | exported_names) ")",
    exported_names: declaration_name ("," declaration_name)* [","],

    declaration:
        import_declaration
        | data_declaration
        | newtype_declaration
        | type_declaration
        | class_declaration
        | instance_declaration
        | fixity_declaration
        | foreign_declaration
        | type_signature
        | function_binding
        | NEWLINE,

    import_declaration: "import" ["qualified"] module_name ["as" module_name] [import_tail] [NEWLINE],
    import_tail: "hiding" import_list | import_list,
    import_list: "(" (import_item %? ",") ")",
    import_item: _name | qualified_operator | "type" _name,
    qualified_operator: "(" operator_name ")",
    operator_name: OP | CONID | ":" | "::" | ".." | "!" | "+" | "-" | "*" | "/" | "<" | ">" | "==" | "$",

    data_declaration: "data" declaration_name type_variables [data_constructors] data_tail,
    data_constructors: "=" constructor ("|" constructor)*,
    data_tail: [deriving_clause] [NEWLINE [indented_deriving]],
    newtype_declaration: "newtype" declaration_name type_variables "=" constructor data_tail,
    deriving_clause: "deriving" (deriving_list | declaration_name),
    indented_deriving: INDENT deriving_clause [NEWLINE] DEDENT,
    deriving_list: "(" (declaration_name %? ",") ")",
    constructor: CONID atom_type* [record_fields],
    record_fields: "{" (_field_declaration %? ",") "}",
    _field_declaration: _name ("," _name)* "::" _type,
    type_variables: _name*,

    type_declaration: "type" declaration_name type_variables "=" _type [NEWLINE],

    class_declaration: "class" class_head ["where" NEWLINE class_block] [NEWLINE],
    class_head: &(atom_type+ "=>") class_constraints declaration_name type_variables | declaration_name type_variables,
    class_constraints: atom_type+ "=>",
    class_block: INDENT (declaration [NEWLINE])* DEDENT,
    instance_declaration: "instance" _type ["where" NEWLINE instance_block] [NEWLINE],
    instance_block: INDENT (declaration [NEWLINE])* DEDENT,

    fixity_declaration: ("infixl" | "infixr" | "infix") [NUMBER] (operator_name % ",") NEWLINE,
    foreign_declaration: "foreign" "import" import_kind _name "::" _type NEWLINE,
    import_kind: IDENT,

    type_signature: _name ("," _name)* "::" _type [NEWLINE],

    function_binding: _name _pattern* (function_rhs | guarded_rhs | NEWLINE indented_guards) [indented_where] [NEWLINE],
    indented_guards: INDENT guarded_rhs DEDENT,
    function_rhs: "=" (rhs_expression | NEWLINE indented_rhs),
    rhs_expression: _expression [NEWLINE],
    indented_rhs: INDENT _expression [NEWLINE] DEDENT,
    guarded_rhs: guarded_clause+,
    guarded_clause: "|" _expression "=" _expression [NEWLINE],
    indented_where: INDENT where_clause DEDENT,
    where_clause: "where" NEWLINE where_block,
    where_block: INDENT (declaration [NEWLINE])* DEDENT,

    // ---- patterns ----

    _pattern: pattern_head [":" _pattern],
    pattern_head: pattern_atom (_pattern_application)*,
    _pattern_application: pattern_atom | "@" pattern_atom,
    pattern_atom: _name | CONID | NUMBER | STRING | CHAR | "True" | "False" | "_" | pattern_brackets | pattern_tuple,
    pattern_brackets: "[" [(_pattern %? ",")] "]",
    pattern_tuple: "(" (_pattern %? ",") ")",

    // ---- expressions ----

    _expression: precedence _operand {
        dollar: right "$" => application_expression,
        annotation: left "::" _type => type_annotation,
        disjunction: left "||" => binary_expression,
        conjunction: left "&&" => binary_expression,
        compare: left "==" | "/=" | "<" | ">" | "<=" | ">=" => binary_expression,
        cons: right ":" => cons_expression,
        append: right "++" | "<>" => binary_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "div" | "mod" | "quot" | "rem" => binary_expression,
        power: right "^" => binary_expression,
        unary: prefix "-" | "not" | "!" => unary_expression,
        apply: postfix _operand => application_expression,
        record: postfix record_braces => record_construction,
        custom: left OP => binary_expression,
    },

    _operand:
        _literal
        | _name
        | CONID
        | QUALIFIED
        | tuple
        | list
        | if_expression
        | case_expression
        | do_expression
        | let_expression
        | lambda,
    _literal: NUMBER | STRING | CHAR | BAD_LITERAL | "True" | "False",
    _name: IDENT,
    tuple: "(" (_expression %? ",") ")",
    list: "[" [&(_expression "..") range_expression | element_list] "]",
    range_expression: _expression ".." [_expression],
    element_list: _expression ("," _expression)* [","],
    record_braces: "{" (_record_binding %? ",") "}",
    _record_binding: _name "=" _expression,

    if_expression: "if" _expression [NEWLINE] "then" _expression [NEWLINE] "else" _expression,
    case_expression: "case" _expression "of" case_body,
    case_body: NEWLINE case_indented | case_alternative,
    case_indented: INDENT (case_alternative [NEWLINE])+ DEDENT,
    case_alternative: _pattern ["|" _expression] "->" _expression,

    do_expression: "do" NEWLINE do_block,
    do_block: INDENT (do_statement [NEWLINE])+ DEDENT,
    do_statement: &(_do_let_shape) do_let | &(_pattern "<-") do_bind | do_expression_statement,
    _do_let_shape: "let" _pattern* "=" _expression NEWLINE,
    do_let: "let" let_bindings,
    do_bind: _pattern "<-" _expression,
    do_expression_statement: _expression,

    let_expression: "let" let_bindings "in" _expression,
    let_bindings: let_inline | let_indented,
    let_inline: binding,
    let_indented: NEWLINE let_block,
    let_block: INDENT (binding [NEWLINE])+ DEDENT,
    binding: _pattern* "=" _expression,

    lambda: "\\" _pattern+ "->" _expression,

    // ---- types ----

    _type: precedence atom_type {
        constrained: right "=>" => constrained_type,
        function: right "->" => function_type,
        type_apply: postfix atom_type => type_application,
    },
    atom_type:
        _name
        | CONID
        | QUALIFIED
        | list_type
        | tuple_type,
    list_type: "[" _type "]",
    tuple_type: "(" (_type %? ",") ")",
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\x0c' | 0x0b => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0c' | 0x0b));
            WHITESPACE
        }
        b'\r' => {
            c.bump();
            c.eat(b'\n');
            NEWLINE
        }
        b'\n' => {
            c.bump();
            NEWLINE
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'-' if c.peek_at(1) == b'-' && !operator_byte(c.peek_at(2)) => {
            // `--` starts a comment unless it is part of a longer operator.
            lex::line_comment(c);
            COMMENT
        }
        b'{' if c.peek_at(1) == b'-' => {
            // A nested block comment, and a `{-# ... #-}` pragma with it.
            lex::block_comment(c, b"{-", b"-}", true);
            COMMENT
        }
        b'"' => if string(c) { STRING } else { BAD_LITERAL },
        b'\'' => character(c),
        b'0'..=b'9' => {
            lex::number(c);
            NUMBER
        }
        b'`' => {
            // An infix use of a name: ``a `div` b`` is one operator token.
            c.bump();
            while !c.at_end() && c.peek() != b'`' && !matches!(c.peek(), b'\n' | b'\r') {
                c.bump_char();
            }
            if c.eat(b'`') { OP } else { BAD_LITERAL }
        }
        b'A'..=b'Z' => conid(c),
        b'a'..=b'z' | b'_' if is_word_byte(c.peek()) => word(c),
        b'_' if !is_word_byte(c.peek_at(1)) => {
            c.bump();
            UNDERSCORE
        }
        byte if byte == b'_' || byte >= 0x80 => word(c),
        _ => operator(c),
    }
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'\'' || byte >= 0x80
}

/// A symbol byte: everything Haskell lets an operator be built from.
#[inline]
fn operator_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'#' | b'$' | b'%' | b'&' | b'*' | b'+' | b'.' | b'/' | b'<' | b'=' | b'>' | b'?' | b'@' | b'\\' | b'^' | b'|' | b'-' | b'~' | b':'
    )
}

/// A lower-case name, primes included (`foldl'`).
fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    keyword(&start[..length])
}

/// An upper-case name, or a qualified one: `Data.List.sort` is one token, so a
/// use site does not read its dot as composition.
fn conid(c: &mut Cursor) -> Kind {
    c.eat_while(is_word_byte);
    let mut qualified = false;
    while c.peek() == b'.' && is_word_start(c.peek_at(1)) {
        c.bump();
        c.eat_while(is_word_byte);
        qualified = true;
    }
    if qualified { QUALIFIED } else { CONID }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"as" => KW_AS,
        b"case" => KW_CASE,
        b"class" => KW_CLASS,
        b"data" => KW_DATA,
        b"default" => KW_DEFAULT,
        b"deriving" => KW_DERIVING,
        b"div" => KW_DIV,
        b"do" => KW_DO,
        b"else" => KW_ELSE,
        b"foreign" => KW_FOREIGN,
        b"hiding" => KW_HIDING,
        b"if" => KW_IF,
        b"import" => KW_IMPORT,
        b"in" => KW_IN,
        b"infix" => KW_INFIX,
        b"infixl" => KW_INFIXL,
        b"infixr" => KW_INFIXR,
        b"instance" => KW_INSTANCE,
        b"let" => KW_LET,
        b"mod" => KW_MOD,
        b"module" => KW_MODULE,
        b"newtype" => KW_NEWTYPE,
        b"not" => KW_NOT,
        b"of" => KW_OF,
        b"qualified" => KW_QUALIFIED,
        b"quot" => KW_QUOT,
        b"rem" => KW_REM,
        b"then" => KW_THEN,
        b"type" => KW_TYPE,
        b"where" => KW_WHERE,
        b"True" => KW_TRUE,
        b"False" => KW_FALSE,
        _ => IDENT,
    }
}

/// A character literal, or the bad-literal token for a lone quote.
fn character(c: &mut Cursor) -> Kind {
    if c.peek_at(1) == b'\\' {
        c.bump_n(2);
        while !c.at_end() {
            match c.peek() {
                b'\'' => {
                    c.bump();
                    return CHAR;
                }
                b'\n' | b'\r' => return BAD_LITERAL,
                _ => c.bump_char(),
            }
        }
        return BAD_LITERAL;
    }
    if c.peek_at(2) == b'\'' {
        c.bump_n(3);
        return CHAR;
    }
    c.bump();
    BAD_LITERAL
}

/// A string with escapes and `\`-newline gaps, which may span lines.
fn string(c: &mut Cursor) -> bool {
    c.bump();
    loop {
        if c.at_end() {
            return false;
        }
        match c.peek() {
            b'\\' => {
                c.bump();
                // A gap: `\` whitespace `\`.
                if matches!(c.peek(), b' ' | b'\t' | b'\n' | b'\r') {
                    c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
                    c.eat(b'\\');
                } else {
                    c.bump_char();
                }
            }
            b'"' => {
                c.bump();
                return true;
            }
            _ => c.bump_char(),
        }
    }
}

/// One operator token: the whole run of symbol bytes, named when the run is a
/// spelling the grammar knows and `operator` otherwise.
fn operator(c: &mut Cursor) -> Kind {
    match c.peek() {
        b'(' => {
            c.bump();
            LPAREN
        }
        b')' => {
            c.bump();
            RPAREN
        }
        b'[' => {
            c.bump();
            LBRACKET
        }
        b']' => {
            c.bump();
            RBRACKET
        }
        b'{' => {
            c.bump();
            LBRACE
        }
        b'}' => {
            c.bump();
            RBRACE
        }
        b',' => {
            c.bump();
            COMMA
        }
        b';' => {
            c.bump();
            SEMI
        }
        byte if operator_byte(byte) => {
            let start = c.pos();
            c.eat_while(operator_byte);
            known_operator(c.since(start))
        }
        _ => {
            c.bump_char();
            UNKNOWN
        }
    }
}

/// The kind of a run of operator bytes: its own token when the grammar names
/// that spelling, `operator` otherwise.
fn known_operator(text: &[u8]) -> Kind {
    match text {
        b"->" => ARROW,
        b"<-" => LARROW,
        b"=>" => FATARROW,
        b"::" => COLONCOLON,
        b".." => DOTDOT,
        b"||" => OROR,
        b"&&" => ANDAND,
        b"==" => EQUALS,
        b"/=" => NOTEQUALS,
        b"<=" => LE,
        b">=" => GE,
        b"++" => PLUSPLUS,
        b"<>" => DIAMOND,
        b">>" => SHR,
        b"<<" => SHL,
        b"=" => ASSIGN,
        b"|" => PIPE,
        b":" => COLON,
        b"@" => AT,
        b"~" => TILDE,
        b"\\" => BACKSLASH,
        b"." => DOT,
        b"$" => DOLLAR,
        b"+" => PLUS,
        b"-" => MINUS,
        b"*" => STAR,
        b"/" => SLASH,
        b"<" => LT,
        b">" => GT,
        b"^" => CARET,
        b"!" => BANG,
        b"%" => PERCENT,
        b"&" => AMP,
        _ => OP,
    }
}
