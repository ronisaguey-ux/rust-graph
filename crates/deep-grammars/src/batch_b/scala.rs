//! Scala through `grammar!`: one of Field 6's four languages (Ruby, PHP, Swift,
//! Scala).
//!
//! The grammar is the brace-and-parenthesis Scala 2/3 subset every source file
//! uses: `package`, `import`, `class`/`trait`/`object`/`enum`, `def`, `val` and
//! `var`, `if`/`while`/`for`/`match`/`try`, and one expression table. Node names
//! follow Tree-sitter's Scala where that costs nothing; the tree is not the
//! same tree.
//!
//! What the DSL covers here:
//!
//! - **Declarations share a modifier prefix.** `@deprecated private final case
//!   class` and `sealed trait` are entered through `&(definition_modifiers
//!   "class")` and friends, exactly as Java's grammar does.
//! - **`match` is a postfix row.** `x match { case ... }` is
//!   `match_expression`, and the row sits just below the lambda arrow so a
//!   `case` pattern (`_expression@or`) cannot swallow the `=>` that follows it.
//! - **A brace after an operand is an argument block.** `xs.map { x => ... }`
//!   and `xs.map { case ... }` are a postfix `block` row, which is safe in
//!   Scala because `if`, `for` and `while` all parenthesise their headers.
//! - **Strings are code at their fields.** A plain `"..."` is text, but
//!   `s"$name ${f(x)}"` is a `string_literal` over a start token, its text, one
//!   `interpolation` per `${...}` field and an end token: the mode stack
//!   (`modes`/`switch`) hands the bytes after the opening quote to the string
//!   step and the bytes inside a field back to code, so a call written in a
//!   field is an ordinary subtree. `$name` stays one `template_name` token, and
//!   `raw"..."` never opens a field.
//! - **`>` before `>` is `GT_JOINED`**, as in the four other grammars, so
//!   `Map[String, List[Int]]` closes both lists and `a >> b` and `a >>> b` are
//!   `GT_JOINED` runs.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **Indentation syntax (Scala 3) is not modelled.** `object X:` with a
//!   significant-indent body has no terminator, and the DSL describes a block
//!   by its brackets; the fixtures use braces. A source using the indentation
//!   form is the one valid Scala file this grammar cannot parse.
//! - **Operator characters are open-ended.** Scala lets a library define any
//!   operator out of punctuation. A grammar cannot enumerate that set, so the
//!   lexer takes the standard operators and the common assignments; an unknown
//!   run becomes `UNKNOWN`, which is an error, not a crash.
//! - **Identifier infix calls are not modelled.** `0 until 3` and `xs map f`
//!   put a *name* where the table expects an operator, and a name after an
//!   operand is also how the next statement starts, so no row is possible; the
//!   fixtures call `.until(3)`/`.map(f)`, which is the same call in Scala.
//!   `case` bodies are therefore `_statement*` while ordinary blocks are
//!   `_member*`, so a local `val` or nested `def` is a block member only.
//! - **A typed pattern's type cannot be a function type.** `case f: A => B` is
//!   legal Scala, but the `=>` before the case body is the same token, so a
//!   typed pattern takes a type without the arrow suffix.
//! - **A decision sees three tokens.** The modifier-prefixed declarations and
//!   the `case` pattern forms (`e: Type` against `e`) use `&(...)` predicates.
//! - **One kind per rule**, so `class_definition`, `trait_definition`,
//!   `object_definition` and `enum_definition` are four rules.

use std::cell::RefCell;

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

