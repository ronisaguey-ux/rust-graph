//! Rust through `grammar!`: Slice 4. Node names follow Tree-sitter's Rust
//! where that costs nothing; the tree is not the same tree.
//!
//! Three choices make Rust LL(3):
//!
//! - `<` and `>` never join a following `<` or `>`, so `Vec<Vec<u8>>` closes
//!   two generic lists and `Vec<<T as X>::Y>` opens one with a qualified path.
//!   A `>` right before another `>` is its own kind, `GT_JOINED`, so the shift
//!   `>>` (`GT_JOINED ">"`) never looks like the comparison `>`; `LT_JOINED` is
//!   the same for `<<`. `<=` and `>=` are tokens, so `Vec<u8>= x` with no space
//!   is an error here.
//! - An item can start with `pub(crate)` and several qualifiers, more than
//!   three tokens ahead of its keyword, so items are chosen by predicates that
//!   try their head (`_fn_head`, ...) before the decision tree runs.
//! - Expressions come in three tables: `_expression` everywhere, `_expression_ns`
//!   where a `{` opens a block (`if`, `while`, `match`, `for`), so it has no
//!   struct literals, and `_expression_nb` for statements and match arms, whose
//!   first operand cannot be block-like: `if a {} (b)` is two statements, as
//!   Rust says. Block-like statements are `_block_like_chain`, which takes only
//!   `.`, `?` and then assignment after the block, never `(`, so the next
//!   statement or match arm may start with one.
//!
//! Macro bodies and attributes are token trees, not parsed further.

