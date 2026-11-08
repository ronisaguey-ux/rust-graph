//! Visual Basic: line-oriented modules, classes and procedures.
//!
//! VB is defined by its lines. The lexer emits a `NEWLINE` token for every
//! physical line end (`_` before it joins the next line instead), `'` starts a
//! comment to the end of the line, and keywords are matched after folding to
//! upper case, because VB does not care about their case.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **A single-line `If x Then y = 1` is not modelled.** A statement body is a
//!   run of lines that ends at `End If`, so the one-line form has nowhere to go
//!   and is reported as an error rather than silently accepted.
//! - **Attributes (`<Serializable>`)** are not modelled; `<` lexes as an
//!   operator and the line is an error.
//! - **`=` is one operator.** VB uses it for assignment and for equality, so
//!   `If x = 1 Then` contains an `assignment` node in this tree. The line is
//!   parsed the way VB's own grammar would parse it; only the node's name is
//!   misleading.
//! - **Generics (`List(Of String)`)** are not modelled: `Of` is a keyword but
//!   the bracketed type argument list is not a rule.
//! - **Handles/WithEvents/Property` bodies** are not modelled.
//! - **A `#date#` literal** is one token, so its separators are not checked.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { VISUAL_BASIC {
    name = "visual-basic";
    lex = lex_vb;

    tokens {
        NEWLINE = "newline",
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        COMMA = ",",
        DOT = ".",
        COLON = ":",
        ASSIGN = "=",
        EQ = "==",
        NE = "<>",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        BACKSLASH = "\\",
        CARET = "^",
        AMP = "&",
        QUESTION = "?",
        HASH = "#",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        DATE = "date",
        BAD_STRING = "bad_string" bad,

        KW_IMPORTS = "Imports",
        KW_NAMESPACE = "Namespace",
        KW_MODULE = "Module",
        KW_CLASS = "Class",
        KW_STRUCTURE = "Structure",
        KW_INTERFACE = "Interface",
        KW_ENUM = "Enum",
        KW_SUB = "Sub",
        KW_FUNCTION = "Function",
        KW_PROPERTY = "Property",
        KW_GET = "Get",
        KW_SET = "Set",
        KW_DIM = "Dim",
        KW_CONST = "Const",
        KW_AS = "As",
        KW_NEW = "New",
        KW_RETURN = "Return",
        KW_IF = "If",
        KW_THEN = "Then",
        KW_ELSEIF = "ElseIf",
        KW_ELSE = "Else",
        KW_END = "End",
        KW_FOR = "For",
        KW_TO = "To",
        KW_STEP = "Step",
        KW_NEXT = "Next",
        KW_EACH = "Each",
        KW_IN = "In",
        KW_WHILE = "While",
        KW_DO = "Do",
        KW_LOOP = "Loop",
        KW_UNTIL = "Until",
        KW_SELECT = "Select",
        KW_CASE = "Case",
        KW_IS = "Is",
        KW_TRY = "Try",
        KW_CATCH = "Catch",
        KW_FINALLY = "Finally",
        KW_THROW = "Throw",
        KW_EXIT = "Exit",
        KW_CONTINUE = "Continue",
        KW_WITH = "With",
        KW_USING = "Using",
        KW_BYVAL = "ByVal",
        KW_BYREF = "ByRef",
        KW_OPTIONAL = "Optional",
        KW_PUBLIC = "Public",
        KW_PRIVATE = "Private",
        KW_PROTECTED = "Protected",
        KW_FRIEND = "Friend",
        KW_SHARED = "Shared",
        KW_STATIC = "Static",
        KW_READONLY = "ReadOnly",
        KW_OVERRIDES = "Overrides",
        KW_MUSTINHERIT = "MustInherit",
        KW_NOTHING = "Nothing",
        KW_TRUE = "True",
        KW_FALSE = "False",
        KW_AND = "And",
        KW_OR = "Or",
        KW_NOT = "Not",
        KW_XOR = "Xor",
        KW_MOD = "Mod",
        KW_LIKE = "Like",
        KW_ANDALSO = "AndAlso",
        KW_ORELSE = "OrElse",
        KW_OF = "Of",
        KW_HANDLES = "Handles",
        KW_WHEN = "When",
        KW_ME = "Me",
    }

    source: _line*,

    _line: NEWLINE | _statement,

    _statement:
        import_statement
        | namespace_block
        | unmodified_declaration
        | declaration
        | enum_block
        | variable_statement
        | constant_statement
        | if_statement
        | for_statement
        | for_each_statement
        | while_statement
        | do_statement
        | select_statement
        | try_statement
        | with_statement
        | using_statement
        | return_statement
        | throw_statement
        | exit_statement
        | continue_statement
        | &(IDENT "=") assignment_statement
        | call_statement,

    import_statement: "Imports" dotted_name NEWLINE,
    dotted_name: _name ("." _name)*,
    namespace_block: "Namespace" dotted_name NEWLINE _line* "End" "Namespace" NEWLINE,

    type_keyword: "Class" | "Module" | "Structure" | "Interface",
    enum_block: "Enum" _name NEWLINE enum_member* "End" "Enum" NEWLINE,
    enum_member: _name ["=" _expression] NEWLINE,

    _modifier:
        "Public"
        | "Private"
        | "Protected"
        | "Friend"
        | "Shared"
        | "ReadOnly"
        | "Overrides"
        | "MustInherit",

    /// A class, module, `Sub`/`Function`/`Property` or field. All four may open
    /// with the same modifier (`Public Class`, `Public Sub`, `Public Title As
    /// String`), so the prefix is one rule and the kind inside it says which
    /// form was read. Splitting them would need a predicate naming a modifier,
    /// and a predicate may not.
    declaration: _modifier+ declaration_kind,
    unmodified_declaration: type_block_body | routine_definition,
    declaration_kind: type_block_body | routine_definition | field_declaration,
    type_block_body: type_keyword _name NEWLINE _line* "End" type_keyword NEWLINE,
    routine_definition: routine_head NEWLINE _line* "End" routine_end NEWLINE,
    field_declaration: declarator ("," declarator)* NEWLINE,

    /// `Sub`, `Function` and `Property` share one head rule: they differ only in
    /// the keyword, and a separate rule each would have the same first set as
    /// the others, which the DSL refuses. The `End` keyword is not checked
    /// against the head, so `End Function` may close a `Sub` here.
    routine_head:
        "Sub" "New" [parameter_list]
        | "Sub" _name [parameter_list]
        | "Function" _name [parameter_list] ["As" _type]
        | "Property" _name [parameter_list] ["As" _type],
    routine_end: "Sub" | "Function" | "Property",

    parameter_list: "(" [_parameter %? ","] ")",
    _parameter: ["Optional"] ["ByVal" | "ByRef"] _name ["As" _type] ["=" _expression],

    _type: _name ["." _name],

    variable_statement: ("Dim" | "Static") declarator ("," declarator)* NEWLINE,
    constant_statement: "Const" _name ["As" _type] "=" _expression NEWLINE,
    declarator: _name ["As" _type] ["=" _expression],
    object_creation: "New" _name [argument_list],

    if_statement: "If" _expression "Then" NEWLINE _line* elseif_clause* [else_clause] "End" "If" NEWLINE,
    elseif_clause: "ElseIf" _expression "Then" NEWLINE _line*,
    else_clause: "Else" NEWLINE _line*,

    for_statement:
        "For" _name ["As" _type] "=" _expression "To" _expression ["Step" _expression] NEWLINE _line* "Next" [_name] NEWLINE,
    for_each_statement:
        "For" "Each" _name ["As" _type] "In" _expression NEWLINE _line* "Next" [_name] NEWLINE,

    while_statement: "While" _expression NEWLINE _line* "End" "While" NEWLINE,
    do_statement: "Do" [loop_condition] NEWLINE _line* "Loop" [loop_condition] NEWLINE,
    loop_condition: ("While" | "Until") _expression,

    select_statement:
        "Select" ["Case"] _expression NEWLINE case_clause* "End" "Select" NEWLINE,
    case_clause: "Case" (case_values | "Else") NEWLINE _line*,
    case_values: _case_value ("," _case_value)*,
    _case_value: "Is" comparison_operator _expression | _expression,
    comparison_operator: "=" | "<>" | "<" | ">" | "<=" | ">=",

    try_statement: "Try" NEWLINE _line* catch_clause* [finally_clause] "End" "Try" NEWLINE,
    catch_clause: "Catch" [_name ["As" _type]] ["When" _expression] NEWLINE _line*,
    finally_clause: "Finally" NEWLINE _line*,

    with_statement: "With" _expression NEWLINE _line* "End" "With" NEWLINE,
    using_statement: "Using" (_name ["As" _type] "=")? _expression NEWLINE _line* "End" "Using" NEWLINE,

    return_statement: "Return" [_expression] NEWLINE,
    throw_statement: "Throw" [_expression] NEWLINE,
    exit_statement: "Exit" ("Sub" | "Function" | "For" | "Do" | "While" | "Try" | "Select") NEWLINE,
    continue_statement: "Continue" ("For" | "Do" | "While") NEWLINE,

    assignment_statement: _name "=" _expression NEWLINE,
    call_statement: _expression NEWLINE,

    argument_list: "(" [_argument %? ","] ")",
    _argument: _expression,

    _expression: precedence _primary {
        right "=" => assignment,
        left "OrElse" | "Or" | "Xor" => logical_or,
        left "AndAlso" | "And" => logical_and,
        left "<>" | "<" | ">" | "<=" | ">=" | "Is" | "Like" => comparison,
        left "&" => concatenation,
        left "+" | "-" => additive,
        left "Mod" => modulo,
        left "*" | "/" => multiplicative,
        left "\\" => integer_division,
        right "^" => power,
        prefix "Not" | "-" | "+" => unary,
        postfix argument_list => call,
        postfix "." _name => member_access,
    },

    _primary:
        _name
        | NUMBER
        | STRING
        | DATE
        | "Nothing"
        | "True"
        | "False"
        | parenthesized_expression
        | object_creation,

    _name: IDENT | "Me",
    parenthesized_expression: "(" _expression ")",
} }