thread_local! {
    /// The quote of each string the `STRING` mode is inside, innermost last,
    /// with whether it is triple-quoted and whether its fields are code (an
    /// `s"..."`/`f"..."` interpolator) or text (a plain or `raw"..."` string).
    static QUOTES: RefCell<Vec<(bool, bool)>> = const { RefCell::new(Vec::new()) };
    /// Open brackets inside each `${...}` field, innermost last, so the `}`
    /// that closes a field is the one at its own depth.
    static FIELDS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

rust_graph_deep::grammar! { SCALA {
    name = "scala";
    lex = lex;

    /// After an interpolator's opening quote the step lexer runs in `STRING`:
    /// the bytes are text until the closing quote or a `$`.
    ///
    /// `TEMPLATE` is the opposite frame: the lexer pushed it at a `${`, so the
    /// bytes are code again until the `}` that matches that brace at the
    /// field's own bracket depth pops it. A plain string never opens a field,
    /// and `raw"..."` keeps its dollars as text.
    modes {
        STRING = lex_string_text,
        TEMPLATE = lex_template_field,
    }
    switch STRING_START => push STRING;
    switch STRING_END => pop;
    switch STRING_BAD => pop STRING;
    switch TEMPLATE_OPEN => push TEMPLATE;
    switch TEMPLATE_CLOSE => pop;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        NUMBER = "integer",
        FLOAT = "float",
        /// The opening quotes of a string, `"` or `"""`.
        STRING_START = "string_start",
        /// The bytes of a string between a template and the next one.
        STRING_TEXT = "string_text",
        /// The `${` of an interpolated field: code again until the matching `}`.
        TEMPLATE_OPEN = "template_open" open field,
        /// The `}` that closes a template field at its own bracket depth.
        TEMPLATE_CLOSE = "template_close" close field,
        /// `$name`, which is one token: there is nothing to recurse into.
        SIMPLE_TEMPLATE = "template_name",
        /// The closing quotes of a string.
        STRING_END = "string_end",
        /// A literal that does not close on its line.
        STRING_BAD = "bad_string" bad,
        CHAR = "character",
        SYMBOL = "symbol",
        /// A string, character or symbol literal with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_ABSTRACT = "abstract",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CLASS = "class",
        KW_DEF = "def",
        KW_DO = "do",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXTENDS = "extends",
        KW_FALSE = "false",
        KW_FINALLY = "finally",
        KW_FOR = "for",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_IMPLICIT = "implicit",
        KW_LAZY = "lazy",
        KW_MATCH = "match",
        KW_NEW = "new",
        KW_NULL = "null",
        KW_OBJECT = "object",
        KW_OVERRIDE = "override",
        KW_PACKAGE = "package",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_RETURN = "return",
        KW_SEALED = "sealed",
        KW_SUPER = "super",
        KW_THIS = "this",
        KW_THROW = "throw",
        KW_TRAIT = "trait",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_TYPE = "type",
        KW_VAL = "val",
        KW_VAR = "var",
        KW_WHILE = "while",
        KW_WITH = "with",
        KW_FINAL = "final",
        KW_OPEN = "open",
        KW_INLINE = "inline",
        KW_TRANSPARENT = "transparent",
        KW_INFIX = "infix",
        KW_GIVEN = "given",
        KW_USING = "using",
        KW_EXPORT = "export",
        KW_YIELD = "yield",

        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        COLON = ":",
        COLONCOLON = "::",
        ASSIGN = "=",
        ARROW = "=>",
        LARROW = "<-",
        RIGHT_ARROW = "->",
        AT = "@",
        UNDERSCORE_DOT = "_.",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        EQ = "==",
        NE = "!=",
        LE = "<=",
        GE = ">=",
        ANDAND = "&&",
        OROR = "||",
        SHL = "<<",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        TILDE = "~",
        BANG = "!",
        LT = "<",
        GT = ">",
        /// A `>` right before another `>`: the first half of `>>`, `>>>`, `>>=`
        /// or a type argument list closing right before another.
        GT_JOINED = ">joined",
        PLUSPLUS = "++",
        MINUSMINUS = "--",
        BOUND = "<:",
        LOWER_BOUND = ">:",
    }

    program: _top*,

    _top: package_clause | import_declaration | _member,

    package_clause: "package" _qualified_name,
    import_declaration: "import" (_import_expr % ","),
    /// An import path is segments to a name, or to `*`, or to a `{...}` group of
    /// selectors. The loop is the same shape as `_qualified_name`'s, because an
    /// optional bracket after a dotted name is not taken.
    _import_expr: _name ("." (_name | "*" | import_selectors))*,
    /// `import scala.util.{Failure, Success, Try}` and the rename form
    /// `{Failure => Bad}`.
    import_selectors: "{" (_import_selector %? ",") "}",
    _import_selector: _name ["=>" _name],
    _qualified_name: _name ("." _name)*,

    _member: _definition | _statement,

    // ---- definitions ----

    definition_modifiers: (annotation | modifier)+,
    modifier:
        "private" | "protected" | "override" | "final" | "sealed" | "abstract" | "lazy"
        | "implicit" | "case" | "open" | "inline" | "transparent" | "infix",
    annotation: "@" _qualified_name [annotation_arguments],
    annotation_arguments: "(" (_expression %? ",") ")",

    _definition:
        &([definition_modifiers] "class") class_definition
        | &([definition_modifiers] "trait") trait_definition
        | &([definition_modifiers] "object") object_definition
        | &([definition_modifiers] "enum") enum_definition
        | &([definition_modifiers] "def") def_definition
        | &([definition_modifiers] ("val" | "var")) val_definition
        | &([definition_modifiers] "given") given_definition
        | &([definition_modifiers] "type") type_definition,

    class_definition: [definition_modifiers] "class" _name [type_parameters] [class_parameters] [extends_clause] [template_body],
    trait_definition: [definition_modifiers] "trait" _name [type_parameters] [extends_clause] [template_body],
    object_definition: [definition_modifiers] "object" _name [extends_clause] [template_body],
    enum_definition: [definition_modifiers] "enum" _name [type_parameters] [class_parameters] [extends_clause] [template_body],
    def_definition: [definition_modifiers] "def" _name [type_parameters] parameter_clause* [":" _type] [def_body],
    def_body: "=" (block | _expression) | block,
    val_definition: [definition_modifiers] ("val" | "var") (_binding % ","),
    _binding: _name [":" _type] ["=" _expression],
    /// Scala 3's `given`: a named instance (`given intShow: Show[Int] with
    /// {...}`) or an anonymous one (`given Show[String] = ...`). The two start
    /// the same way — a name or a type — so the named form is entered through
    /// the predicate that sees the `:` after its head.
    given_definition: [definition_modifiers] "given" _given_head,
    _given_head:
        &(_given_named ":") _given_named ":" _given_type ["with" template_body] ["=" _expression]
        | _given_anonymous,
    _given_named: _name [type_parameters] [parameter_clause],
    _given_anonymous: _given_type ["with" template_body] ["=" _expression],
    /// A `given`'s type stops before `with`: that keyword starts the instance's
    /// body here, not a compound type, so this is `_type` without the `with`
    /// suffix.
    _given_type: _type_atom (type_arguments | "*" | "=>" _type | "&" _type | "|" _type)*,
    type_definition: [definition_modifiers] "type" _name [type_parameters] "=" _type,

    class_parameters: "(" (class_parameter %? ",") ")",
    class_parameter: [definition_modifiers] [_class_parameter_kind] _name ":" _type ["=" _expression],
    _class_parameter_kind: "val" | "var",
    parameter_clause: "(" (parameter %? ",") ")",
    parameter: [_parameter_kind] _name ":" _type ["=" _expression],
    _parameter_kind: "implicit" | "using",

    extends_clause: "extends" _type,
    template_body: "{" _member* "}",
    type_parameters: "[" (type_parameter %? ",") "]",
    type_parameter: ["+" | "-"] _name [(":" | "<:" | ">:") _type],

    // ---- statements ----

    /// `if`, `for`, `while` and `try` are expressions here, so a statement that
    /// is one is reached through `expression_statement`: two alternatives that
    /// start the same way cannot be told apart.
    _statement:
        do_statement
        | return_statement
        | throw_statement
        | block
        | ";"
        | expression_statement,

    /// A block may hold definitions too: local `val`s and nested `def`s are
    /// ordinary Scala, and `f { case ... }` is a partial-function literal.
    block: "{" (_member | case_clause)* "}",
    expression_statement: _expression,
    if_statement: "if" parenthesized_expression _statement ["else" _statement],
    while_statement: "while" parenthesized_expression _statement,
    do_statement: "do" _statement "while" parenthesized_expression,
    for_statement: "for" for_header ["yield"] _statement,
    for_header: "(" _enumerators ")",
    _enumerators: _enumerator (";" _enumerator)*,
    _enumerator: generator | _expression@lambda,
    /// A guard binds to the generator it follows (`x <- xs if p`), so it is a
    /// suffix of the generator and not an enumerator of its own: a leading
    /// `if` would start an `if` expression and the two could not be told apart.
    generator: _name "<-" _expression@assign ("if" _expression@or)*,
    try_statement: "try" try_body catch_clause* [finally_clause],
    try_body: block | _expression,
    catch_clause: "catch" catch_block,
    catch_block: "{" case_clause* "}",
    finally_clause: "finally" try_body,
    return_statement: "return" [_expression],
    throw_statement: "throw" _expression,

    /// A case body holds statements and local definitions. `_member*` cannot be
    /// used: `case` is also a declaration modifier (`case class`), so the list
    /// would swallow the next `case` clause as a broken definition. The local
    /// definitions that a case body really holds are named instead.
    case_clause: "case" _pattern ["if" _expression@or] "=>" (_statement | _local_definition)*,
    _local_definition:
        &("val") _definition | &("var") _definition | &("def") _definition
        | &("type") _definition | &("class") _definition | &("object") _definition
        | &("trait") _definition,
    _pattern: &(_name ":") typed_pattern | _expression@or,
    typed_pattern: _name ":" _pattern_type,
    /// A typed pattern's type, without the `=>` suffix: the case arrow is not a
    /// function type.
    _pattern_type: _type_atom (type_arguments | "with" _type)*,
    match_block: "{" case_clause* "}",

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | GT_JOINED ">=" => assignment,
        lambda: right "=>" => lambda_expression,
        match: postfix "match" match_block => match_expression,
        or: left "||" => binary,
        and: left "&&" => binary,
        bor: left "|" => binary,
        xor: left "^" => binary,
        band: left "&" => binary,
        eq: left "==" | "!=" => binary,
        cmp: left "<" | ">" | "<=" | ">=" => binary,
        cons: right "::" => binary,
        pair: right "->" => pair_expression,
        concat: right "++" | "--" => binary,
        shift: left "<<" | GT_JOINED ">" | GT_JOINED GT_JOINED ">" => binary,
        add: left "+" | "-" => binary,
        mul: left "*" | "/" | "%" => binary,
        unary: prefix "-" | "+" | "!" | "~" => unary,
        block_call: postfix block => block_argument,
        call: postfix arguments => call,
        index: postfix subscript => subscript_expression,
        select: postfix "." _name => select,
    },

    _primary:
        NUMBER | FLOAT | CHAR | SYMBOL
        | string_literal
        | "true" | "false" | "null"
        | "this" | "super"
        | _name
        | parenthesized_expression
        | collection_literal
        | new_expression
        | if_statement
        | try_statement
        | for_statement
        | while_statement,

    string_literal: STRING_START _string_part* STRING_END,
    _string_part: STRING_TEXT | interpolation | SIMPLE_TEMPLATE,
    interpolation: TEMPLATE_OPEN _expression TEMPLATE_CLOSE,

    _name: IDENT,
    /// `(x)` or `(a, b)`: a parenthesised expression or a tuple, and a typed
    /// lambda parameter list (`(value: String) => ...`), which is the same
    /// brackets with a binding after the name.
    parenthesized_expression: "(" (_paren_item %? ",") ")",
    _paren_item: &(_name ":") typed_binding | _expression@assign,
    typed_binding: _name ":" _type,
    arguments: "(" (_argument %? ",") ")",
    _argument: &(_name "=") named_argument | _expression@assign,
    named_argument: _name "=" _expression@assign,
    subscript: "[" (_expression %? ",") "]",
    collection_literal: "[" (_expression %? ",") "]",
    new_expression: "new" _type,

    // ---- types ----

    _type: _type_atom _type_suffix*,
    _type_suffix: type_arguments | "*" | "=>" _type | "with" _type | "&" _type | "|" _type,
    _type_atom: _name | tuple_type,
    tuple_type: "(" (_type %? ",") ")",
    type_arguments: "[" (_type %? ",") "]",
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    if c.pos() == 0 {
        QUOTES.with(|quotes| quotes.borrow_mut().clear());
        FIELDS.with(|fields| fields.borrow_mut().clear());
    }
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
            WHITESPACE
        }
        0xef if c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n' && byte != b'\r');
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            block_comment(c);
            COMMENT
        }
        b'"' => start_string(c, false),
        b'\'' => quoted_single(c),
        b'`' => {
            c.bump();
            c.eat_while(|byte| byte != b'`' && byte != b'\n');
            c.eat(b'`');
            IDENT
        }
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() => number(c),
        byte if is_word_start(byte) => word(c),
        _ => operator(c),
    }
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    // `s"..."`, `f"..."` and `raw"..."` are interpolators, not names: the
    // first two make their fields code, `raw` keeps its dollars as text.
    if matches!(text, b"s" | b"f" | b"raw") && c.peek() == b'"' {
        return start_string(c, !matches!(text, b"raw"));
    }
    keyword(text)
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"abstract" => KW_ABSTRACT,
        b"case" => KW_CASE,
        b"catch" => KW_CATCH,
        b"class" => KW_CLASS,
        b"def" => KW_DEF,
        b"do" => KW_DO,
        b"else" => KW_ELSE,
        b"enum" => KW_ENUM,
        b"extends" => KW_EXTENDS,
        b"false" => KW_FALSE,
        b"finally" => KW_FINALLY,
        b"for" => KW_FOR,
        b"if" => KW_IF,
        b"import" => KW_IMPORT,
        b"implicit" => KW_IMPLICIT,
        b"lazy" => KW_LAZY,
        b"match" => KW_MATCH,
        b"new" => KW_NEW,
        b"null" => KW_NULL,
        b"object" => KW_OBJECT,
        b"override" => KW_OVERRIDE,
        b"package" => KW_PACKAGE,
        b"private" => KW_PRIVATE,
        b"protected" => KW_PROTECTED,
        b"return" => KW_RETURN,
        b"sealed" => KW_SEALED,
        b"super" => KW_SUPER,
        b"this" => KW_THIS,
        b"throw" => KW_THROW,
        b"trait" => KW_TRAIT,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"type" => KW_TYPE,
        b"val" => KW_VAL,
        b"var" => KW_VAR,
        b"while" => KW_WHILE,
        b"with" => KW_WITH,
        b"final" => KW_FINAL,
        b"open" => KW_OPEN,
        b"inline" => KW_INLINE,
        b"transparent" => KW_TRANSPARENT,
        b"infix" => KW_INFIX,
        b"given" => KW_GIVEN,
        b"using" => KW_USING,
        b"export" => KW_EXPORT,
        b"yield" => KW_YIELD,
        _ => IDENT,
    }
}