use super::lexer::Cursor;
use super::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { RUST {
    name = "rust";
    lex = lex;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "identifier",
        LIFETIME = "lifetime",
        INTEGER = "integer_literal",
        FLOAT = "float_literal",
        STR = "string_literal",
        RAW_STR = "raw_string_literal",
        CHAR = "char_literal",
        /// A string, raw string or character with no closing quote.
        BAD_LITERAL = "bad_literal" bad,

        KW_AS = "as",
        KW_ASYNC = "async",
        KW_AWAIT = "await",
        KW_BREAK = "break",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_CRATE = "crate",
        KW_DYN = "dyn",
        KW_ELSE = "else",
        KW_ENUM = "enum",
        KW_EXTERN = "extern",
        KW_FALSE = "false",
        KW_FN = "fn",
        KW_FOR = "for",
        KW_IF = "if",
        KW_IMPL = "impl",
        KW_IN = "in",
        KW_LET = "let",
        KW_LOOP = "loop",
        KW_MATCH = "match",
        KW_MOD = "mod",
        KW_MOVE = "move",
        KW_MUT = "mut",
        KW_PUB = "pub",
        KW_REF = "ref",
        KW_RETURN = "return",
        KW_SELF = "self",
        KW_SELF_TYPE = "Self",
        KW_STATIC = "static",
        KW_STRUCT = "struct",
        KW_SUPER = "super",
        KW_TRAIT = "trait",
        KW_TRUE = "true",
        KW_TYPE = "type",
        KW_UNSAFE = "unsafe",
        KW_USE = "use",
        KW_WHERE = "where",
        KW_WHILE = "while",
        /// Contextual keywords: names everywhere but where they start their item.
        KW_UNION = "union",
        KW_DEFAULT = "default",
        KW_MACRO_RULES = "macro_rules",
        /// Reserved since 2018; starts a `decl_macro` (macros 2.0, used in core and std).
        KW_MACRO = "macro",
        KW_RAW = "raw",
        KW_AUTO = "auto",
        KW_SAFE = "safe",
        UNDERSCORE = "_",

        COMMA = ",",
        SEMI = ";",
        COLON = ":",
        PATHSEP = "::",
        DOT = ".",
        DOTDOT = "..",
        DOTDOTDOT = "...",
        DOTDOTEQ = "..=",
        ARROW = "->",
        FATARROW = "=>",
        HASH = "#",
        DOLLAR = "$",
        QUESTION = "?",
        AT = "@",
        TILDE = "~",
        ASSIGN = "=",
        EQ = "==",
        NE = "!=",
        LT = "<",
        LE = "<=",
        GE = ">=",
        /// A `>` right before another `>`: the first half of `>>` or `>>=`, or
        /// a generic list closing right before another.
        GT_JOINED = ">joined",
        /// A `<` right before another `<`: the first half of `<<` or `<<=`, or
        /// a generic list opening with a qualified path, `Vec<<T as X>::Y>`.
        LT_JOINED = "<joined",
        GT = ">",
        BANG = "!",
        PLUS = "+",
        MINUS = "-",
        STAR = "*",
        SLASH = "/",
        PERCENT = "%",
        CARET = "^",
        AMP = "&",
        PIPE = "|",
        ANDAND = "&&",
        OROR = "||",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        PERCENT_EQ = "%=",
        CARET_EQ = "^=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
    }

    source_file: _statement*,

    // ---- statements and items ----

    _statement:
        &_item_start _declaration_statement
        | let_declaration
        | expression_statement
        | empty_statement,

    /// What can start an item and nothing else: tried as a predicate.
    _item_start: "#" | "macro_rules" "!" | [visibility_modifier] _item_keyword,
    _item_keyword:
        "fn" | "struct" | "enum" | "trait" | "impl" | "mod" | "use" | "type" | "extern"
        | "union" _name
        | "const" (_name | "_" | "fn" | "unsafe" | "async" | "extern")
        | "static" (_name | "mut")
        | "async" ("fn" | "unsafe" | "extern")
        | "unsafe" ("fn" | "impl" | "trait" | "extern" | "mod" | "auto" | "static")
        | "safe" ("fn" | "static")
        | "default" ("fn" | "type" | "const" | "impl" | "unsafe" | "async")
        | "auto" "trait"
        | "macro" _name,

    _declaration_statement:
        &_fn_head function_item
        | &([visibility_modifier] "struct") struct_item
        | &([visibility_modifier] "enum") enum_item
        | &([visibility_modifier] "use") use_declaration
        | &([visibility_modifier] ["default"] "type") type_item
        | &([visibility_modifier] ["default"] "const" (_name | "_")) const_item
        | &([visibility_modifier] ["unsafe" | "safe"] "static") static_item
        | &([visibility_modifier] ["unsafe"] "mod") mod_item
        | &([visibility_modifier] ["unsafe"] ["auto"] "trait") trait_item
        | &([visibility_modifier] "union") union_item
        | &([visibility_modifier] "extern" "crate") extern_crate_declaration
        | impl_item
        | foreign_mod_item
        | macro_definition
        | &([visibility_modifier] "macro") decl_macro
        | macro_invocation_item
        | attribute_item
        | inner_attribute_item
        | empty_statement,

    empty_statement: ";",

    _fn_head: [visibility_modifier] _fn_qualifiers "fn",
    _fn_qualifiers: ["default"] ["const"] ["async"] ["unsafe" | "safe"] [extern_modifier],
    extern_modifier: "extern" [STR | RAW_STR],

    attribute_item: "#" attribute,
    inner_attribute_item: "#" "!" attribute,
    attribute: "[" _tt* "]",

    visibility_modifier: "pub" [visibility_restriction],
    visibility_restriction: "(" ("crate" | "self" | "super" | "in" _simple_path) ")",

    function_item: [visibility_modifier] _fn_qualifiers "fn" _name [type_parameters] parameters ["->" _type] [where_clause] (block | ";"),
    parameters: "(" (_any_parameter %? ",") ")",
    _any_parameter: attribute_item* (self_parameter | parameter | variadic_parameter),
    self_parameter: [("&" | "&&") [LIFETIME]] ["mut"] "self" [":" _type],
    parameter: _pattern@bind ":" (_type | "..."),
    variadic_parameter: "...",

    struct_item: [visibility_modifier] "struct" _name [type_parameters] [where_clause]
        (field_declaration_list | ordered_field_declaration_list [where_clause] ";" | ";"),
    union_item: [visibility_modifier] "union" _name [type_parameters] [where_clause] field_declaration_list,
    field_declaration_list: "{" (_field %? ",") "}",
    _field: attribute_item* field_declaration,
    field_declaration: [visibility_modifier] _name ":" _type ["=" _expression],
    ordered_field_declaration_list: "(" (_tuple_field %? ",") ")",
    _tuple_field: attribute_item* [visibility_modifier] _type,

    enum_item: [visibility_modifier] "enum" _name [type_parameters] [where_clause] enum_variant_list,
    enum_variant_list: "{" (_variant %? ",") "}",
    _variant: attribute_item* enum_variant,
    enum_variant: [visibility_modifier] _name [field_declaration_list | ordered_field_declaration_list] ["=" _expression],

    trait_item: [visibility_modifier] ["unsafe"] ["auto"] "trait" _name [type_parameters] [":" [_bounds]] [where_clause]
        (declaration_list | "=" _bounds [where_clause] ";"),
    impl_item: ["default"] ["unsafe"] "impl" [type_parameters] ["!"] _type ["for" _type] [where_clause] (declaration_list | ";"),
    declaration_list: "{" _declaration_statement* "}",

    mod_item: [visibility_modifier] ["unsafe"] "mod" _name (";" | declaration_list),
    foreign_mod_item: [visibility_modifier] ["unsafe"] "extern" [STR | RAW_STR] (declaration_list | ";"),
    extern_crate_declaration: [visibility_modifier] "extern" "crate" (_name | "self") ["as" (_name | "_")] ";",

    const_item: [visibility_modifier] ["default"] "const" (_name | "_") [":" _type] ["=" _expression] ";",
    static_item: [visibility_modifier] ["unsafe" | "safe"] "static" ["mut"] _name ":" _type ["=" _expression] ";",
    type_item: [visibility_modifier] ["default"] "type" _name [type_parameters] [":" _bounds] [where_clause] ["=" _type] [where_clause] ";",

    use_declaration: [visibility_modifier] "use" _use_clause ";",
    _use_clause: precedence _use_start {
        postfix "as" (_name | "_") => use_as_clause,
        postfix "::" use_list => scoped_use_list,
        postfix "::" "*" => use_wildcard,
        postfix "::" _path_name => scoped_identifier,
    },
    _use_start: _path_name | use_list | "*" | "::" _path_name,
    use_list: "{" (_use_clause %? ",") "}",

    macro_definition: "macro_rules" "!" _name (token_tree_brace | (token_tree | token_tree_bracket) ";"),
    /// `macro m($x:expr) { $x }` or `macro m { (...) => { ... } }`: macros 2.0.
    decl_macro: [visibility_modifier] "macro" _name (token_tree token_tree_brace | token_tree_brace),
    /// A macro call where items go. In expressions, types and patterns it is a row: `macro_invocation`.
    macro_invocation_item: _macro_path "!" (token_tree_brace | (token_tree | token_tree_bracket) [";"]),
    _simple_path: ["::"] _path_name ("::" _path_name)*,
    /// A path that cannot be `macro_rules`, which starts a definition there.
    _macro_path: (IDENT | "union" | "default" | "raw" | "auto" | "safe" | "self" | "super" | "crate" | "Self" | "::" _path_name) ("::" _path_name)*,

    token_tree: "(" _tt* ")",
    token_tree_bracket: "[" _tt* "]",
    token_tree_brace: "{" _tt* "}",
    _token_tree_any: token_tree | token_tree_bracket | token_tree_brace,
    _tt: _token_tree_any | _tt_token,
    _tt_token:
        IDENT | LIFETIME | INTEGER | FLOAT | STR | RAW_STR | CHAR | BAD_LITERAL
        | "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else" | "enum"
        | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match" | "mod"
        | "move" | "mut" | "pub" | "ref" | "return" | "self" | "Self" | "static" | "struct" | "super"
        | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while" | "union" | "default"
        | "macro_rules" | "macro" | "raw" | "auto" | "safe" | "_"
        | "," | ";" | ":" | "::" | "." | ".." | "..." | "..=" | "->" | "=>" | "#" | "$" | "?" | "@" | "~"
        | "=" | "==" | "!=" | "<" | "<=" | LT_JOINED | ">" | GT_JOINED | ">=" | "!" | "+" | "-" | "*" | "/" | "%" | "^" | "&" | "|" | "&&" | "||"
        | "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=",

    let_declaration: "let" _pattern [":" _type] ["=" _expression ["else" block]] ";",

    expression_statement: (_block_like_chain | _expression_nb) [";"],

    // ---- generics ----

    type_parameters: "<" (_generic_parameter %? ",") _gt,
    /// What closes a generic list: `>`, or the first `>` of `>>`.
    _gt: ">" | GT_JOINED,
    _generic_parameter: attribute_item* (lifetime_parameter | const_parameter | type_parameter),
    lifetime_parameter: LIFETIME [":" (LIFETIME % "+")],
    type_parameter: _name [":" [_bounds]] ["=" _type],
    const_parameter: "const" _name ":" _type ["=" _const_argument],
    where_clause: "where" (where_predicate %? ","),
    where_predicate: (LIFETIME | _type) ":" [_bounds],
    _bounds: _bound ("+" _bound)* ["+"],
    _bound: LIFETIME | use_bounds | trait_bound,
    trait_bound: ["?" | "~" "const" | "const" | "async"] _type,
    use_bounds: "use" type_arguments,
    for_lifetimes: "for" "<" (_generic_parameter %? ",") _gt,

    type_arguments: ("<" | LT_JOINED) (_type_argument %? ",") _gt,
    _type_argument: precedence _type_argument_start {
        postfix "=" _type => type_binding,
        postfix ":" _bounds => trait_bound_constraint,
    },
    _type_argument_start: _type | LIFETIME | _const_argument,
    _const_argument: block | _literal | negative_literal,

    // ---- types ----

    _type:
        _type_path
        | reference_type
        | pointer_type
        | tuple_type
        | array_type
        | function_type
        | higher_ranked_type
        | never_type
        | "_"
        | dynamic_type
        | abstract_type,
    reference_type: ("&" | "&&") [LIFETIME] ["mut"] _type,
    pointer_type: "*" ("const" | "mut") _type,
    tuple_type: "(" (_type %? ",") ")",
    array_type: "[" _type [";" _expression] "]",
    function_type: ["unsafe"] [extern_modifier] "fn" fn_type_parameters ["->" _type],
    fn_type_parameters: "(" (_fn_type_parameter %? ",") ")",
    _fn_type_parameter: attribute_item* (_type_with_name | "..."),
    _type_with_name: precedence _type { postfix ":" _type => named_type, },
    higher_ranked_type: for_lifetimes _type,
    never_type: "!",
    dynamic_type: "dyn" _bounds,
    abstract_type: "impl" _bounds,

    _type_path: precedence _path_start {
        postfix "::" type_arguments => generic_type,
        postfix "::" _path_name => scoped_type_identifier,
        postfix type_arguments => generic_type,
        postfix fn_type_parameters ["->" _type] => function_trait,
        postfix "!" _token_tree_any => macro_invocation,
    },

    _path_start: _path_name | "::" _path_name | qualified_type,
    _path_name: _name | "self" | "super" | "crate" | "Self",
    qualified_type: ("<" | LT_JOINED) _type ["as" _type] _gt,

    // ---- patterns ----

    _pattern: precedence _pattern_atom {
        or: left "|" => or_pattern,
        bind: right "@" => captured_pattern,
        postfix ("..=" | "...") _range_end => range_pattern,
        postfix ".." [_range_end] => range_pattern,
        postfix tuple_pattern => tuple_struct_pattern,
        postfix field_pattern_list => struct_pattern,
        postfix "!" _token_tree_any => macro_invocation,
    },
    _pattern_atom:
        _pattern_path
        | _literal
        | negative_literal
        | "_"
        | rest_pattern
        | tuple_pattern
        | slice_pattern
        | reference_pattern
        | mut_pattern
        | ref_pattern
        | const_block,
    _pattern_path: precedence _pattern_path_start {
        postfix "::" type_arguments => generic_type,
        postfix "::" _path_name => scoped_identifier,
    },
    /// No `self`: a parameter `self: Box<Self>` is a self parameter.
    _pattern_path_start: _name | "super" | "crate" | "Self" | "::" _path_name | qualified_type,
    _range_end: _pattern_path | _literal | negative_literal,
    rest_pattern: (".." | "..=") [_range_end],
    tuple_pattern: "(" (_pattern %? ",") ")",
    slice_pattern: "[" (_pattern %? ",") "]",
    reference_pattern: ("&" | "&&") ["mut"] _pattern@bind,
    mut_pattern: "mut" _pattern@bind,
    ref_pattern: "ref" ["mut"] _name,
    field_pattern_list: "{" (_field_pattern_item %? ",") "}",
    _field_pattern_item: attribute_item* (field_pattern | ".."),
    field_pattern: ["ref"] ["mut"] _name [":" _pattern] | INTEGER ":" _pattern,
    negative_literal: "-" (INTEGER | FLOAT),

    // ---- expressions ----

    _literal: INTEGER | FLOAT | STR | RAW_STR | CHAR | BAD_LITERAL | "true" | "false",

    _expression: precedence _primary {
        assign: right "=" => assignment_expression,
        right "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=" | LT_JOINED "<=" | GT_JOINED ">=" => compound_assignment_expr,
        postfix (".." | "..=" | "...") [_expression@or] => range_expression,
        or: left "||" => binary_expression,
        and: left "&&" => binary_expression,
        cmp: left "==" | "!=" | "<=" | ">=" | "<" | ">" => binary_expression,
        bor: left "|" => binary_expression,
        xor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        shift: left LT_JOINED "<" | GT_JOINED ">" => binary_expression,
        add: left "+" | "-" => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        cast: postfix "as" _type => type_cast_expression,
        unary: prefix "-" | "*" | "!" => unary_expression,
        prefix _borrow => reference_expression,
        postfix "?" => try_expression,
        postfix arguments => call_expression,
        postfix index => index_expression,
        postfix "." "await" => await_expression,
        postfix "." (_name | INTEGER) ["::" type_arguments] => field_expression,
        postfix "!" _token_tree_any => macro_invocation,
        postfix field_initializer_list => struct_expression,
    },

    /// `_expression` with no struct literals: conditions and scrutinees.
    _expression_ns: precedence _primary_ns {
        right "=" => assignment_expression,
        right "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=" | LT_JOINED "<=" | GT_JOINED ">=" => compound_assignment_expr,
        postfix (".." | "..=" | "...") [_expression_ns@or_ns] => range_expression,
        or_ns: left "||" => binary_expression,
        and_ns: left "&&" => binary_expression,
        cmp_ns: left "==" | "!=" | "<=" | ">=" | "<" | ">" => binary_expression,
        bor_ns: left "|" => binary_expression,
        xor_ns: left "^" => binary_expression,
        band_ns: left "&" => binary_expression,
        shift_ns: left LT_JOINED "<" | GT_JOINED ">" => binary_expression,
        add_ns: left "+" | "-" => binary_expression,
        mul_ns: left "*" | "/" | "%" => binary_expression,
        cast_ns: postfix "as" _type => type_cast_expression,
        prefix "-" | "*" | "!" => unary_expression,
        prefix _borrow => reference_expression,
        postfix "?" => try_expression,
        postfix arguments => call_expression,
        postfix index => index_expression,
        postfix "." "await" => await_expression,
        postfix "." (_name | INTEGER) ["::" type_arguments] => field_expression,
        postfix "!" _token_tree_any => macro_invocation,
    },

    /// A statement or match arm whose first operand is not block-like. Right
    /// operands are full expressions.
    _expression_nb: precedence _primary_nb {
        postfix "=" _expression@assign => assignment_expression,
        postfix ("+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=" | LT_JOINED "<=" | GT_JOINED ">=") _expression@assign => compound_assignment_expr,
        postfix (".." | "..=" | "...") [_expression@or] => range_expression,
        postfix "||" _expression@and => binary_expression,
        postfix "&&" _expression@cmp => binary_expression,
        postfix ("==" | "!=" | "<=" | ">=" | "<" | ">") _expression@bor => binary_expression,
        postfix "|" _expression@xor => binary_expression,
        postfix "^" _expression@band => binary_expression,
        postfix "&" _expression@shift => binary_expression,
        postfix (LT_JOINED "<" | GT_JOINED ">") _expression@add => binary_expression,
        postfix ("+" | "-") _expression@mul => binary_expression,
        postfix ("*" | "/" | "%") _expression@cast => binary_expression,
        postfix "as" _type => type_cast_expression,
        prefix "-" | "*" | "!" => unary_expression,
        prefix _borrow => reference_expression,
        postfix "?" => try_expression,
        postfix arguments => call_expression,
        postfix index => index_expression,
        postfix "." "await" => await_expression,
        postfix "." (_name | INTEGER) ["::" type_arguments] => field_expression,
        postfix "!" _token_tree_any => macro_invocation,
        postfix field_initializer_list => struct_expression,
    },

    /// A block-like expression in statement position, and the little that may
    /// follow it, as in rustc: `.` and `?`, then assignment. Never a call or an
    /// index, so `if a {} (b, c) = d;` is two statements. A method call here is
    /// one `field_expression` with its `arguments` inside, unlike elsewhere.
    _block_like_chain: precedence _block_like {
        postfix "=" _expression@assign => assignment_expression,
        postfix ("+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=" | LT_JOINED "<=" | GT_JOINED ">=") _expression@assign => compound_assignment_expr,
        postfix "?" => try_expression,
        postfix "." "await" => await_expression,
        postfix "." (_name | INTEGER) ["::" type_arguments] [arguments] => field_expression,
    },

    _primary: _primary_nb | _block_like,
    _primary_ns: _primary_nb | _block_like_ns | let_condition,
    _primary_nb:
        _literal
        | "_"
        | _expression_path
        | tuple_expression
        | array_expression
        | closure_expression
        | return_expression
        | break_expression
        | continue_expression
        | const_block
        | range_prefix,
    /// `&`, `&mut`, `&raw const`; `&raw` alone borrows a variable named `raw`.
    _borrow: &(("&" | "&&") "raw" ("const" | "mut")) ("&" | "&&") "raw" ("const" | "mut") | ("&" | "&&") ["mut"],
    _block_like: block | _block_like_ns,
    _block_like_ns:
        if_expression
        | match_expression
        | loop_expression
        | while_expression
        | for_expression
        | unsafe_block
        | async_block
        | labeled_block,

    _expression_path: precedence _path_start {
        postfix "::" type_arguments => generic_function,
        postfix "::" _path_name => scoped_identifier,
    },

    tuple_expression: "(" (_argument %? ",") ")",
    array_expression: "[" [_argument [";" _expression | "," (_argument %? ",")]] "]",
    arguments: "(" (_argument %? ",") ")",
    _argument: attribute_item* _expression,
    index: "[" _expression "]",
    field_initializer_list: "{" (_field_initializer_item %? ",") "}",
    _field_initializer_item: attribute_item* (field_initializer | shorthand_field_initializer | base_field_initializer),
    field_initializer: (_name | INTEGER) ":" _expression,
    shorthand_field_initializer: _name,
    base_field_initializer: ".." [_expression],

    closure_expression: ["async"] ["move"] closure_parameters ["->" _type] _expression,
    closure_parameters: "||" | "|" (_closure_parameter %? ",") "|",
    _closure_parameter: attribute_item* _pattern@bind [":" _type],

    return_expression: "return" [_expression],
    break_expression: "break" [LIFETIME] [_expression],
    continue_expression: "continue" [LIFETIME],
    range_prefix: (".." | "..=") [_expression@or],

    block: "{" _statement* "}",
    labeled_block: LIFETIME ":" (block | loop_expression | while_expression | for_expression),
    unsafe_block: "unsafe" block,
    async_block: "async" ["move"] block,
    const_block: "const" block,
    if_expression: "if" _condition block [else_clause],
    else_clause: "else" (block | if_expression),
    /// A condition or scrutinee may start with a block, as in `while { step() } {}`
    /// and `match { it }.next() { ... }`. Operators may follow the block, with
    /// their right operands in `_expression_ns`, but not a call or an index.
    _condition: _block_condition | _expression_ns,
    _block_condition: precedence block {
        postfix "||" _expression_ns@and_ns => binary_expression,
        postfix "&&" _expression_ns@cmp_ns => binary_expression,
        postfix ("==" | "!=" | "<=" | ">=" | "<" | ">") _expression_ns@bor_ns => binary_expression,
        postfix "|" _expression_ns@xor_ns => binary_expression,
        postfix "^" _expression_ns@band_ns => binary_expression,
        postfix "&" _expression_ns@shift_ns => binary_expression,
        postfix (LT_JOINED "<" | GT_JOINED ">") _expression_ns@add_ns => binary_expression,
        postfix ("+" | "-") _expression_ns@mul_ns => binary_expression,
        postfix ("*" | "/" | "%") _expression_ns@cast_ns => binary_expression,
        postfix "as" _type => type_cast_expression,
        postfix "?" => try_expression,
        postfix "." "await" => await_expression,
        postfix "." (_name | INTEGER) ["::" type_arguments] [arguments] => field_expression,
    },
    /// `let P = e` in a condition: an operand, so `a && let P = e && b` chains.
    let_condition: "let" _pattern "=" (block | _expression_ns@cmp_ns),
    while_expression: "while" _condition block,
    loop_expression: "loop" block,
    for_expression: "for" _pattern "in" _expression_ns block,
    match_expression: "match" _condition match_block,
    match_block: "{" match_arm* "}",
    match_arm: attribute_item* ["|"] _pattern ["if" _condition] "=>" (_block_like_chain | _expression_nb) [","],

    _name: IDENT | "union" | "default" | "raw" | "auto" | "safe" | "macro_rules",
} }

