//! Solidity: contracts, their members, and the statements a scanner walks.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Types are expressions.** `uint256`, `mapping(address => uint256)` and
//!   `address payable` are modelled as names with optional type arguments, so a
//!   state variable's type is a `type_expression` with the right tokens but not
//!   a resolved type.
//! - **`assembly { ... }`** is a block of Yul, not of Solidity; the grammar
//!   accepts it as a statement block, so Yul's own grammar is not modelled.
//! - **NatSpec comments** are comments; a doc comment is not a node.
//! - **Modifiers, `using ... for`, `event`, `error`, `struct`, `enum`** each have
//!   a member rule; their bodies are a parameter list or a block.
//! - **Overloads and inheritance** are names and lists, not resolved.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { SOLIDITY {
    name = "solidity";
    lex = lex_solidity;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        COMMA = ",",
        SEMI = ";",
        COLON = ":",
        DOT = ".",
        QUESTION = "?",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LE = "<=",
        GE = ">=",
        LT = "<",
        GT = ">",
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
        LSHIFT = "<<",
        RSHIFT = ">>",
        FAT_ARROW = "=>",
        AND = "&&",
        OR = "||",
        ARROW = "->",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        HEX_STRING = "hex_string",
        BAD_STRING = "bad_string" bad,

        KW_PRAGMA = "pragma",
        KW_IMPORT = "import",
        KW_CONTRACT = "contract",
        KW_INTERFACE = "interface",
        KW_LIBRARY = "library",
        KW_ABSTRACT = "abstract",
        KW_IS = "is",
        KW_AS = "as",
        KW_FROM = "from",
        KW_FUNCTION = "function",
        KW_CONSTRUCTOR = "constructor",
        KW_MODIFIER = "modifier",
        KW_EVENT = "event",
        KW_ERROR = "error",
        KW_STRUCT = "struct",
        KW_ENUM = "enum",
        KW_MAPPING = "mapping",
        KW_RETURNS = "returns",
        KW_RETURN = "return",
        KW_IF = "if",
        KW_ELSE = "else",
        KW_FOR = "for",
        KW_WHILE = "while",
        KW_DO = "do",
        KW_BREAK = "break",
        KW_CONTINUE = "continue",
        KW_EMIT = "emit",
        KW_NEW = "new",
        KW_DELETE = "delete",
        KW_USING = "using",
        KW_PUBLIC = "public",
        KW_PRIVATE = "private",
        KW_INTERNAL = "internal",
        KW_EXTERNAL = "external",
        KW_PURE = "pure",
        KW_VIEW = "view",
        KW_PAYABLE = "payable",
        KW_MEMORY = "memory",
        KW_STORAGE = "storage",
        KW_CALLDATA = "calldata",
        KW_CONSTANT = "constant",
        KW_IMMUTABLE = "immutable",
        KW_OVERRIDE = "override",
        KW_VIRTUAL = "virtual",
        KW_ANONYMOUS = "anonymous",
        KW_INDEXED = "indexed",
        KW_TRUE = "true",
        KW_FALSE = "false",
    }

    source_unit: _item*,

    _item: pragma_directive | import_directive | contract_definition | free_function,

    pragma_directive: "pragma" IDENT (_pragma_token)* ";",
    _pragma_token: IDENT | NUMBER | "." | "^" | ">" | "<" | "=" | "-" | "/" | "*" | STRING,

    import_directive: "import" _import_body ";",
    _import_body: STRING | ident_path | "*" "as" IDENT | import_list,
    import_list: braces_names "from" STRING,
    braces_names: "{" (_import_name %? ",") "}",
    _import_name: IDENT ["as" IDENT],
    ident_path: IDENT ("." IDENT)*,

    contract_definition: ["abstract"] ("contract" | "interface" | "library") IDENT [inheritance] contract_body,
    inheritance: "is" (ident_path % ","),
    contract_body: "{" _member* "}",

    _member:
        function_definition
        | constructor_definition
        | modifier_definition
        | state_variable
        | struct_definition
        | enum_definition
        | event_definition
        | error_definition
        | using_directive,

    function_definition: "function" IDENT parameter_list _function_attributes (block | ";"),
    free_function: "function" IDENT parameter_list _function_attributes (block | ";"),
    constructor_definition: "constructor" parameter_list _function_attributes block,
    modifier_definition: "modifier" IDENT [parameter_list] _function_attributes (block | ";"),
    _function_attributes: (_function_attribute)*,
    _function_attribute:
        "public" | "private" | "internal" | "external" | "pure" | "view" | "payable" | "virtual" | "override" | returns_clause | modifier_invocation,
    returns_clause: "returns" parameter_list,
    modifier_invocation: IDENT [call_arguments],

    parameter_list: "(" [(parameter %? ",")] ")",
    parameter: type_expression ["indexed"] [storage_location] [IDENT],
    storage_location: "memory" | "storage" | "calldata",

    type_expression: IDENT [bracket_size] [type_expression_suffix],
    state_type: mapping_type | type_expression,
    bracket_size: "[" [NUMBER] "]",
    type_expression_suffix: "payable" | empty_brackets,
    empty_brackets: "[" "]",
    mapping_type: "mapping" mapping_body,
    mapping_body: "(" type_expression "=>" type_expression ")",

    state_variable: state_type _state_attribute* IDENT ["=" _expression] ";",
    _state_attribute: "public" | "private" | "internal" | "constant" | "immutable" | "override",

    struct_definition: "struct" IDENT struct_body,
    struct_body: "{" (struct_field %? ";") "}",
    struct_field: type_expression IDENT,

    enum_definition: "enum" IDENT enum_body,
    enum_body: "{" (IDENT %? ",") "}",

    event_definition: "event" IDENT parameter_list ["anonymous"] ";",
    error_definition: "error" IDENT parameter_list ";",
    using_directive: "using" ident_path "for" (type_expression | "*") ";",

    block: "{" _statement* "}",

    _statement:
        block
        | if_statement
        | for_statement
        | while_statement
        | do_statement
        | return_statement
        | break_statement
        | continue_statement
        | emit_statement
        | variable_declaration
        | expression_statement,

    if_statement: "if" paren_expression _statement ["else" _statement],
    for_statement: "for" for_header _statement,
    for_header: "(" [variable_declaration | expression_statement] ";" [_expression] ";" [_expression] ")",
    while_statement: "while" paren_expression _statement,
    do_statement: "do" _statement "while" paren_expression [";"],
    return_statement: "return" [_expression] ";",
    break_statement: "break" ";",
    continue_statement: "continue" ";",
    emit_statement: "emit" _expression ";",
    variable_declaration: local_type [storage_location] IDENT ["=" _expression] ";",
    local_type: mapping_type | IDENT,

    expression_statement: _expression ";",
    paren_expression: "(" _expression ")",

    _expression: precedence _primary {
        right "=" | "+=" | "-=" | "*=" | "/=" => assignment,
        left "?" _expression ":" => ternary,
        left "||" => logical_expression,
        left "&&" => logical_expression,
        left "==" | "!=" | "<" | ">" | "<=" | ">=" => comparison,
        left "|" => bitwise,
        left "^" => bitwise,
        left "&" => bitwise,
        left "<<" | ">>" => shift,
        left "+" | "-" => arithmetic,
        left "*" | "/" | "%" => arithmetic,
        prefix "!" | "-" | "~" | "delete" | "new" => unary,
        postfix call_arguments => call,
        postfix index_arguments => index,
        postfix member_expression => member_access,
    },

    call_arguments: "(" [(argument %? ",")] ")",
    argument: _expression,
    index_arguments: "[" [_expression] "]",
    member_expression: "." IDENT,

    _primary:
        IDENT
        | NUMBER
        | STRING
        | HEX_STRING
        | "true"
        | "false"
        | paren_expression
        | array_literal
        | payable_call,

    array_literal: "[" [(argument %? ",")] "]",
    payable_call: "payable" call_arguments,
} }