/// `/* ... */`: Scala's block comments nest.
fn block_comment(c: &mut Cursor) {
    c.bump_n(2);
    let mut depth = 1u32;
    while !c.at_end() && depth > 0 {
        if c.starts_with(b"/*") {
            depth += 1;
            c.bump_n(2);
        } else if c.starts_with(b"*/") {
            depth -= 1;
            c.bump_n(2);
        } else {
            c.bump();
        }
    }
}

/// An opening quote (an interpolator's prefix is already consumed): remember
/// how the body closes and whether its fields are code, and emit the start.
fn start_string(c: &mut Cursor, code: bool) -> Kind {
    if c.peek() != b'"' {
        return BAD_LITERAL;
    }
    let triple = c.peek_at(1) == b'"' && c.peek_at(2) == b'"';
    c.bump_n(if triple { 3 } else { 1 });
    QUOTES.with(|quotes| quotes.borrow_mut().push((triple, code)));
    STRING_START
}

/// The innermost string's (triple, code), if the mode stack and this state
/// still agree.
fn quotes() -> Option<(bool, bool)> {
    QUOTES.with(|quotes| quotes.borrow().last().copied())
}

fn pop_string() {
    QUOTES.with(|quotes| {
        quotes.borrow_mut().pop();
    });
}

/// The `STRING` mode's step: the bytes between fields, the `$` that starts one
/// (in an interpolator), or the closing quotes. A one-line string that reaches
/// its line end is `STRING_BAD`, which costs its line and pops the mode.
pub fn lex_string_text(c: &mut Cursor) -> Kind {
    let Some((triple, code)) = quotes() else {
        // The mode stack and this state disagree: lexing code cannot lose a
        // token, and the parser still gets a tree.
        return lex(c);
    };
    if c.at_end() {
        pop_string();
        return STRING_BAD;
    }
    if triple {
        if c.starts_with(b"\"\"\"") {
            c.bump_n(3);
            pop_string();
            return STRING_END;
        }
    } else if c.peek() == b'"' {
        c.bump();
        pop_string();
        return STRING_END;
    }
    if !triple && matches!(c.peek(), b'\n' | b'\r') {
        c.eat(b'\r');
        c.bump();
        pop_string();
        return STRING_BAD;
    }
    if code && c.peek() == b'$' && c.peek_at(1) == b'{' {
        c.bump_n(2);
        // The field's expression is code: the mode stack returns to code here
        // and `lex_template_field` closes it at the brace matching this one.
        FIELDS.with(|fields| fields.borrow_mut().push(0));
        return TEMPLATE_OPEN;
    }
    if code && c.peek() == b'$' && is_word_start(c.peek_at(1)) {
        c.bump();
        c.eat_while(is_word_byte);
        return SIMPLE_TEMPLATE;
    }
    // Text up to a field, the closing quotes, or a line end in a one-line
    // string. `$$` is an escaped dollar and stays text.
    let start = c.pos();
    while !c.at_end() {
        let byte = c.peek();
        if triple {
            if c.starts_with(b"\"\"\"") {
                break;
            }
        } else if byte == b'"' || matches!(byte, b'\n' | b'\r') {
            break;
        }
        if code && byte == b'$' {
            if c.peek_at(1) == b'$' {
                c.bump_n(2);
                continue;
            }
            break;
        }
        if byte == b'\\' {
            c.bump_n(2);
        } else {
            c.bump();
        }
    }
    if c.pos() == start {
        // A byte the checks above did not take: consume it so every step
        // advances.
        c.bump();
    }
    STRING_TEXT
}