/// One token at the cursor.
pub fn lex(c: &mut Cursor) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c'));
            WHITESPACE
        }
        b'/' if c.peek_at(1) == b'/' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            block_comment(c);
            COMMENT
        }
        0xef if c.pos() == 0 && c.starts_with(b"\xef\xbb\xbf") => {
            c.bump_n(3);
            WHITESPACE
        }
        // A shebang line, which `#![` is not.
        b'#' if c.pos() == 0 && c.peek_at(1) == b'!' && c.peek_at(2) != b'[' => {
            c.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'"' => {
            c.bump();
            if c.quoted(b'"', false) { STR } else { BAD_LITERAL }
        }
        b'\'' => quote(c),
        b'a'..=b'z' | b'A'..=b'Z' | b'_' | 0x80..=0xff => word(c),
        b'0'..=b'9' => number(c),
        _ => operator(c),
    }
}

/// `/* ... */`, nested. An unclosed one runs to the end of the file.
fn block_comment(c: &mut Cursor) {
    c.bump_n(2);
    let mut depth = 1u32;
    while !c.at_end() {
        if c.starts_with(b"*/") {
            c.bump_n(2);
            depth -= 1;
            if depth == 0 {
                return;
            }
        } else if c.starts_with(b"/*") {
            c.bump_n(2);
            depth += 1;
        } else {
            c.bump();
        }
    }
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// A `'`: a character literal or a lifetime (or loop label).
fn quote(c: &mut Cursor) -> Kind {
    let next = c.peek_at(1);
    if next == b'\\' {
        c.bump();
        return if c.quoted(b'\'', true) { CHAR } else { BAD_LITERAL };
    }
    // One character, then a closing quote: a character literal.
    let width = match next {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    };
    if next != b'\n' && next != 0 && c.peek_at(1 + width) == b'\'' {
        c.bump_n(2 + width);
        return CHAR;
    }
    if next.is_ascii_alphabetic() || next == b'_' || next >= 0x80 {
        c.bump();
        // `'r#ident`, a raw lifetime.
        if c.starts_with(b"r#") {
            c.bump_n(2);
        }
        c.eat_while(is_word_byte);
        return LIFETIME;
    }
    c.bump();
    BAD_LITERAL
}

fn word(c: &mut Cursor) -> Kind {
    // String and character prefixes: b"", br"", r"", r#""#, c"", cr"", b''.
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    match (a, b, d) {
        (b'b', b'\'', _) => {
            c.bump();
            return quote(c);
        }
        (b'b' | b'c', b'"', _) => {
            c.bump_n(2);
            return if c.quoted(b'"', false) { STR } else { BAD_LITERAL };
        }
        (b'r', b'"' | b'#', _) if b == b'"' || d == b'"' || d == b'#' => return raw_string(c, 1),
        (b'b' | b'c', b'r', b'"' | b'#') => return raw_string(c, 2),
        // A raw identifier.
        (b'r', b'#', _) if is_word_byte(d) => {
            c.bump_n(2);
            c.eat_while(is_word_byte);
            return IDENT;
        }
        _ => {}
    }
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    keyword(&start[..length])
}

/// A raw string from its prefix of `prefix` letters: `r#"..."#`.
fn raw_string(c: &mut Cursor, prefix: usize) -> Kind {
    c.bump_n(prefix);
    let hashes = c.eat_while(|byte| byte == b'#');
    if !c.eat(b'"') {
        // `r#` then not a quote: not a string after all.
        return BAD_LITERAL;
    }
    loop {
        if c.at_end() {
            return BAD_LITERAL;
        }
        if c.peek() == b'"' {
            c.bump();
            let mut closing = 0;
            while closing < hashes && c.peek() == b'#' {
                c.bump();
                closing += 1;
            }
            if closing == hashes {
                return RAW_STR;
            }
        } else {
            c.bump();
        }
    }
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"as" => KW_AS,
        b"async" => KW_ASYNC,
        b"await" => KW_AWAIT,
        b"break" => KW_BREAK,
        b"const" => KW_CONST,
        b"continue" => KW_CONTINUE,
        b"crate" => KW_CRATE,
        b"dyn" => KW_DYN,
        b"else" => KW_ELSE,
        b"enum" => KW_ENUM,
        b"extern" => KW_EXTERN,
        b"false" => KW_FALSE,
        b"fn" => KW_FN,
        b"for" => KW_FOR,
        b"if" => KW_IF,
        b"impl" => KW_IMPL,
        b"in" => KW_IN,
        b"let" => KW_LET,
        b"loop" => KW_LOOP,
        b"match" => KW_MATCH,
        b"mod" => KW_MOD,
        b"move" => KW_MOVE,
        b"mut" => KW_MUT,
        b"pub" => KW_PUB,
        b"ref" => KW_REF,
        b"return" => KW_RETURN,
        b"self" => KW_SELF,
        b"Self" => KW_SELF_TYPE,
        b"static" => KW_STATIC,
        b"struct" => KW_STRUCT,
        b"super" => KW_SUPER,
        b"trait" => KW_TRAIT,
        b"true" => KW_TRUE,
        b"type" => KW_TYPE,
        b"unsafe" => KW_UNSAFE,
        b"use" => KW_USE,
        b"where" => KW_WHERE,
        b"while" => KW_WHILE,
        b"union" => KW_UNION,
        b"default" => KW_DEFAULT,
        b"macro_rules" => KW_MACRO_RULES,
        b"macro" => KW_MACRO,
        b"raw" => KW_RAW,
        b"auto" => KW_AUTO,
        b"safe" => KW_SAFE,
        b"_" => UNDERSCORE,
        _ => IDENT,
    }
}