pub fn lex_solidity(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => { lex::line_comment(c); COMMENT }
        b'/' if c.peek_at(1) == b'*' => { lex::block_comment(c, b"/*", b"*/", false); COMMENT }
        b'"' | b'\'' => {
            if c.peek_at(1) == byte {
                c.bump_n(3);
                STRING
            } else if lex::quoted(c, byte) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'h' if c.starts_with(b"hex\"") => {
            c.bump_n(4);
            c.eat_while(|byte| byte != b'"' && byte != b'\n');
            c.eat(b'"');
            HEX_STRING
        }
        b'0'..=b'9' => { lex::number(c); NUMBER }
        b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
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
        b':' => { c.bump(); COLON }
        b'.' => { c.bump(); DOT }
        b'?' => { c.bump(); QUESTION }
        b'=' if c.peek_at(1) == b'=' => { c.bump_n(2); EQ }
        b'=' if c.peek_at(1) == b'>' => { c.bump_n(2); FAT_ARROW }
        b'=' => { c.bump(); ASSIGN }
        b'!' if c.peek_at(1) == b'=' => { c.bump_n(2); NE }
        b'!' => { c.bump(); BANG }
        b'<' if c.peek_at(1) == b'=' => { c.bump_n(2); LE }
        b'<' if c.peek_at(1) == b'<' => { c.bump_n(2); LSHIFT }
        b'<' => { c.bump(); LT }
        b'>' if c.peek_at(1) == b'=' => { c.bump_n(2); GE }
        b'>' if c.peek_at(1) == b'>' => { c.bump_n(2); RSHIFT }
        b'>' => { c.bump(); GT }
        b'+' if c.peek_at(1) == b'=' => { c.bump_n(2); PLUS_EQ }
        b'+' => { c.bump(); PLUS }
        b'-' if c.peek_at(1) == b'>' => { c.bump_n(2); ARROW }
        b'-' if c.peek_at(1) == b'=' => { c.bump_n(2); MINUS_EQ }
        b'-' => { c.bump(); MINUS }
        b'*' if c.peek_at(1) == b'=' => { c.bump_n(2); STAR_EQ }
        b'*' => { c.bump(); STAR }
        b'/' if c.peek_at(1) == b'=' => { c.bump_n(2); SLASH_EQ }
        b'/' => { c.bump(); SLASH }
        b'%' => { c.bump(); PERCENT }
        b'&' if c.peek_at(1) == b'&' => { c.bump_n(2); AND }
        b'&' => { c.bump(); AMP }
        b'|' if c.peek_at(1) == b'|' => { c.bump_n(2); OR }
        b'|' => { c.bump(); PIPE }
        b'^' => { c.bump(); CARET }
        b'~' => { c.bump(); TILDE }
        _ => UNKNOWN,
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"pragma" => KW_PRAGMA,
        b"import" => KW_IMPORT,
        b"contract" => KW_CONTRACT,
        b"interface" => KW_INTERFACE,
        b"library" => KW_LIBRARY,
        b"abstract" => KW_ABSTRACT,
        b"is" => KW_IS,
        b"as" => KW_AS,
        b"from" => KW_FROM,
        b"function" => KW_FUNCTION,
        b"constructor" => KW_CONSTRUCTOR,
        b"modifier" => KW_MODIFIER,
        b"event" => KW_EVENT,
        b"error" => KW_ERROR,
        b"struct" => KW_STRUCT,
        b"enum" => KW_ENUM,
        b"mapping" => KW_MAPPING,
        b"returns" => KW_RETURNS,
        b"return" => KW_RETURN,
        b"if" => KW_IF,
        b"else" => KW_ELSE,
        b"for" => KW_FOR,
        b"while" => KW_WHILE,
        b"do" => KW_DO,
        b"break" => KW_BREAK,
        b"continue" => KW_CONTINUE,
        b"emit" => KW_EMIT,
        b"new" => KW_NEW,
        b"delete" => KW_DELETE,
        b"using" => KW_USING,
        b"public" => KW_PUBLIC,
        b"private" => KW_PRIVATE,
        b"internal" => KW_INTERNAL,
        b"external" => KW_EXTERNAL,
        b"pure" => KW_PURE,
        b"view" => KW_VIEW,
        b"payable" => KW_PAYABLE,
        b"memory" => KW_MEMORY,
        b"storage" => KW_STORAGE,
        b"calldata" => KW_CALLDATA,
        b"constant" => KW_CONSTANT,
        b"immutable" => KW_IMMUTABLE,
        b"override" => KW_OVERRIDE,
        b"virtual" => KW_VIRTUAL,
        b"anonymous" => KW_ANONYMOUS,
        b"indexed" => KW_INDEXED,
        b"true" => KW_TRUE,
        b"false" => KW_FALSE,
        _ => IDENT,
    }
}