/// The `TEMPLATE` mode's step: ordinary code, except that the `}` matching the
/// field's own `${` closes it, and a one-line string's field cannot outlive its
/// line.
pub fn lex_template_field(c: &mut Cursor) -> Kind {
    let Some(depth) = FIELDS.with(|fields| fields.borrow().last().copied()) else {
        return lex(c);
    };
    if c.at_end() {
        pop_string();
        FIELDS.with(|fields| fields.borrow_mut().pop());
        return STRING_BAD;
    }
    if depth == 0 && c.peek() == b'}' {
        c.bump();
        FIELDS.with(|fields| fields.borrow_mut().pop());
        return TEMPLATE_CLOSE;
    }
    let kind = lex(c);
    match kind {
        LPAREN | LBRACKET | LBRACE => FIELDS.with(|fields| {
            if let Some(depth) = fields.borrow_mut().last_mut() {
                *depth += 1;
            }
        }),
        RPAREN | RBRACKET | RBRACE => FIELDS.with(|fields| {
            if let Some(depth) = fields.borrow_mut().last_mut() {
                *depth = depth.saturating_sub(1);
            }
        }),
        _ => {}
    }
    kind
}

/// A character literal (`'a'`) or a symbol (`'name`).
fn quoted_single(c: &mut Cursor) -> Kind {
    c.bump();
    if is_word_start(c.peek()) {
        c.eat_while(is_word_byte);
        if c.peek() == b'\'' {
            c.bump();
            return CHAR;
        }
        return SYMBOL;
    }
    loop {
        match c.peek() {
            0 if c.at_end() => return BAD_LITERAL,
            b'\\' => c.bump_n(2),
            b'\n' | b'\r' => return BAD_LITERAL,
            b'\'' => {
                c.bump();
                return CHAR;
            }
            _ => c.bump(),
        }
    }
}