/// An integer or float literal, suffix included. After a lone `.` (a tuple
/// index, `x.0.1`) only the integer is taken.
fn number(c: &mut Cursor) -> Kind {
    let tuple_index = c.behind(1) == b'.' && c.behind(2) != b'.';
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'o' | b'b') {
        c.bump_n(2);
        c.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        return INTEGER;
    }
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if tuple_index {
        return INTEGER;
    }
    let mut float = false;
    if c.peek() == b'.' && c.peek_at(1) != b'.' && !(c.peek_at(1).is_ascii_alphabetic() || c.peek_at(1) == b'_') {
        c.bump();
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
        float = true;
    }
    if matches!(c.peek(), b'e' | b'E') {
        let sign = usize::from(matches!(c.peek_at(1), b'+' | b'-'));
        if c.peek_at(1 + sign).is_ascii_digit() || c.peek_at(1 + sign) == b'_' {
            c.bump_n(1 + sign);
            c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
            float = true;
        }
    }
    // A suffix: `u8`, `f64`, `usize`.
    let suffix = c.rest();
    let length = c.eat_while(is_word_byte);
    if length > 0 && suffix[0] == b'f' {
        float = true;
    }
    if float { FLOAT } else { INTEGER }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'.', b'.', b'.') => (DOTDOTDOT, 3),
        (b'.', b'.', b'=') => (DOTDOTEQ, 3),
        (b'.', b'.', _) => (DOTDOT, 2),
        (b'<', b'<', _) => (LT_JOINED, 1),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'>', _) => (GT_JOINED, 1),
        (b'>', b'=', _) => (GE, 2),
        (b':', b':', _) => (PATHSEP, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b'=', b'>', _) => (FATARROW, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'(', ..) => (LPAREN, 1),
        (b')', ..) => (RPAREN, 1),
        (b'[', ..) => (LBRACKET, 1),
        (b']', ..) => (RBRACKET, 1),
        (b'{', ..) => (LBRACE, 1),
        (b'}', ..) => (RBRACE, 1),
        (b',', ..) => (COMMA, 1),
        (b';', ..) => (SEMI, 1),
        (b':', ..) => (COLON, 1),
        (b'.', ..) => (DOT, 1),
        (b'#', ..) => (HASH, 1),
        (b'$', ..) => (DOLLAR, 1),
        (b'?', ..) => (QUESTION, 1),
        (b'@', ..) => (AT, 1),
        (b'~', ..) => (TILDE, 1),
        (b'=', ..) => (ASSIGN, 1),
        (b'<', ..) => (LT, 1),
        (b'>', ..) => (GT, 1),
        (b'!', ..) => (BANG, 1),
        (b'+', ..) => (PLUS, 1),
        (b'-', ..) => (MINUS, 1),
        (b'*', ..) => (STAR, 1),
        (b'/', ..) => (SLASH, 1),
        (b'%', ..) => (PERCENT, 1),
        (b'^', ..) => (CARET, 1),
        (b'&', ..) => (AMP, 1),
        (b'|', ..) => (PIPE, 1),
        _ => return UNKNOWN,
    };
    c.bump_n(width);
    kind
}