pub fn lex_vb(c: &mut Cursor) -> Kind {
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\x0c'));
            WHITESPACE
        }
        b'\n' => { c.bump(); NEWLINE }
        b'\r' => {
            c.bump();
            c.eat(b'\n');
            NEWLINE
        }
        b'_' if matches!(c.peek_at(1), b'\n' | b'\r') => {
            c.bump();
            c.eat(b'\r');
            c.eat(b'\n');
            WHITESPACE
        }
        b'\'' => {
            lex::line_comment(c);
            COMMENT
        }
        b'"' => {
            c.bump();
            if c.quoted(b'"', true) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'#' => {
            c.bump();
            c.eat_while(|byte| byte != b'#' && byte != b'\n' && byte != b'\r');
            c.eat(b'#');
            DATE
        }
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'.' if c.peek_at(1).is_ascii_digit() => { c.bump(); lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'[' => {
            if byte == b'[' {
                // `[If]` escapes a keyword as a name.
                c.bump();
                c.eat_while(|byte| byte != b']' && byte != b'\n');
                c.eat(b']');
                return IDENT;
            }
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
            let text = &start[..length];
            if text.eq_ignore_ascii_case(b"REM") {
                lex::line_comment(c);
                return COMMENT;
            }
            keyword(&lex::upper(text))
        }
        b'(' => { c.bump(); LPAREN }
        b')' => { c.bump(); RPAREN }
        b'[' => { c.bump(); LBRACKET }
        b']' => { c.bump(); RBRACKET }
        b',' => { c.bump(); COMMA }
        b'.' => { c.bump(); DOT }
        b':' => { c.bump(); COLON }
        b'=' => { c.bump(); ASSIGN }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'>' => { c.bump_n(2); NE }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' => { c.bump(); GT }
        b'+' => { c.bump(); PLUS }
        b'-' => { c.bump(); MINUS }
        b'*' => { c.bump(); STAR }
        b'/' => { c.bump(); SLASH }
        b'\\' => { c.bump(); BACKSLASH }
        b'^' => { c.bump(); CARET }
        b'&' => { c.bump(); AMP }
        b'?' => { c.bump(); QUESTION }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"IMPORTS" => KW_IMPORTS,
        b"NAMESPACE" => KW_NAMESPACE,
        b"MODULE" => KW_MODULE,
        b"CLASS" => KW_CLASS,
        b"STRUCTURE" => KW_STRUCTURE,
        b"INTERFACE" => KW_INTERFACE,
        b"ENUM" => KW_ENUM,
        b"SUB" => KW_SUB,
        b"FUNCTION" => KW_FUNCTION,
        b"PROPERTY" => KW_PROPERTY,
        b"GET" => KW_GET,
        b"SET" => KW_SET,
        b"DIM" => KW_DIM,
        b"CONST" => KW_CONST,
        b"AS" => KW_AS,
        b"NEW" => KW_NEW,
        b"RETURN" => KW_RETURN,
        b"IF" => KW_IF,
        b"THEN" => KW_THEN,
        b"ELSEIF" => KW_ELSEIF,
        b"ELSE" => KW_ELSE,
        b"END" => KW_END,
        b"FOR" => KW_FOR,
        b"TO" => KW_TO,
        b"STEP" => KW_STEP,
        b"NEXT" => KW_NEXT,
        b"EACH" => KW_EACH,
        b"IN" => KW_IN,
        b"WHILE" => KW_WHILE,
        b"DO" => KW_DO,
        b"LOOP" => KW_LOOP,
        b"UNTIL" => KW_UNTIL,
        b"SELECT" => KW_SELECT,
        b"CASE" => KW_CASE,
        b"IS" => KW_IS,
        b"TRY" => KW_TRY,
        b"CATCH" => KW_CATCH,
        b"FINALLY" => KW_FINALLY,
        b"THROW" => KW_THROW,
        b"EXIT" => KW_EXIT,
        b"CONTINUE" => KW_CONTINUE,
        b"WITH" => KW_WITH,
        b"USING" => KW_USING,
        b"BYVAL" => KW_BYVAL,
        b"BYREF" => KW_BYREF,
        b"OPTIONAL" => KW_OPTIONAL,
        b"PUBLIC" => KW_PUBLIC,
        b"PRIVATE" => KW_PRIVATE,
        b"PROTECTED" => KW_PROTECTED,
        b"FRIEND" => KW_FRIEND,
        b"SHARED" => KW_SHARED,
        b"STATIC" => KW_STATIC,
        b"READONLY" => KW_READONLY,
        b"OVERRIDES" => KW_OVERRIDES,
        b"MUSTINHERIT" => KW_MUSTINHERIT,
        b"NOTHING" => KW_NOTHING,
        b"TRUE" => KW_TRUE,
        b"FALSE" => KW_FALSE,
        b"AND" => KW_AND,
        b"OR" => KW_OR,
        b"NOT" => KW_NOT,
        b"XOR" => KW_XOR,
        b"MOD" => KW_MOD,
        b"LIKE" => KW_LIKE,
        b"ANDALSO" => KW_ANDALSO,
        b"ORELSE" => KW_ORELSE,
        b"OF" => KW_OF,
        b"HANDLES" => KW_HANDLES,
        b"WHEN" => KW_WHEN,
        b"ME" => KW_ME,
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_f::harness;

    const VALID: &[(&str, &[&str])] = &[
        ("Module1.vb", &["import_statement", "import_statement", "unmodified_declaration"]),
        ("Report.vb", &["import_statement", "declaration"]),
        ("Service.vb", &["namespace_block"]),
    ];

    #[test]
    fn valid_fixtures() {
        harness::check_valid("visual_basic", &VISUAL_BASIC, VALID);
    }

    #[test]
    fn malformed_keeps_the_root() {
        harness::check_malformed(
            "malformed",
            "visual_basic_broken.vb",
            &VISUAL_BASIC,
            &["import_statement", "unmodified_declaration"],
        );
    }

    #[test]
    #[ignore]
    fn dump_top_level_kinds() {
        harness::dump("visual_basic", &VISUAL_BASIC);
        harness::dump("malformed", &VISUAL_BASIC);
    }

    #[test]
    fn lexes_case_insensitive_keywords_and_a_comment() {
        use crate::deep::lexer::{lex, Tokens};
        let src = b"dIm x As InTeGeR ' note\n";
        let mut tokens = Tokens::default();
        lex(src, &VISUAL_BASIC, &mut tokens);
        let kinds: Vec<&str> = (0..tokens.len())
            .map(|i| VISUAL_BASIC.name(tokens.kind[i]))
            .filter(|kind| *kind != "whitespace")
            .collect();
        assert_eq!(kinds, ["Dim", "identifier", "As", "identifier", "comment", "newline"]);
    }
}