fn number(c: &mut Cursor) -> Kind {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return NUMBER;
    }
    let mut float = false;
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.peek() == b'.' && c.peek_at(1).is_ascii_digit() {
        float = true;
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.peek(), b'e' | b'E') && (c.peek_at(1).is_ascii_digit() || (matches!(c.peek_at(1), b'+' | b'-') && c.peek_at(2).is_ascii_digit())) {
        float = true;
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if float {
        eat_suffix(c, b"fFdD");
        FLOAT
    } else {
        eat_suffix(c, b"lL");
        NUMBER
    }
}

/// A numeric suffix such as `L`, `f` or `d`.
fn eat_suffix(c: &mut Cursor, letters: &[u8]) {
    if letters.contains(&c.peek()) {
        c.bump();
    }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'<', b'-', _) => (LARROW, 2),
        (b'=', b'>', _) => (ARROW, 2),
        (b'-', b'>', _) => (RIGHT_ARROW, 2),
        (b'<', b':', _) => (BOUND, 2),
        (b'>', b':', _) => (LOWER_BOUND, 2),
        (b':', b':', _) => (COLONCOLON, 2),
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'<', b'<', _) => (SHL, 2),
        (b'+', b'+', _) => (PLUSPLUS, 2),
        (b'-', b'-', _) => (MINUSMINUS, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'>', b'>', _) => (GT_JOINED, 1),
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
        (b'@', ..) => (AT, 1),
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::deep::syntax::ERROR;
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    fn parse(src: &str) -> Workspace {
        let mut workspace = Workspace::default();
        workspace.parse(src.as_bytes(), &SCALA, &Budget::default()).unwrap_or_else(|_| panic!("input too large"));
        workspace
    }

    fn sexp(src: &str) -> String {
        parse(src).tree.sexp(&SCALA, true)
    }

    fn clean(src: &str) -> bool {
        parse(src).tree.root().flags & HAS_ERROR == 0
    }

    fn errors(src: &str) -> usize {
        let workspace = parse(src);
        let nodes = &workspace.tree.nodes;
        (0..nodes.len()).filter(|&index| nodes[index].kind == ERROR || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0).count()
    }

    fn tokens(src: &str) -> Vec<&'static str> {
        let workspace = parse(src);
        (0..workspace.tokens.len())
            .filter(|&at| !SCALA.is_trivia(workspace.tokens.kind[at]))
            .map(|at| SCALA.name(workspace.tokens.kind[at]))
            .collect()
    }

    /// An interpolated string's `${...}` field is code: the tokens after
    /// `template_open` are the expression's own, and the byte after its `}` is
    /// string text again.
    #[test]
    fn an_interpolated_field_is_code() {
        assert_eq!(
            tokens("val s = s\"n=${items.size}\"\n"),
            [
                "val", "identifier", "=", "string_start", "string_text", "template_open", "identifier", ".",
                "identifier", "template_close", "string_end",
            ]
        );
        assert_eq!(
            tokens("val s = s\"$name and ${f(x)}\"\n"),
            [
                "val", "identifier", "=", "string_start", "template_name", "string_text", "template_open",
                "identifier", "(", "identifier", ")", "template_close", "string_end",
            ]
        );
    }

    /// The call written in a field is an ordinary node with its own span, so a
    /// caller can see through the interpolation.
    #[test]
    fn a_field_holds_a_call_node_with_its_span() {
        let src = "val s = s\"x${f(a, 1)}y\"\n";
        let dump = sexp(src);
        assert!(dump.contains("(interpolation"), "{dump}");
        assert!(dump.contains("(call"), "{dump}");
        let at = src.find("f(a, 1)").unwrap() as u32;
        let workspace = parse(src);
        let nodes = &workspace.tree.nodes;
        let call = (0..nodes.len())
            .find(|&index| SCALA.name(nodes[index].kind) == "call")
            .unwrap_or_else(|| panic!("no call node in {dump}"));
        assert_eq!((nodes[call].start, nodes[call].end), (at, at + 7), "the call's span is its own bytes:\n{dump}");
    }

    /// `raw"..."` is text: its dollars never open a field. A plain string never
    /// does either.
    #[test]
    fn only_an_interpolator_opens_a_field() {
        assert_eq!(tokens("val s = raw\"$x\"\n"), ["val", "identifier", "=", "string_start", "string_text", "string_end"]);
        assert_eq!(tokens("val s = \"$x\"\n"), ["val", "identifier", "=", "string_start", "string_text", "string_end"]);
        assert!(clean("object A { val s = s\"\"\"\n|${name}\n\"\"\".stripMargin }\n"));
    }

    /// `->` is a pair, so a map built from tuples parses.
    #[test]
    fn a_right_arrow_builds_a_pair() {
        assert_eq!(tokens("a -> b\n"), ["identifier", "->", "identifier"]);
        assert!(clean("object A { val m = Map(\"a\" -> 1) }\n"));
        assert!(sexp("val p = a -> b\n").contains("(pair_expression"), "{}", sexp("val p = a -> b\n"));
    }

    /// A `given` instance, named or anonymous, with a `with` body or an `=`.
    #[test]
    fn a_given_instance_parses() {
        let src = "object A {\n  given intShow: Show[Int] with {\n    def show(value: Int): String = value.toString\n  }\n  given Show[String] = (value: String) => value\n  def render[A](value: A)(using show: Show[A]): String = show.show(value)\n}\n";
        assert!(clean(src), "{}", sexp(src));
        assert!(sexp("given intShow: Show[Int] with { }\n").contains("(given_definition"));
    }

    /// A `for` with a guard and `yield` is an expression, so it can be the body
    /// of a `def`.
    #[test]
    fn a_for_comprehension_is_an_expression() {
        let src = "def cheap(limit: Double): List[String] = for (item <- items if item.price < limit) yield item.name\n";
        assert!(clean(src), "{}", sexp(src));
        assert!(sexp(src).contains("(for_statement"), "{}", sexp(src));
        assert!(sexp(src).contains("(generator identifier"), "{}", sexp(src));
    }

    /// `if` and `try` are expressions, so they can be a `val`'s value or a
    /// `def`'s body.
    #[test]
    fn if_and_try_are_expressions() {
        assert!(clean("val x = if (a) b else c\n"), "{}", sexp("val x = if (a) b else c\n"));
        assert!(clean("val x = try f() catch { case _: E => g() }\n"), "{}", sexp("val x = try f() catch { case _: E => g() }\n"));
        assert!(clean("def f(a: Boolean): Int = if (a) 1 else 2\n"));
    }

    /// A field that never closes costs its own line, not the file: the root
    /// survives and the error is inside it.
    #[test]
    fn a_broken_field_is_an_error_inside_the_root() {
        let src = "object A {\n  val s = s\"${f(\"\n  val t = 1\n}\n";
        let workspace = parse(src);
        assert!(workspace.tree.root().flags & HAS_ERROR != 0 || errors(src) > 0, "a broken field must be an error");
        assert_eq!(SCALA.name(workspace.tree.root().kind), "program");
        assert!(errors(src) > 0);
    }

    fn corpus() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join("scala_corpus")
    }

    /// Malformed and truncated input must give a tree with the error inside it,
    /// never a panic and never a hang: every prefix of every corpus file at a
    /// character boundary is parsed here.
    #[test]
    fn every_prefix_of_a_corpus_file_still_builds_its_tree() {
        let dir = corpus();
        let mut names: Vec<String> = fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in &names {
            let src = fs::read_to_string(dir.join(name)).unwrap();
            let cuts: Vec<usize> = src.char_indices().map(|(at, _)| at).filter(|at| at % 5 == 0).chain(std::iter::once(src.len())).collect();
            for at in cuts {
                let head = &src[..at];
                let workspace = parse(head);
                assert_eq!(SCALA.name(workspace.tree.root().kind), "program", "{name} at {at}: the root must survive");
            }
        }
    }

    /// Real files, not the eight hand-written fixtures: every one in
    /// `fixtures/scala_corpus` must parse with no error node. The count is the
    /// measurement the report quotes.
    #[test]
    fn scala_corpus_is_clean() {
        let dir = corpus();
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{dir:?}: {error}"))
            .filter(|entry| entry.as_ref().map(|entry| entry.file_type().map(|kind| kind.is_file()).unwrap_or(false)).unwrap_or(false))
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert!(names.len() >= 8, "the corpus is at least eight realistic files, not {}", names.len());
        let mut dirty = Vec::new();
        for name in &names {
            let text = fs::read_to_string(dir.join(name)).unwrap();
            if !clean(&text) {
                dirty.push((name.clone(), errors(&text)));
            }
        }
        println!("scala_corpus: {}/{} clean", names.len() - dirty.len(), names.len());
        if !dirty.is_empty() {
            let mut report = String::new();
            for (name, count) in &dirty {
                let text = fs::read_to_string(dir.join(name)).unwrap();
                report.push_str(&format!("\n{name}: {count} error nodes\n{}\n", sexp(&text)));
            }
            panic!("{} of {} corpus files are not clean:\n{report}", dirty.len(), names.len());
        }
    }
}