#[cfg(test)]
mod tests {
    use super::super::budget::Budget;
    use super::super::parser::Workspace;
    use super::super::tests::{check, mutate, on_big_stack, Rng};
    use super::super::tree::HAS_ERROR;
    use super::*;

    fn parse(src: &str) -> Workspace {
        let mut workspace = Workspace::default();
        let outcome = workspace.parse(src.as_bytes(), &RUST, &Budget::default()).unwrap();
        assert!(outcome.complete(), "{src:?}: {}", outcome.describe());
        check(&workspace, src.as_bytes(), &RUST);
        workspace
    }

    fn named(src: &str) -> String {
        parse(src).tree.sexp(&RUST, true)
    }

    fn clean(src: &str) -> bool {
        parse(src).tree.root().flags & HAS_ERROR == 0
    }

    /// The expression in `fn f() { SRC; }`.
    fn expression(src: &str) -> String {
        let tree = named(&format!("fn f() {{ {src}; }}"));
        let prefix = "(source_file (function_item fn identifier (parameters) (block (expression_statement ";
        tree.strip_prefix(prefix).and_then(|rest| rest.strip_suffix("))))")).unwrap_or_else(|| panic!("{src}: {tree}")).to_string()
    }

    #[test]
    fn an_error_stays_inside_its_construct() {
        // #5910: a bad name is an error inside the function, and the items
        // around it survive whole.
        assert_eq!(
            named("fn 1() {}\nstruct S;\n"),
            "(source_file (function_item fn (ERROR integer_literal) (parameters) (block)) (struct_item struct identifier))"
        );
        let tree = named("impl X { fn a(&self) { let = 1; } fn b() {} }\n");
        assert!(tree.starts_with("(source_file (impl_item impl identifier (declaration_list (function_item fn identifier (parameters (self_parameter self)) (block (let_declaration let (ERROR"), "{tree}");
        assert!(tree.contains("(function_item fn identifier (parameters) (block))))"), "{tree}");
        // An unclosed brace costs its own block, not the file.
        let tree = named("fn a() { if x { b(); }\nfn c() {}\n");
        assert!(tree.contains("(function_item fn identifier (parameters) (block))"), "{tree}");
        // A bad token inside an argument list stays in that list.
        let tree = named("fn a() { f(1, $, 2); g(); }\n");
        assert!(tree.contains("(arguments integer_literal (ERROR $) integer_literal)") || tree.contains("(arguments integer_literal (ERROR"), "{tree}");
        assert!(tree.contains("(call_expression identifier (arguments))"), "{tree}");
    }

