//! Go through `grammar!`, with the automatic semicolon insertion Go's
//! specification defines done in the step lexer.
//!
//! Node names follow Tree-sitter's Go where that costs nothing; the tree is
//! not the same tree.
//!
//! **Modelled.** Package and import declarations (single and parenthesised
//! lists, aliases, dot imports); `const`/`var`/`type` declarations in groups,
//! with grouped names, optional types and initialisers; generic type
//! parameters and constraints (`~T | U`); function and method declarations
//! (pointer receivers, variadic parameters, multiple results); every type
//! form (pointers, arrays, slices, maps, channels in both directions, function
//! types, structs with tags, interfaces with embedded types and methods);
//! blocks, `if` with an init statement, three-clause and `range` `for`,
//! `switch` (expression and type switches), `select`, `go`, `defer`, `goto`,
//! labels, `fallthrough`, `return`, `break`/`continue` with labels; channel
//! sends and receives, `++`/`--`, short declarations and augmented
//! assignments; the whole binary operator precedence table, unary operators,
//! calls, indexes and slices, selectors, type assertions, function literals,
//! composite literals (keyed and positional, nested) and conversions.
//!
//! **Not modelled.** A bare `T{...}` composite literal is not accepted where
//! a control-flow header is being parsed (Go's own grammar has the same
//! restriction: `if x {` must not read `x{...}`), and the grammar has no
//! general "type names from this package" knowledge. Struct field tags are
//! kept as string tokens. Comments are trivia and are not attached to nodes.
//! `//go:build` lines are ordinary comments. Semantic things — package
//! resolution, iota evaluation, whether a `range` variable is a copy — are
//! out of scope by construction.
//!
//! **DSL gaps.** (1) A lexer cannot tell "this `{` follows a name with no
//! space" because the step function sees bytes only from the cursor forward;
//! the header restriction above is the grammar-level workaround. (2) Token
//! text cannot be changed after lexing, so an inserted semicolon is a `;`
//! token whose span is whitespace, not a zero-width token. (3) There is no
//! way to mark a rule's alternatives with different node names, so method and
//! function declarations are separate rules that share their body shape.
use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { GO {
    name = "go";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        INT = "int_literal",
        FLOAT = "float_literal",
        IMAGINARY = "imaginary_literal",
        RUNE = "rune_literal",
        STRING = "interpreted_string_literal",
        RAW_STRING = "raw_string_literal",
        /// A string, rune or raw string with no end.
        BAD_LITERAL = "bad_literal" bad,

        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CHAN = "chan",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DEFAULT = "default",
        KW_DEFER = "defer",
        KW_ELSE = "else",
        KW_FALLTHROUGH = "fallthrough",
        KW_FOR = "for",
        KW_FUNC = "func",
        KW_GO = "go",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_IMPORT = "import",
        KW_INTERFACE = "interface",
        KW_MAP = "map",
        KW_PACKAGE = "package",
        KW_RANGE = "range",
        KW_RETURN = "return",
        KW_SELECT = "select",
        KW_STRUCT = "struct",
        KW_SWITCH = "switch",
        KW_TYPE = "type",
        KW_VAR = "var",

        /// Written, or a line break where Go inserts one.
        SEMI = ";",
        COMMA = ",",
        DOT = ".",
        ELLIPSIS = "...",
        COLON = ":",
        DEFINE = ":=",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GT = ">",
        GE = ">=",
        BANG = "!",
        TILDE = "~",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        AMP = "&",
        PIPE = "|",
        CARET = "^",
        AND_NOT = "&^",
        SHL = "<<",
        SHR = ">>",
        ANDAND = "&&",
        OROR = "||",
        ARROW = "<-",
        INCREMENT = "++",
        DECREMENT = "--",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        AND_NOT_EQ = "&^=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
    }

    source_file: (_top_level %? ";"),
    _top_level:
        package_clause
        | import_declaration
        | &("func" _name) function_declaration
        | method_declaration
        | _declaration,

    package_clause: "package" _name,
    import_declaration: "import" (import_spec | import_spec_list),
    import_spec_list: "(" (import_spec %? ";") ")",
    import_spec: [_name | "."] _string,
    _string: STRING | RAW_STRING,

    // ---- declarations ----

    _declaration: const_declaration | var_declaration | type_declaration,
    const_declaration: "const" (const_spec | const_spec_list),
    const_spec_list: "(" (const_spec %? ";") ")",
    const_spec: _name ("," _name)* [_type] ["=" _expression_list],
    var_declaration: "var" (var_spec | var_spec_list),
    var_spec_list: "(" (var_spec %? ";") ")",
    var_spec: _name ("," _name)* (_type ["=" _expression_list] | "=" _expression_list),
    type_declaration: "type" (type_spec | type_spec_list),
    type_spec_list: "(" (type_spec %? ";") ")",
    type_spec: _name _type_spec_rest,
    _type_spec_rest: &type_parameter_list type_parameter_list ["="] _type | ["="] _type,

    function_declaration: "func" _name [type_parameter_list] parameter_list [_result] [block],
    method_declaration: "func" parameter_list _name parameter_list [_result] [block],
    type_parameter_list: "[" (type_parameter_declaration %? ",") "]",
    type_parameter_declaration: _name ("," _name)* _constraint,
    _constraint: _constraint_term ("|" _constraint_term)*,
    _constraint_term: "~" _type | _type,
    parameter_list: "(" (_parameter %? ",") ")",
    _parameter:
        &(_name ["..."] _type) parameter_declaration
        | variadic_parameter_declaration
        | _type,
    parameter_declaration: _name ["..."] _type,
    variadic_parameter_declaration: "..." _type,
    _result: parameter_list | _type_np,

    // ---- types ----

    _type: _type_np | parenthesized_type,
    _type_np:
        _type_path
        | pointer_type
        | array_type
        | map_type
        | channel_type
        | receive_channel_type
        | function_type
        | struct_type
        | interface_type,
    parenthesized_type: "(" _type ")",
    _type_path: precedence _name {
        postfix "." _name => qualified_type,
        postfix type_arguments => generic_type,
    },
    type_arguments: "[" (_type %? ",") "]",
    pointer_type: "*" _type,
    array_type: array_length _type,
    array_length: "[" [_expression | "..."] "]",
    map_type: "map" map_key _type,
    map_key: "[" _type "]",
    channel_type: "chan" ["<-"] _type,
    /// `<-chan T`. Not an expression's operand: there `<-` receives, and the
    /// operand is `chan T`.
    receive_channel_type: "<-" "chan" _type,
    function_type: "func" parameter_list [_result],
    struct_type: "struct" field_declaration_list,
    field_declaration_list: "{" (field_declaration %? ";") "}",
    field_declaration: _field_head [_string],
    _field_head: &(_name ("," _name)* _type) _name ("," _name)* _type | _type,
    interface_type: "interface" interface_body,
    interface_body: "{" (_interface_element %? ";") "}",
    _interface_element: &(_name parameter_list) method_elem | type_elem,
    method_elem: _name parameter_list [_result],
    type_elem: _constraint,

    // ---- statements ----

    block: "{" (_statement %? ";") "}",
    _statement:
        _declaration
        | &(_name ":") labeled_statement
        | _simple
        | go_statement
        | defer_statement
        | return_statement
        | break_statement
        | continue_statement
        | goto_statement
        | fallthrough_statement
        | block
        | if_statement
        | for_statement
        | switch_statement
        | select_statement,
    labeled_statement: _name ":" [_statement],
    go_statement: "go" _expression,
    defer_statement: "defer" _expression,
    return_statement: "return" [_expression_list],
    break_statement: "break" [_name],
    continue_statement: "continue" [_name],
    goto_statement: "goto" _name,