    #[test]
    fn operators_nest_by_precedence() {
        let cases = [
            ("a + b * c", "(binary_expression identifier (binary_expression identifier identifier))"),
            ("a * b + c", "(binary_expression (binary_expression identifier identifier) identifier)"),
            ("a - b - c", "(binary_expression (binary_expression identifier identifier) identifier)"),
            ("a = b = c", "(assignment_expression identifier (assignment_expression identifier identifier))"),
            ("a >> 2 > b", "(binary_expression (binary_expression identifier integer_literal) identifier)"),
            ("x >>= 1", "(compound_assignment_expr identifier integer_literal)"),
            ("a << b <= c", "(binary_expression (binary_expression identifier identifier) identifier)"),
            ("-x as u32", "(type_cast_expression (unary_expression identifier) as identifier)"),
            ("&mut *a.b()?", "(reference_expression mut (unary_expression (try_expression (call_expression (field_expression identifier identifier) (arguments)))))"),
            ("a || b && c == d", "(binary_expression identifier (binary_expression identifier (binary_expression identifier identifier)))"),
            ("x.0.1", "(field_expression (field_expression identifier integer_literal) integer_literal)"),
            ("0..n", "(range_expression integer_literal identifier)"),
            ("&raw const x", "(reference_expression raw const identifier)"),
            ("f(&raw)", "(call_expression identifier (arguments (reference_expression raw)))"),
            ("v.iter().map(|x| x + 1).collect::<Vec<_>>()", "(call_expression (field_expression (call_expression (field_expression (call_expression (field_expression identifier identifier) (arguments)) identifier) (arguments (closure_expression (closure_parameters identifier) (binary_expression identifier integer_literal)))) identifier (type_arguments (generic_type identifier (type_arguments)))) (arguments))"),
            ("S { a, b: 1, ..d }", "(struct_expression identifier (field_initializer_list (shorthand_field_initializer identifier) (field_initializer identifier integer_literal) (base_field_initializer identifier)))"),
            ("<T as Tr>::f()", "(call_expression (scoped_identifier (qualified_type identifier as identifier) identifier) (arguments))"),
            ("_ = f()", "(assignment_expression (call_expression identifier (arguments)))"),
        ];
        for (src, want) in cases {
            assert_eq!(expression(src), want, "{src:?}");
        }
    }

    #[test]
    fn generics_close_on_each_angle() {
        assert!(clean("fn f(a: Vec<Vec<u8>>, b: HashMap<K, Vec<Option<T>>>) -> Box<dyn Fn(u8) -> u8 + Send + 'static> {}\n"));
        assert!(clean("type A = Vec<<T as Iterator>::Item>;\nconst B: [u8; 3] = [0; 3];\n"));
        assert!(clean("fn f<'a, T: ?Sized + 'a, const N: usize = 3>(x: &'a T) where for<'b> F: Fn(&'b T), T: Iterator<Item = u8> {}\n"));
        assert!(clean("impl<T> Trait for Wrapper<T> where T: Clone { type Out<'a> = &'a T where Self: 'a; }\n"));
        assert!(clean("fn g() { let x: Vec<u8>= v; }\n") == false, "`>=` is a token, as documented");
    }

    #[test]
    fn statements_items_and_patterns() {
        assert!(clean("#![allow(x)]\n#[derive(Debug)]\npub(crate) struct S<T> { pub a: T, #[cfg(x)] b: u8 }\nenum E { A, B(u8), C { x: i32 } = 3 }\nunion U { a: u8 }\n"));
        assert!(clean("pub(in crate::a) unsafe extern \"C\" fn f(x: i32, ...) -> i32;\nextern \"C\" { static X: u8; fn g(); }\nextern crate alloc as a;\nmod m;\nuse a::{b, c::*, d as _};\n"));
        assert!(clean("pub unsafe trait T: Send where Self: Sized { const C: u8; fn f(&self) -> u8 { 0 } }\nunsafe impl T for S {}\nimpl !Send for S {}\ndefault fn d() {}\n"));
        assert!(clean("macro_rules! m { ($x:expr) => { $x + 1 }; }\nm!(1);\nlazy_static! { static ref X: u8 = 1; }\n"));
        assert!(clean("fn f() { let macro_rules = 0; let union = 1; let raw = &raw; default(); }\n"));
        assert!(clean("fn f() { match x { (a, b) if a > b => {} (Some(1..=5), None) | (None, _) => (), S { x: ref mut y, .. } => y, [first, .., last] => 1, &(a, b) => 2, -1 => 3, x @ 1..=9 => 4, _ => unreachable!() } }\n"));
        assert!(clean("fn f() { if let Some(x) = y && x > 0 && let Ok(z) = g() { } while let Some(v) = { it } { } let Some(a) = b else { return }; }\n"));
        assert!(clean("fn f() { unsafe { p.as_mut() }.field = 1; 'outer: loop { break 'outer; } let c = async move { x.await }; let k = |a: u8, (b, c): (u8, u8)| -> u8 { a }; }\n"));
        assert!(clean("fn f() { for i in 0.. { } if x == S::A { } let s = r#\"raw \"str\"#; let b = b'\\''; let l = 'a'; let lt: &'static str = \"\\\"\"; }\n"));
        // A block-like statement ends at its block: `(a, b)` is the next statement.
        let tree = named("fn f() { if c { } (a, b) = (b, a); }\n");
        assert!(tree.contains("(expression_statement (if_expression if identifier (block))) (expression_statement (assignment_expression (tuple_expression identifier identifier)"), "{tree}");
        // A condition or scrutinee may start with a block, as rustc allows.
        let tree = named("fn f() { while { step() } {} match { it }.next()? { _ => 1 } if {a} == b && c {} }\n");
        assert!(tree.contains("(while_expression while (block (expression_statement (call_expression identifier (arguments)))) (block))"), "{tree}");
        assert!(tree.contains("(match_expression match (try_expression (field_expression (block (expression_statement identifier)) identifier (arguments))) (match_block"), "{tree}");
        assert!(tree.contains("(if_expression if (binary_expression (binary_expression (block (expression_statement identifier)) identifier) identifier) (block))"), "{tree}");
        // Macros 2.0, as in core and std.
        assert_eq!(
            named("pub(crate) macro m($x:expr) { $x }\nmacro n { () => {} }\n"),
            "(source_file (decl_macro (visibility_modifier pub (visibility_restriction crate)) macro identifier (token_tree identifier identifier) (token_tree_brace identifier)) (decl_macro macro identifier (token_tree_brace (token_tree) (token_tree_brace))))"
        );
        // A struct literal is not allowed where a `{` opens a block.
        let tree = named("fn f() { if x == y { z } }\n");
        assert!(tree.contains("(if_expression if (binary_expression identifier identifier) (block"), "{tree}");
    }

    #[test]
    fn lexing_literals_lifetimes_and_comments() {
        let workspace = parse("/* a /* nested */ b */ 'a 'b' b\"x\" br##\"y\"# \"##z 1.5e3f64 0x1F_u8 1. r#match // end\n");
        let kinds: Vec<&str> = (0..workspace.tokens.len())
            .filter(|&at| !RUST.is_trivia(workspace.tokens.kind[at]))
            .map(|at| RUST.name(workspace.tokens.kind[at]))
            .collect();
        assert_eq!(kinds, ["lifetime", "char_literal", "string_literal", "raw_string_literal", "identifier", "float_literal", "integer_literal", "float_literal", "identifier"]);
    }

    /// Malformed inputs: each must report an error, and keep its root.
    #[test]
    fn malformed_inputs_report_errors() {
        let bad = [
            "fn f( {}\n",
            "fn f() { let x = ; }\n",
            "struct S { a: , }\n",
            "fn f() -> { }\n",
            "impl for X {}\n",
            "fn f() { x.; }\n",
            "fn f() { match x { => 1 } }\n",
            "enum E { A(,) }\n",
            "fn f() { let s = \"unterminated; }\n",
            "use a::{b;\n",
            "fn f() { if { } }\n",
            "trait T { fn f(&self) -> ; }\n",
            "fn f() { a + ; }\n",
            "pub pub fn f() {}\n",
            "pub macro m($x:expr);\n",
            "fn f() { while { a } }\n",
            "fn f() { let r = &raw; let p = &raw const; }\n",
        ];
        for src in bad {
            let workspace = parse(src);
            assert!(workspace.tree.root().flags & HAS_ERROR != 0, "{src:?} parsed clean: {}", workspace.tree.sexp(&RUST, true));
            assert_eq!(workspace.tree.root().kind, SOURCE_FILE);
        }
    }

    #[test]
    fn deep_input_stays_within_the_budget() {
        on_big_stack(|| {
            let deep = [
                format!("fn f() {{ {} }}", "(".repeat(100_000)),
                format!("fn f() {{ {}x; }}", "-".repeat(100_000)),
                format!("fn f() {{ {}x; }}", "&".repeat(100_000)),
                format!("fn f() {{ x = {}; }}", "a + ".repeat(100_000) + "b"),
                format!("fn f() {{ {}1 }}", "if x {} else ".repeat(30_000)),
                format!("type T = {}u8{};", "Vec<".repeat(50_000), ">".repeat(50_000)),
                format!("fn f() {{ {}x }}", "|a| ".repeat(100_000)),
                "{".repeat(100_000),
            ];
            for src in deep {
                let mut workspace = Workspace::default();
                let outcome = workspace.parse(src.as_bytes(), &RUST, &Budget::default()).unwrap();
                assert!(outcome.exhausted.is_none(), "{}", outcome.describe());
                check(&workspace, src.as_bytes(), &RUST);
                assert_eq!(workspace.tree.root().kind, SOURCE_FILE);
            }
        });
    }

    const SEEDS: &[&str] = &[
        "use std::collections::HashMap;\n\n#[derive(Debug, Clone)]\npub struct Graph<'a, T: Clone> {\n    nodes: Vec<Node<'a, T>>,\n    index: HashMap<&'a str, usize>,\n}\n\nimpl<'a, T: Clone> Graph<'a, T> {\n    pub fn add(&mut self, name: &'a str, value: T) -> usize {\n        if let Some(&at) = self.index.get(name) {\n            return at;\n        }\n        let at = self.nodes.len();\n        self.nodes.push(Node { name, value, edges: vec![] });\n        self.index.insert(name, at);\n        at\n    }\n}\n",
        "fn walk(tree: &Tree) -> Result<u32, Error> {\n    let mut total = 0u32;\n    for (i, node) in tree.nodes.iter().enumerate() {\n        match node.kind {\n            Kind::Leaf(n) if n > 0 => total += n as u32,\n            Kind::Branch { left, right } => {\n                total = total.checked_add(left * right).ok_or(Error::Overflow)?;\n            }\n            _ => continue,\n        }\n        while i > 3 && total >> 2 >= 10 { total -= 1; }\n    }\n    Ok(total)\n}\n",
        "macro_rules! square { ($x:expr) => { $x * $x }; }\n\npub trait Shape: Send + Sync {\n    fn area(&self) -> f64;\n    fn name(&self) -> String { String::from(\"shape\") }\n}\n\nasync fn run() -> std::io::Result<()> {\n    let data = tokio::fs::read(\"x\").await?;\n    let f = move |x: u8| -> u8 { x.wrapping_add(1) };\n    unsafe { ptr::write(p, square!(2)) };\n    Ok(())\n}\n",
    ];

    const RUST_BYTES: &[u8] = b"()[]{}<>:;,.=+-*/%!&|^~?@#$'\"\\_0123456789 \n\t xyfnletmutimplstructenummatchifelsewhereselfSelfpubusemod\xc3\xa9";

    #[test]
    fn fuzzed_rust_terminates_and_keeps_every_invariant() {
        on_big_stack(|| {
            let mut rng = Rng(0x5910_0004_0072_75ff);
            let mut workspace = Workspace::default();
            let budget = Budget::default();
            let mut erroneous = 0;
            const ROUNDS: usize = 3000;
            for round in 0..ROUNDS {
                let mut src = SEEDS[round % SEEDS.len()].as_bytes().to_vec();
                for _ in 0..1 + rng.below(6) {
                    let length = src.len();
                    mutate(&mut rng, &mut src, RUST_BYTES, b"", 0, length);
                }
                let outcome = workspace.parse(&src, &RUST, &budget).unwrap();
                assert!(outcome.exhausted.is_none(), "budget ran out on {:?}", String::from_utf8_lossy(&src));
                check(&workspace, &src, &RUST);
                assert_eq!(workspace.tree.root().kind, SOURCE_FILE);
                if workspace.tree.root().flags & HAS_ERROR != 0 {
                    erroneous += 1;
                }
            }
            assert!(erroneous > ROUNDS / 2, "{erroneous} of {ROUNDS}");
        });
        for seed in SEEDS {
            assert!(clean(seed), "{seed}");
        }
    }
}
