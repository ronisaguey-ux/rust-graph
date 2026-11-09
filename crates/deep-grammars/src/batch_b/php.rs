//! PHP through `grammar!`: one of Field 6's four languages (Ruby, PHP, Swift,
//! Scala).
//!
//! PHP is brace- and semicolon-delimited, so no layout rule is needed. A file
//! is not one language, though: it is PHP runs joined by inline HTML, so the
//! step lexer has a mode stack and the file *starts* in the HTML mode.
//!
//! ```text
//! HTML --`<?php`/`<?=`--> PHP --`?>`--> HTML
//!                           |  `"`      pushes DQUOTE   (text and fields)
//!                           |  `{$`     pushes FIELD    (ordinary code again)
//!                           |  `<<<X`   pushes HEREDOC  (heredoc text)
//! ```
//!
//! What the DSL covers here:
//!
//! - **The open tag, the close tag and inline HTML are statements.** `<?php`
//!   and `<?=` are tokens that switch the lexer to code, `?>` is a token that
//!   switches it back to HTML, and the bytes between two PHP runs are one
//!   `inline_html` token. `?>` also ends a statement, so `echo $x ?>` needs no
//!   `;` — that is PHP's own rule, not a recovery.
//! - **Variables are one token.** `$name` is `VARIABLE`, which is what makes
//!   `$x = 1`, `$this->y` and `$a[$i]` ordinary expression forms.
//! - **A member and a static member are postfix rows.** `$this->name(...)`,
//!   `Foo::bar`, `Foo::CONST` and `$obj?->name` are `member_access`,
//!   `scoped_property_access` and `nullsafe_member_access`, taken after their
//!   operands. A word after `->`, `?->` or `::` is a name even when it is a
//!   keyword (`$this->namespace`, `$a->list()`, `Foo::class`): the step lexer
//!   reads the previous significant token, which is what makes that possible
//!   without a contextual-keyword mode.
//! - **An interpolated string is a tree, not a token.** `"a{$o->m()}b"` is a
//!   `double_quoted_string` over its quote, its text runs and one
//!   `interpolation` per field; a call written in a field is an ordinary
//!   `function_call_expression`, and `?>` inside the string closes nothing.
//!   A heredoc (not a nowdoc) is a `heredoc_string` shaped the same way; a
//!   nowdoc is one `string` token, because nothing inside it is code.
//! - **`{$...}` and `${...}` are the same node.** Both push the field mode, so
//!   the braces keep their own spans and the code between them is parsed. The
//!   simple `"$a"`, `"$a->b"` and `"$a[0]"` forms are an `interpolated_variable`
//!   part instead: they name a variable, they do not run an expression.
//! - **Alternative syntax is a body, not a second grammar.** `if (...): ...
//!   endif;` is `if_statement` with a `":" _statement* ["endif" _end]` body,
//!   and the same shape is used by `while`, `for`, `foreach`, `switch` and
//!   `declare`. That is why a keyword such as `endif` only has to end the
//!   statement loop: it can never start a statement.
//!
//! What the DSL cannot express, and how it is handled here instead:
//!
//! - **A quoted string, a heredoc and a field are lexer states.** `modes` and
//!   `switch` are the runtime's own mechanism; the `{` of a field pushes the
//!   field frame and the `}` at that field's own bracket depth pops it, so
//!   `"{$a["k"]}"` closes at the right brace. A string that cannot close ends
//!   its frame on one bad token, and the mode stack is never left holding a
//!   frame the step function abandoned.
//! - **Attributes are siblings of what they decorate.** One decision sees at
//!   most three tokens, and `#[A] final public static function` puts the
//!   declaration keyword past that window. The attribute group is therefore a
//!   statement/member of its own, with its own span, instead of a child of the
//!   declaration it decorates. Nothing is lost from the bytes.
//! - **A cast is one token.** `(int)`, `(string)`, `(array)` and friends are
//!   lexed as a single `CAST` token when the bytes spell exactly one, so
//!   `(int) $x` cannot be confused with a parenthesized constant named `int`.
//! - **No negative lookahead.** A choice is decided by the next one to three
//!   tokens, so a construct that starts like an expression is entered through
//!   a leading predicate (`&(...)`): `function name(` is a declaration,
//!   `function (` is an anonymous function.
//! - **One kind per rule**, so `member_access` and `nullsafe_member_access` are
//!   two rules rather than one labelled alternative.
//!
//! Not modelled, and named rather than silently dropped: an intersection type
//! (`A&B`; union types `A|B` and nullable `?A` are), property hooks (PHP 8.4),
//! `__halt_compiler`, and short open tags (`<?`); `<?xml` at the top of a file
//! is inline HTML, which is what makes ignoring `<?` the safe choice. An `&`
//! after a parameter's type is a reference, which is the reading that matters:
//! `array &$options` is everywhere, `Countable&Traversable $x` is not.
//!
//! Measured, with `PHP_CORPUS=<dir> cargo test -p deep-grammars -- --ignored
//! --nocapture`: 5048 of 5053 files of WordPress, Guzzle, Laravel, Symfony
//! Console, PHPUnit, Composer, phpseclib and Doctrine parse with no error
//! node; the five that remain are the intersection types above. The same
//! WordPress and Guzzle files were 436 of 1553 before this grammar handled
//! `?>`, the interpolation and the older declaration forms.

use std::cell::RefCell;

use crate::deep::lexer::Cursor;
use crate::deep::syntax::{Kind, COMMENT, UNKNOWN, WHITESPACE};

rust_graph_deep::grammar! { PHP {
    name = "php";
    lex = lex_html;

    /// The mode stack of a file: HTML text, PHP code, the text of a
    /// double-quoted string or heredoc, and the code of an interpolation.
    modes {
        HTML = lex_html,
        PHP = lex_php,
        DQUOTE = lex_dquote,
        HEREDOC = lex_heredoc,
        FIELD = lex_field,
    }
    switch OPEN_TAG => PHP;
    switch ECHO_TAG => PHP;
    switch CLOSE_TAG => HTML;
    switch DQUOTE_START => push DQUOTE;
    switch DQUOTE_END => pop DQUOTE;
    /// A string that cannot close takes its own frame and every field opened
    /// inside it, so the mode stack cannot hold a frame the lexer has left.
    switch DQUOTE_BAD => pop DQUOTE;
    switch INTERPOLATION_OPEN => push FIELD;
    switch INTERPOLATION_CLOSE => pop;
    switch HEREDOC_START => push HEREDOC;
    switch HEREDOC_END => pop HEREDOC;
    switch HEREDOC_BAD => pop HEREDOC;

    tokens {
        /// `<?php`: the tag itself switches the lexer to code.
        OPEN_TAG = "<?php",
        /// `<?=`: the short echo tag, an `echo` with its `<?php`.
        ECHO_TAG = "<?=",
        /// `?>`: ends the PHP run and the statement it ends.
        CLOSE_TAG = "?>",
        /// Everything between two PHP runs.
        INLINE_HTML = "inline_html",
        /// `#[`: an attribute group is a bracket pair.
        ATTRIBUTE_START = "#[",

        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        LBRACKET = "[" open brackets,
        RBRACKET = "]" close brackets,
        LBRACE = "{" open braces,
        RBRACE = "}" close braces,
        IDENT = "name",
        VARIABLE = "variable_name",
        NUMBER = "integer",
        FLOAT = "float",
        /// A single-quoted, backtick or nowdoc string: nothing inside is code.
        STRING = "string",
        /// A string literal with no end.
        BAD_LITERAL = "bad_literal" bad,
        /// The opening quote of a double-quoted string.
        DQUOTE_START = "string_start",
        /// The bytes between the fields of a double-quoted string.
        DQUOTE_TEXT = "string_content",
        /// The closing quote of a double-quoted string.
        DQUOTE_END = "string_end",
        /// A double-quoted string that cannot close.
        DQUOTE_BAD = "bad_string" bad,
        /// The `<<<X` line that opens a heredoc.
        HEREDOC_START = "heredoc_start",
        /// The terminator line of a heredoc, marker included.
        HEREDOC_END = "heredoc_end",
        /// A heredoc with no terminator line.
        HEREDOC_BAD = "bad_heredoc" bad,
        /// The `{` of `{$...}` or the `$`-brace of `${...}`: code again until
        /// the brace that matches it.
        INTERPOLATION_OPEN = "interpolation_open" open field,
        /// The `}` that closes an interpolation at its own depth.
        INTERPOLATION_CLOSE = "interpolation_close" close field,
        /// `(int)`, `(string)`, `(array)`: a cast is one token.
        CAST = "cast",

        KW_ABSTRACT = "abstract",
        KW_AND = "and",
        KW_ARRAY = "array",
        KW_AS = "as",
        KW_BREAK = "break",
        KW_CASE = "case",
        KW_CATCH = "catch",
        KW_CLASS = "class",
        KW_CLONE = "clone",
        KW_CONST = "const",
        KW_CONTINUE = "continue",
        KW_DECLARE = "declare",
        KW_DEFAULT = "default",
        KW_DO = "do",
        KW_ECHO = "echo",
        KW_ELSE = "else",
        KW_ELSEIF = "elseif",
        KW_EMPTY = "empty",
        KW_ENDDECLARE = "enddeclare",
        KW_ENDFOR = "endfor",
        KW_ENDFOREACH = "endforeach",
        KW_ENDIF = "endif",
        KW_ENDSWITCH = "endswitch",
        KW_ENDWHILE = "endwhile",
        KW_ENUM = "enum",
        KW_EXTENDS = "extends",
        KW_FALSE = "false",
        KW_FINAL = "final",
        KW_FINALLY = "finally",
        KW_FN = "fn",
        KW_FOR = "for",
        KW_FOREACH = "foreach",
        KW_FROM = "from",
        KW_FUNCTION = "function",
        KW_GLOBAL = "global",
        KW_GOTO = "goto",
        KW_IF = "if",
        KW_IMPLEMENTS = "implements",
        KW_INCLUDE = "include",
        KW_INCLUDE_ONCE = "include_once",
        KW_INSTANCEOF = "instanceof",
        KW_INSTEADOF = "insteadof",
        KW_INTERFACE = "interface",
        KW_ISSET = "isset",
        KW_LIST = "list",
        KW_MATCH = "match",
        KW_NAMESPACE = "namespace",
        KW_NEW = "new",
        KW_NULL = "null",
        KW_OR = "or",
        KW_PARENT = "parent",
        KW_PRINT = "print",
        KW_PRIVATE = "private",
        KW_PROTECTED = "protected",
        KW_PUBLIC = "public",
        KW_READONLY = "readonly",
        KW_REQUIRE = "require",
        KW_REQUIRE_ONCE = "require_once",
        KW_RETURN = "return",
        KW_SELF = "self",
        KW_STATIC = "static",
        KW_SWITCH = "switch",
        KW_THROW = "throw",
        KW_TRAIT = "trait",
        KW_TRUE = "true",
        KW_TRY = "try",
        KW_UNSET = "unset",
        KW_USE = "use",
        KW_VAR = "var",
        KW_WHILE = "while",
        KW_XOR = "xor",
        KW_YIELD = "yield",

        SEMI = ";",
        COMMA = ",",
        COLON = ":",
        DOT = ".",
        ELLIPSIS = "...",
        ARROW = "->",
        NULLSAFE = "?->",
        COLONCOLON = "::",
        FATARROW = "=>",
        BACKSLASH = "\\",
        QUESTION = "?",
        AT = "@",
        ASSIGN = "=",
        PLUS_EQ = "+=",
        MINUS_EQ = "-=",
        STAR_EQ = "*=",
        SLASH_EQ = "/=",
        DOT_EQ = ".=",
        PERCENT_EQ = "%=",
        STARSTAR_EQ = "**=",
        AMP_EQ = "&=",
        PIPE_EQ = "|=",
        CARET_EQ = "^=",
        SHL_EQ = "<<=",
        SHR_EQ = ">>=",
        NULLISH_EQ = "??=",
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
        SPACESHIP = "<=>",
        EQ = "==",
        IDENTICAL = "===",
        NE = "!=",
        ANGLE_NE = "<>",
        NOT_IDENTICAL = "!==",
        ANDAND = "&&",
        OROR = "||",
        NULLISH = "??",
        SHL = "<<",
        SHR = ">>",
        INCREMENT = "++",
        DECREMENT = "--",
    }

    program: _statement*,

    _statement:
        attribute_group
        // A declaration is entered through the tokens that make it one: the
        // keyword alone is not enough, because `function (` is an expression.
        | &([top_modifiers] "class") class_declaration
        | &([top_modifiers] "interface") interface_declaration
        | &([top_modifiers] "trait") trait_declaration
        | &([top_modifiers] "enum") enum_declaration
        | &("function" ["&"] IDENT) function_declaration
        | &("const" IDENT "=") const_declaration
        | namespace_declaration
        | use_declaration
        | if_statement
        | while_statement
        | do_statement
        | for_statement
        | foreach_statement
        | switch_statement
        | try_statement
        | return_statement
        | break_statement
        | continue_statement
        | echo_statement
        | global_statement
        | &("static" VARIABLE) static_statement
        | unset_statement
        | declare_statement
        | goto_statement
        | &(IDENT ":") labeled_statement
        | block
        | OPEN_TAG
        | CLOSE_TAG
        | INLINE_HTML
        | ";"
        | expression_statement,

    /// A statement ends at `;` or at `?>`, which PHP counts as a `;`.
    _end: ";" | CLOSE_TAG,

    // ---- declarations ----

    namespace_declaration: "namespace" [_qualified_name] (";" | namespace_block),
    namespace_block: "{" _statement* "}",
    use_declaration: "use" [("function" | "const")] (_use_name % ",") ";",
    /// `Foo\Bar`, and `Foo\{Bar, Baz as Qux}`: the group is a tail of the same
    /// name, so the two forms share every token up to the brace the group needs.
    _use_name: [BACKSLASH] _name use_segment* ["as" _name],
    /// `\Bar`, or `\{Bar, Baz}`: the token after the backslash decides.
    use_segment: BACKSLASH (_name | use_group),
    use_group: "{" (_use_item %? ",") "}",
    _use_item: _qualified_name ["as" _name],

    /// An attribute group is a statement or a member of its own; see the module
    /// comment for why it is not a child of the declaration it decorates.
    attribute_group: "#[" (_attribute %? ",") "]",
    _attribute: _qualified_name [arguments],

    modifiers: _modifier+,
    _modifier: "public" | "protected" | "private" | "static" | "abstract" | "final" | "readonly" | "var",
    /// What a top-level declaration may have: never `static`, which at the top
    /// level starts a `static::` expression instead.
    top_modifiers: ("abstract" | "final" | "readonly")+,

    class_declaration: [top_modifiers] "class" _name ["extends" _qualified_name] ["implements" _interface_list] class_body,
    interface_declaration: [top_modifiers] "interface" _name ["extends" _interface_list] class_body,
    trait_declaration: [top_modifiers] "trait" _name class_body,
    enum_declaration: [top_modifiers] "enum" _name [":" _type] ["implements" _interface_list] enum_body,
    enum_body: "{" enum_member* "}",
    enum_member: ";" | attribute_group | [modifiers] (enum_case | member_declaration),
    enum_case: "case" _name ["=" _expression] ";",
    class_body: "{" class_member* "}",
    /// Attributes and modifiers come first, the declaration keyword decides.
    class_member: ";" | attribute_group | [modifiers] member_declaration,
    member_declaration: method_declaration | const_declaration | trait_use_declaration | property_declaration,

    method_declaration: "function" ["&"] _name formal_parameters [":" _type] (block | ";"),
    function_declaration: "function" ["&"] _name formal_parameters [":" _type] (block | ";"),
    formal_parameters: "(" (formal_parameter %? ",") ")",
    formal_parameter: attribute_group* [modifiers] [_type] ["&"] ["..."] VARIABLE ["=" _expression],
    property_declaration: [_type] _property_item ("," _property_item)* ";",
    _property_item: VARIABLE ["=" _expression],
    const_declaration: "const" _const_item ("," _const_item)* ";",
    _const_item: _name "=" _expression,
    trait_use_declaration: "use" _qualified_name ("," _qualified_name)* [trait_adaptations],
    trait_adaptations: "{" trait_adaptation* "}",
    /// `A::foo insteadof B;` and `A::foo as protected bar;`
    trait_adaptation: trait_reference trait_adaptation_tail ";",
    trait_adaptation_tail: "insteadof" _qualified_name ("," _qualified_name)* | "as" [modifiers] [_name],
    trait_reference: _qualified_name ["::" _name],
    _interface_list: _qualified_name ("," _qualified_name)*,

    // ---- statements ----

    block: "{" _statement* "}",
    expression_statement: _expression _end,
    /// The body of a control structure: a block, an alternative-syntax run, or
    /// one statement (`if ($x) return;`).
    _body: ":" _statement* | _statement,
    if_statement: "if" parenthesized_expression _body elseif_clause* [else_clause] ["endif" _end],
    elseif_clause: "elseif" parenthesized_expression _body,
    else_clause: "else" _body,
    while_statement: "while" parenthesized_expression _body ["endwhile" _end],
    do_statement: "do" _statement "while" parenthesized_expression _end,
    for_statement: "for" for_header _body ["endfor" _end],
    for_header: "(" [_expression_list] ";" [_expression_list] ";" [_expression_list] ")",
    _expression_list: _expression ("," _expression)*,
    foreach_statement: "foreach" foreach_header _body ["endforeach" _end],
    foreach_header: "(" _expression "as" _foreach_target ")",
    _foreach_target: &(_expression "=>") key_value_target | _expression,
    key_value_target: _expression "=>" _expression,
    switch_statement: "switch" parenthesized_expression (switch_body | switch_alternative),
    switch_body: "{" (case_clause | default_clause)* "}",
    switch_alternative: ":" (case_clause | default_clause)* ["endswitch" _end],
    case_clause: "case" _expression (":" | ";") _statement*,
    default_clause: "default" (":" | ";") _statement*,
    try_statement: "try" block catch_clause* [finally_clause],
    catch_clause: "catch" catch_header block,
    catch_header: "(" _catch_types [VARIABLE] ")",
    _catch_types: _qualified_name ("|" _qualified_name)*,
    finally_clause: "finally" block,
    return_statement: "return" [_expression] _end,
    break_statement: "break" [NUMBER] _end,
    continue_statement: "continue" [NUMBER] _end,
    echo_statement: ("echo" | ECHO_TAG) _expression ("," _expression)* _end,
    global_statement: "global" (VARIABLE % ",") _end,
    /// `static $cache = [];` inside a function.
    static_statement: "static" static_item ("," static_item)* _end,
    static_item: VARIABLE ["=" _expression],
    unset_statement: "unset" unset_arguments _end,
    unset_arguments: "(" (_expression %? ",") ")",
    declare_statement: "declare" declare_arguments (";" | ":" _statement* ["enddeclare" _end] | block),
    declare_arguments: "(" (declare_item %? ",") ")",
    declare_item: _name "=" _expression,
    goto_statement: "goto" _name _end,
    labeled_statement: _name ":",
    /// `$x ?: require $y`: the include family is an expression in PHP.
    include_expression: ("require" | "require_once" | "include" | "include_once") _expression,
    parenthesized_expression: "(" _expression ")",

    // ---- expressions ----

    _expression: precedence _primary {
        assign: right "=" | "+=" | "-=" | "*=" | "/=" | ".=" | "%=" | "**=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "??=" => assignment_expression,
        word_or: left "or" => binary_expression,
        word_xor: left "xor" => binary_expression,
        word_and: left "and" => binary_expression,
        cond: postfix "?" _expression ":" _expression@assign => conditional_expression,
        short_cond: postfix "?" ":" _expression@assign => conditional_expression,
        nullish: left "??" => binary_expression,
        or: left "||" => binary_expression,
        and: left "&&" => binary_expression,
        bor: left "|" => binary_expression,
        xor: left "^" => binary_expression,
        band: left "&" => binary_expression,
        eq: left "==" | "!=" | "===" | "!==" | "<>" | "<=>" => binary_expression,
        cmp: left "<" | ">" | "<=" | ">=" => binary_expression,
        instance: left "instanceof" => instance_of_expression,
        shift: left "<<" | ">>" => binary_expression,
        add: left "+" | "-" | "." => binary_expression,
        mul: left "*" | "/" | "%" => binary_expression,
        exp: right "**" => binary_expression,
        cast: prefix CAST => cast_expression,
        clone: prefix "clone" => clone_expression,
        print: prefix "print" => print_expression,
        throw: prefix "throw" => throw_expression,
        unary: prefix "!" | "~" | "-" | "+" | "@" => unary_expression,
        reference: prefix "&" => reference_expression,
        update: prefix "++" | "--" => update_expression,
        postfix "++" | "--" => update_expression,
        call: postfix arguments => function_call_expression,
        index: postfix subscript => subscript_expression,
        member: postfix "->" (_member_name | VARIABLE | dynamic_member) => member_access_expression,
        nullsafe: postfix "?->" (_member_name | VARIABLE | dynamic_member) => nullsafe_member_access_expression,
        scope: postfix "::" (_member_name | VARIABLE) => scoped_property_access_expression,
    },

    _primary:
        VARIABLE
        | NUMBER
        | FLOAT
        | string_literal
        | "true"
        | "false"
        | "null"
        | "self"
        | "parent"
        | static_arrow_function
        | static_anonymous_function
        | "static"
        | parenthesized_expression
        | array_literal
        | array_expression
        | new_expression
        | isset_expression
        | empty_expression
        | list_expression
        | match_expression
        | include_expression
        | anonymous_function
        | arrow_function
        | yield_expression
        | _qualified_name
        | BACKSLASH _literal_name,

    /// A double-quoted string or a heredoc is a tree; a nowdoc, a single-quoted
    /// string and a backtick string are one token.
    string_literal: (STRING | BAD_LITERAL | double_quoted_string | heredoc_string)+,
    double_quoted_string: DQUOTE_START _string_part* (DQUOTE_END | DQUOTE_BAD),
    heredoc_string: HEREDOC_START _string_part* (HEREDOC_END | HEREDOC_BAD),
    _string_part: string_content | interpolation | interpolated_variable,
    string_content: DQUOTE_TEXT,
    /// `{$obj->m()}` or `${name}`: the braces keep their own spans and the code
    /// between them is ordinary grammar.
    interpolation: INTERPOLATION_OPEN _expression INTERPOLATION_CLOSE,
    /// `"$a"`, `"$a->b"`, `"$a[0]"`: a variable named in a string, not code.
    interpolated_variable: VARIABLE [simple_member | simple_subscript],
    simple_member: "->" _member_name,
    simple_subscript: "[" _simple_key "]",
    _simple_key: IDENT | NUMBER | VARIABLE,

    _qualified_name: [BACKSLASH] _name (BACKSLASH _name)*,
    _name: IDENT,
    _literal_name: "true" | "false" | "null",
    /// After `->`, `?->` and `::` every word is a member name.
    _member_name: IDENT | "class",
    /// `$data->{$column}`: a computed member name.
    dynamic_member: "{" _expression "}",

    new_expression: "new" _new_class [arguments],
    _new_class: "self" | "parent" | "static" | VARIABLE | _qualified_name | anonymous_class,
    anonymous_class: "class" [arguments] ["extends" _qualified_name] ["implements" _interface_list] class_body,
    anonymous_function: "function" ["&"] formal_parameters ["use" closure_use] [":" _type] block,
    closure_use: "(" (closure_use_item %? ",") ")",
    closure_use_item: ["&"] VARIABLE,
    arrow_function: "fn" ["&"] formal_parameters [":" _type] "=>" _expression,
    /// `static fn () => ...` and `static function () {}`: the modifier is the
    /// same token a `static::` call starts with, so each is its own rule.
    static_arrow_function: "static" arrow_function,
    static_anonymous_function: "static" anonymous_function,
    match_expression: "match" parenthesized_expression match_body,
    match_body: "{" [match_arms] "}",
    match_arms: _arm %? ",",
    _arm: match_arm | default_arm,
    match_arm: _expression ("," _expression)* "=>" _expression,
    default_arm: "default" "=>" _expression,
    yield_expression: "yield" [yield_body],
    yield_body: "from" _expression | _expression ["=>" _expression],

    arguments: "(" (_argument %? ",") ")",
    _argument: &(_name ":") named_argument | spread_element | _expression,
    named_argument: _name ":" _expression,
    /// `f(...$xs)` and, with no operand, the first-class callable `f(...)`.
    spread_element: "..." [_expression],
    subscript: "[" [_expression] "]",

    array_literal: "[" ([_array_element] ("," [_array_element])*) "]",
    array_expression: "array" array_arguments,
    array_arguments: "(" (_array_element %? ",") ")",
    _array_element: &(_expression "=>") pair_element | spread_element | _expression,
    pair_element: _expression "=>" _expression,

    isset_expression: "isset" isset_arguments,
    isset_arguments: "(" (_expression % ",") ")",
    empty_expression: "empty" empty_arguments,
    empty_arguments: "(" _expression ")",
    list_expression: "list" list_arguments,
    list_arguments: "(" ([_expression] ("," [_expression])*) ")",

    // ---- types ----

    _type: ["?"] _type_atom ("|" _type_atom)*,
    _type_atom: _type_name | parenthesized_type,
    /// `(A&B)|C`: a DNF type.
    parenthesized_type: "(" _type ")",
    _type_name: [BACKSLASH] _type_word (BACKSLASH _word)*,
    _type_word: IDENT | "array" | "self" | "parent" | "static" | "null" | "false" | "true",
    _word: IDENT,
} }

thread_local! {
    /// The marker of every heredoc the `HEREDOC` mode is inside, innermost last:
    /// a heredoc opened inside an interpolated string restores the outer one.
    static HEREDOC_MARKERS: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    /// One frame per open interpolation: the brackets opened inside it. The
    /// frame is pushed when the lexer emits `INTERPOLATION_OPEN` and popped on
    /// the `}` that matches it, so `"{$a["k"]}"` closes at the right brace.
    static FIELDS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// What the simple form of an interpolation (`"$a->b"`, `"$a[0]"`) may
    /// continue with, one frame per open string or heredoc. Without it the
    /// `->b` and `[0]` would be text: they are code only right after `$a`.
    static TAILS: RefCell<Vec<Tail>> = const { RefCell::new(Vec::new()) };
}

/// How a string's text may continue after the token just emitted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tail {
    /// Ordinary text.
    None,
    /// Right after a `$name`: a `->` or a `[` continues the interpolation.
    AfterVariable,
    /// Right after that `->`: a member name follows.
    AfterArrow,
    /// Inside `$name[`: a key and then the `]`.
    InIndex,
}

fn tail_push() {
    TAILS.with(|tails| tails.borrow_mut().push(Tail::None));
}

fn tail_pop() {
    TAILS.with(|tails| {
        tails.borrow_mut().pop();
    });
}

fn tail() -> Tail {
    TAILS.with(|tails| tails.borrow().last().copied().unwrap_or(Tail::None))
}

fn set_tail(tail: Tail) {
    TAILS.with(|tails| {
        if let Some(frame) = tails.borrow_mut().last_mut() {
            *frame = tail;
        }
    });
}

/// The HTML mode, mode 0: the file starts here. Bytes are `inline_html` up to
/// the tag that opens code. Only `<?php` and `<?=` open it: short tags would
/// turn `<?xml` in a template into a PHP run.
pub fn lex_html(c: &mut Cursor) -> Kind {
    if c.pos() == 0 && c.starts_with(b"\xef\xbb\xbf") {
        c.bump_n(3);
        return WHITESPACE;
    }
    if c.starts_with(b"<?php") {
        c.bump_n(5);
        return OPEN_TAG;
    }
    if c.starts_with(b"<?=") {
        c.bump_n(3);
        return ECHO_TAG;
    }
    let start = c.pos();
    while !c.at_end() {
        if c.peek() == b'<' && (c.starts_with(b"<?php") || c.starts_with(b"<?=")) {
            break;
        }
        c.bump();
    }
    if c.pos() == start {
        // A lone `<` that opens no tag: consume it so the step always advances.
        c.bump();
    }
    INLINE_HTML
}

/// One token of PHP code: mode 0's other half, and the `PHP` mode itself.
pub fn lex_php(c: &mut Cursor) -> Kind {
    match c.peek() {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c));
            WHITESPACE
        }
        b'<' if c.starts_with(b"<?php") => {
            c.bump_n(5);
            OPEN_TAG
        }
        b'<' if c.starts_with(b"<<<") => heredoc(c),
        b'?' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            CLOSE_TAG
        }
        b'/' if c.peek_at(1) == b'/' => {
            line_comment(c);
            COMMENT
        }
        b'#' if c.peek_at(1) == b'[' => {
            c.bump_n(2);
            ATTRIBUTE_START
        }
        b'#' => {
            line_comment(c);
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            block_comment(c);
            COMMENT
        }
        b'\'' | b'`' => string(c),
        b'"' => {
            c.bump();
            tail_push();
            DQUOTE_START
        }
        b'$' if is_word_start(c.peek_at(1)) => {
            c.bump();
            c.eat_while(is_word_byte);
            VARIABLE
        }
        // `$$name` is one variable-variable token.
        b'$' if c.peek_at(1) == b'$' && is_word_start(c.peek_at(2)) => {
            c.bump_n(2);
            c.eat_while(is_word_byte);
            VARIABLE
        }
        b'0'..=b'9' => number(c),
        b'.' if c.peek_at(1).is_ascii_digit() => number(c),
        byte if is_word_start(byte) => word(c),
        _ => operator(c),
    }
}

/// The `DQUOTE` mode: the text of a double-quoted string, its closing quote, or
/// a `{$`/`${` that opens an interpolation. A lone `$` and a `{` not followed
/// by one are text, which is what PHP itself decided.
pub fn lex_dquote(c: &mut Cursor) -> Kind {
    if c.at_end() {
        tail_pop();
        return DQUOTE_BAD;
    }
    if let Some(kind) = tail_step(c) {
        return kind;
    }
    match c.peek() {
        b'"' => {
            c.bump();
            tail_pop();
            return DQUOTE_END;
        }
        b'\\' => {
            c.bump_n(2);
            return DQUOTE_TEXT;
        }
        b'{' if c.peek_at(1) == b'$' => {
            // `{$name}`: the `$` belongs to the variable, so only the brace is
            // the field's opener and the code inside starts at the `$`.
            c.bump();
            open_field();
            return INTERPOLATION_OPEN;
        }
        b'$' if c.peek_at(1) == b'{' => {
            // `${name}`: the `$` is the interpolation's own.
            c.bump_n(2);
            open_field();
            return INTERPOLATION_OPEN;
        }
        b'$' if is_word_start(c.peek_at(1)) => {
            c.bump();
            c.eat_while(is_word_byte);
            set_tail(Tail::AfterVariable);
            return VARIABLE;
        }
        _ => {}
    }
    set_tail(Tail::None);
    // Text up to the next quote, escape or interpolation.
    loop {
        if c.at_end() {
            return DQUOTE_TEXT;
        }
        match c.peek() {
            b'"' => return DQUOTE_TEXT,
            b'\\' => return DQUOTE_TEXT,
            b'{' if c.peek_at(1) == b'$' => return DQUOTE_TEXT,
            b'$' if c.peek_at(1) == b'{' || is_word_start(c.peek_at(1)) => return DQUOTE_TEXT,
            _ => c.bump(),
        }
    }
}

/// The tokens that continue a simple interpolation: `->name`, and `[key]` with
/// its one key token. Nothing else in a string's text is code.
fn tail_step(c: &mut Cursor) -> Option<Kind> {
    match tail() {
        Tail::AfterVariable => {
            if c.starts_with(b"->") {
                c.bump_n(2);
                set_tail(Tail::AfterArrow);
                return Some(ARROW);
            }
            if c.peek() == b'[' {
                c.bump();
                set_tail(Tail::InIndex);
                return Some(LBRACKET);
            }
            set_tail(Tail::None);
        }
        Tail::AfterArrow => {
            if is_word_start(c.peek()) {
                c.eat_while(is_word_byte);
                set_tail(Tail::None);
                return Some(IDENT);
            }
            set_tail(Tail::None);
        }
        Tail::InIndex => match c.peek() {
            b']' => {
                c.bump();
                set_tail(Tail::None);
                return Some(RBRACKET);
            }
            b'$' if is_word_start(c.peek_at(1)) => {
                c.bump();
                c.eat_while(is_word_byte);
                return Some(VARIABLE);
            }
            b'0'..=b'9' => return Some(number(c)),
            b'-' if c.peek_at(1).is_ascii_digit() => return Some(number(c)),
            byte if is_word_start(byte) => {
                c.eat_while(is_word_byte);
                return Some(IDENT);
            }
            _ => set_tail(Tail::None),
        },
        Tail::None => {}
    }
    None
}

/// The `FIELD` mode: ordinary code, with one token that only exists inside an
/// interpolation. At the field's own depth a `}` closes it; deeper, it is a
/// block's or an array's brace and `lex_php` takes it.
pub fn lex_field(c: &mut Cursor) -> Kind {
    let Some(depth) = FIELDS.with(|fields| fields.borrow().last().copied()) else {
        // No field is open and the frame disagrees with the mode stack: lexing
        // code cannot lose a token, and the parser still gets a tree.
        return lex_php(c);
    };
    if depth == 0 && c.peek() == b'}' {
        c.bump();
        close_field();
        return INTERPOLATION_CLOSE;
    }
    let kind = lex_php(c);
    match kind {
        LPAREN | LBRACKET | LBRACE | ATTRIBUTE_START => with_field(|depth| *depth += 1),
        RPAREN | RBRACKET | RBRACE => with_field(|depth| *depth = depth.saturating_sub(1)),
        _ => {}
    }
    kind
}

/// The `HEREDOC` mode: the body of a heredoc up to its terminator line, one
/// interpolation, or one `$name`.
pub fn lex_heredoc(c: &mut Cursor) -> Kind {
    let Some(marker) = HEREDOC_MARKERS.with(|markers| markers.borrow().last().cloned()) else {
        return lex_php(c);
    };
    if c.at_end() {
        HEREDOC_MARKERS.with(|markers| {
            markers.borrow_mut().pop();
        });
        return HEREDOC_BAD;
    }
    if heredoc_terminator(c, &marker) {
        c.bump_n(terminator_width(c, &marker));
        HEREDOC_MARKERS.with(|markers| {
            markers.borrow_mut().pop();
        });
        return HEREDOC_END;
    }
    match c.peek() {
        b'{' if c.peek_at(1) == b'$' => {
            // `{$name}`: the `$` belongs to the variable, so only the brace is
            // the field's opener and the code inside starts at the `$`.
            c.bump();
            open_field();
            return INTERPOLATION_OPEN;
        }
        b'$' if c.peek_at(1) == b'{' => {
            // `${name}`: the `$` is the interpolation's own.
            c.bump_n(2);
            open_field();
            return INTERPOLATION_OPEN;
        }
        b'$' if is_word_start(c.peek_at(1)) => {
            c.bump();
            c.eat_while(is_word_byte);
            return VARIABLE;
        }
        _ => {}
    }
    let start = c.pos();
    while !c.at_end() {
        match c.peek() {
            b'\n' => {
                c.bump();
                if heredoc_terminator(c, &marker) {
                    break;
                }
            }
            b'{' if c.peek_at(1) == b'$' => break,
            b'$' if c.peek_at(1) == b'{' || is_word_start(c.peek_at(1)) => break,
            b'\\' => c.bump_n(2),
            _ => c.bump(),
        }
    }
    if c.pos() == start {
        c.bump();
    }
    DQUOTE_TEXT
}

/// The marker's line starts here: indentation, the marker, and no word byte
/// after it (`EOT;`, `EOT,` and a bare `EOT` all close a heredoc).
fn heredoc_terminator(c: &Cursor, marker: &[u8]) -> bool {
    if c.pos() != 0 && c.behind(1) != b'\n' {
        return false;
    }
    let rest = c.rest();
    let mut at = 0;
    while matches!(rest.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    rest.get(at..).is_some_and(|tail| tail.starts_with(marker)) && !rest.get(at + marker.len()).is_some_and(|byte| is_word_byte(*byte))
}

/// Bytes from the cursor through the terminator's marker.
fn terminator_width(c: &Cursor, marker: &[u8]) -> usize {
    let rest = c.rest();
    let mut at = 0;
    while matches!(rest.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    at + marker.len()
}

fn open_field() {
    FIELDS.with(|fields| fields.borrow_mut().push(0));
}

fn close_field() {
    FIELDS.with(|fields| {
        fields.borrow_mut().pop();
    });
}

fn with_field(change: impl FnOnce(&mut u32)) {
    FIELDS.with(|fields| {
        if let Some(depth) = fields.borrow_mut().last_mut() {
            change(depth);
        }
    });
}

#[inline]
fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

#[inline]
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn word(c: &mut Cursor) -> Kind {
    let start = c.rest();
    let length = c.eat_while(is_word_byte);
    let text = &start[..length];
    // A word after `->`, `?->` or `::` is a name, whatever it spells:
    // `$this->namespace`, `$a->list()`, `Foo::class`.
    if matches!(c.previous(), ARROW | NULLSAFE | COLONCOLON | KW_FUNCTION) {
        return IDENT;
    }
    // `f(name: 1)`, `f(default: false)`: a keyword before a `:` right after an
    // argument opener or a comma is a named argument's name.
    if matches!(c.previous(), LPAREN | COMMA) && named_argument_ahead(c) {
        return IDENT;
    }
    keyword(text)
}

fn keyword(text: &[u8]) -> Kind {
    match text {
        b"abstract" => KW_ABSTRACT,
        b"and" => KW_AND,
        b"array" => KW_ARRAY,
        b"as" => KW_AS,
        b"break" => KW_BREAK,
        b"case" => KW_CASE,
        b"catch" => KW_CATCH,
        b"class" => KW_CLASS,
        b"clone" => KW_CLONE,
        b"const" => KW_CONST,
        b"continue" => KW_CONTINUE,
        b"declare" => KW_DECLARE,
        b"default" => KW_DEFAULT,
        b"do" => KW_DO,
        b"echo" => KW_ECHO,
        b"else" => KW_ELSE,
        b"elseif" => KW_ELSEIF,
        b"empty" => KW_EMPTY,
        b"enddeclare" => KW_ENDDECLARE,
        b"endfor" => KW_ENDFOR,
        b"endforeach" => KW_ENDFOREACH,
        b"endif" => KW_ENDIF,
        b"endswitch" => KW_ENDSWITCH,
        b"endwhile" => KW_ENDWHILE,
        b"enum" => KW_ENUM,
        b"extends" => KW_EXTENDS,
        b"false" => KW_FALSE,
        b"final" => KW_FINAL,
        b"finally" => KW_FINALLY,
        b"fn" => KW_FN,
        b"for" => KW_FOR,
        b"foreach" => KW_FOREACH,
        b"from" => KW_FROM,
        b"function" => KW_FUNCTION,
        b"global" => KW_GLOBAL,
        b"goto" => KW_GOTO,
        b"if" => KW_IF,
        b"implements" => KW_IMPLEMENTS,
        b"include" => KW_INCLUDE,
        b"include_once" => KW_INCLUDE_ONCE,
        b"instanceof" => KW_INSTANCEOF,
        b"insteadof" => KW_INSTEADOF,
        b"interface" => KW_INTERFACE,
        b"isset" => KW_ISSET,
        b"list" => KW_LIST,
        b"match" => KW_MATCH,
        b"namespace" => KW_NAMESPACE,
        b"new" => KW_NEW,
        b"null" => KW_NULL,
        b"or" => KW_OR,
        b"parent" => KW_PARENT,
        b"print" => KW_PRINT,
        b"private" => KW_PRIVATE,
        b"protected" => KW_PROTECTED,
        b"public" => KW_PUBLIC,
        b"readonly" => KW_READONLY,
        b"require" => KW_REQUIRE,
        b"require_once" => KW_REQUIRE_ONCE,
        b"return" => KW_RETURN,
        b"self" => KW_SELF,
        b"static" => KW_STATIC,
        b"switch" => KW_SWITCH,
        b"throw" => KW_THROW,
        b"trait" => KW_TRAIT,
        b"true" => KW_TRUE,
        b"try" => KW_TRY,
        b"unset" => KW_UNSET,
        b"use" => KW_USE,
        b"var" => KW_VAR,
        b"while" => KW_WHILE,
        b"xor" => KW_XOR,
        b"yield" => KW_YIELD,
        _ => IDENT,
    }
}

/// Whether the cursor is just past the name of `name:` — the byte after the
/// spaces that follow is a colon that does not start a `::`.
fn named_argument_ahead(c: &Cursor) -> bool {
    let rest = c.rest();
    let mut at = 0;
    while matches!(rest.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    rest.get(at) == Some(&b':') && rest.get(at + 1) != Some(&b':')
}

/// `//` or `#` to the end of the line, or to a `?>` that closes PHP inside it:
/// PHP ends a one-line comment at the close tag, so the mode must see it.
fn line_comment(c: &mut Cursor) {
    while !c.at_end() && !matches!(c.peek(), b'\n' | b'\r') && !c.starts_with(b"?>") {
        c.bump();
    }
}

/// `/* ... */`. An unclosed one runs to the end of the file.
fn block_comment(c: &mut Cursor) {
    c.bump_n(2);
    while !c.at_end() && !c.starts_with(b"*/") {
        c.bump();
    }
    c.bump_n(2);
}

/// A single-quoted or backtick string from its quote: one token, and it may
/// span lines, because PHP lets it.
fn string(c: &mut Cursor) -> Kind {
    let quote = c.peek();
    c.bump();
    loop {
        match c.peek() {
            0 if c.at_end() => return BAD_LITERAL,
            b'\\' => c.bump_n(2),
            byte if byte == quote => {
                c.bump();
                return STRING;
            }
            _ => c.bump(),
        }
    }
}

/// A heredoc or a nowdoc from `<<<`. A nowdoc's body is not code, so the whole
/// thing is one `STRING` token; a heredoc opens the `HEREDOC` mode after the
/// line that declares its marker, and its body becomes parts.
fn heredoc(c: &mut Cursor) -> Kind {
    let rest = c.rest();
    let mut at = 3;
    while matches!(rest.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    let quote = rest.get(at).copied().unwrap_or(0);
    let quoted = matches!(quote, b'\'' | b'"');
    if quoted {
        at += 1;
    }
    let mark_start = at;
    while at < rest.len() && is_word_byte(rest[at]) {
        at += 1;
    }
    let marker = rest[mark_start..at].to_vec();
    if marker.is_empty() {
        c.bump_n(3);
        return BAD_LITERAL;
    }
    if quoted && rest.get(at) == Some(&quote) {
        at += 1;
    }
    while at < rest.len() && rest[at] != b'\n' {
        at += 1;
    }
    if at >= rest.len() {
        c.bump_n(rest.len());
        return BAD_LITERAL;
    }
    at += 1;
    if quote == b'\'' {
        return nowdoc_body(c, at, &marker);
    }
    c.bump_n(at);
    HEREDOC_MARKERS.with(|markers| markers.borrow_mut().push(marker));
    tail_push();
    HEREDOC_START
}

/// The body of a nowdoc: every line up to the marker's line, terminator and all,
/// as one token.
fn nowdoc_body(c: &mut Cursor, body: usize, marker: &[u8]) -> Kind {
    let rest = c.rest();
    let mut line = body;
    loop {
        let mut at = line;
        while matches!(rest.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        if rest.get(at..).is_some_and(|tail| tail.starts_with(marker)) && !rest.get(at + marker.len()).is_some_and(|byte| is_word_byte(*byte)) {
            c.bump_n(at + marker.len());
            return STRING;
        }
        match rest[line..].iter().position(|&byte| byte == b'\n') {
            Some(offset) => line += offset + 1,
            None => {
                c.bump_n(rest.len());
                return BAD_LITERAL;
            }
        }
    }
}

/// `(int)`, `(string)`, `(array)` and the rest, as one token.
fn cast(c: &mut Cursor) -> Option<Kind> {
    let rest = c.rest();
    let mut at = 1;
    while matches!(rest.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    let start = at;
    while rest.get(at).is_some_and(|byte| is_word_byte(*byte)) {
        at += 1;
    }
    let word = rest.get(start..at)?;
    while matches!(rest.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    if rest.get(at) != Some(&b')') {
        return None;
    }
    let known = matches!(
        word,
        b"int" | b"integer" | b"float" | b"double" | b"string" | b"binary" | b"bool" | b"boolean" | b"array" | b"object" | b"unset"
    );
    if !known {
        return None;
    }
    c.bump_n(at + 1);
    Some(CAST)
}

fn number(c: &mut Cursor) -> Kind {
    if c.peek() == b'0' && matches!(c.peek_at(1), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
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
    if float { FLOAT } else { NUMBER }
}

fn operator(c: &mut Cursor) -> Kind {
    let (a, b, d) = (c.peek(), c.peek_at(1), c.peek_at(2));
    let (kind, width) = match (a, b, d) {
        (b'<', b'<', b'=') => (SHL_EQ, 3),
        (b'>', b'>', b'=') => (SHR_EQ, 3),
        (b'*', b'*', b'=') => (STARSTAR_EQ, 3),
        (b'.', b'.', b'.') => (ELLIPSIS, 3),
        (b'=', b'=', b'=') => (IDENTICAL, 3),
        (b'!', b'=', b'=') => (NOT_IDENTICAL, 3),
        (b'<', b'=', b'>') => (SPACESHIP, 3),
        (b'?', b'-', b'>') => (NULLSAFE, 3),
        (b'?', b'?', b'=') => (NULLISH_EQ, 3),
        (b'>', b'>', _) => (SHR, 2),
        (b'<', b'<', _) => (SHL, 2),
        (b'*', b'*', _) => (STARSTAR, 2),
        (b'-', b'>', _) => (ARROW, 2),
        (b'=', b'>', _) => (FATARROW, 2),
        (b':', b':', _) => (COLONCOLON, 2),
        (b'=', b'=', _) => (EQ, 2),
        (b'!', b'=', _) => (NE, 2),
        (b'<', b'>', _) => (ANGLE_NE, 2),
        (b'<', b'=', _) => (LE, 2),
        (b'>', b'=', _) => (GE, 2),
        (b'&', b'&', _) => (ANDAND, 2),
        (b'|', b'|', _) => (OROR, 2),
        (b'?', b'?', _) => (NULLISH, 2),
        (b'+', b'+', _) => (INCREMENT, 2),
        (b'-', b'-', _) => (DECREMENT, 2),
        (b'+', b'=', _) => (PLUS_EQ, 2),
        (b'-', b'=', _) => (MINUS_EQ, 2),
        (b'*', b'=', _) => (STAR_EQ, 2),
        (b'/', b'=', _) => (SLASH_EQ, 2),
        (b'.', b'=', _) => (DOT_EQ, 2),
        (b'%', b'=', _) => (PERCENT_EQ, 2),
        (b'&', b'=', _) => (AMP_EQ, 2),
        (b'|', b'=', _) => (PIPE_EQ, 2),
        (b'^', b'=', _) => (CARET_EQ, 2),
        (b'(', ..) => match cast(c) {
            Some(kind) => (kind, 0),
            None => (LPAREN, 1),
        },
        (b')', ..) => (RPAREN, 1),
        (b'[', ..) => (LBRACKET, 1),
        (b']', ..) => (RBRACKET, 1),
        (b'{', ..) => (LBRACE, 1),
        (b'}', ..) => (RBRACE, 1),
        (b';', ..) => (SEMI, 1),
        (b',', ..) => (COMMA, 1),
        (b':', ..) => (COLON, 1),
        (b'.', ..) => (DOT, 1),
        (b'\\', ..) => (BACKSLASH, 1),
        (b'?', ..) => (QUESTION, 1),
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
mod real_world {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::PHP;
    use crate::deep::syntax::ERROR;
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    fn php_files(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                php_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "php") {
                out.push(path);
            }
        }
    }

    fn errors_of(src: &[u8]) -> Vec<(usize, usize)> {
        let mut workspace = Workspace::default();
        let Ok(outcome) = workspace.parse(src, &PHP, &Budget::default()) else { return Vec::new() };
        assert!(outcome.complete(), "the parse did not finish");
        let nodes = &workspace.tree.nodes;
        (0..nodes.len())
            .filter(|&index| nodes[index].kind == ERROR || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0)
            .map(|index| (nodes[index].start as usize, nodes[index].end as usize))
            .collect()
    }

    /// `(clean files, total files, one sample per bucket, buckets by the first
    /// word of a failing file's first error)`.
    fn census(root: &Path) -> (usize, usize, Vec<String>, Vec<(usize, String, String)>) {
        let mut files = Vec::new();
        php_files(root, &mut files);
        files.sort();
        let mut clean = 0usize;
        let mut samples = Vec::new();
        let mut histogram: BTreeMap<String, (usize, String)> = BTreeMap::new();
        for path in &files {
            let Ok(src) = fs::read(path) else { continue };
            let text = String::from_utf8_lossy(&src);
            let errors = errors_of(&src);
            if errors.is_empty() {
                clean += 1;
                continue;
            }
            let (start, end) = errors[0];
            let snippet: String = text.get(start..end).unwrap_or("").chars().take(70).collect();
            let key = snippet.split_whitespace().next().unwrap_or("<empty>").to_string();
            let entry = histogram.entry(key).or_insert((0, format!("{}: {:?}", path.display(), snippet)));
            entry.0 += 1;
            if samples.len() < 30 {
                samples.push(format!("{}: {:?}", path.display(), snippet));
            }
        }
        let mut rows: Vec<(usize, String, String)> = histogram.into_iter().map(|(key, (count, sample))| (count, key, sample)).collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        (clean, files.len(), samples, rows)
    }

    /// The real-world measure: how many files of a PHP corpus parse with no
    /// error node, and what the first error of each remaining file is.
    ///
    /// `PHP_CORPUS=<dir> cargo test -p deep-grammars -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn census_of_a_real_php_corpus() {
        let root = std::env::var("PHP_CORPUS").unwrap_or_else(|_| "/tmp/opencode/php-corpus".to_string());
        let (clean, total, _samples, rows) = census(Path::new(&root));
        println!("php corpus {root}: {clean}/{total} files parse with no error node");
        for (count, key, sample) in rows.iter().take(25) {
            println!("{count:5}  {key:28} first: {:?}", sample.replace('\n', "\\n"));
        }
        assert!(total > 0, "{root}: no PHP files");
    }

    /// One file, its error nodes and the path that leads to each, with the line
    /// each starts on: `PHP_TRACE=<file> cargo test ... --ignored --nocapture`.
    #[test]
    #[ignore]
    fn trace_one_file() {
        let Ok(path) = std::env::var("PHP_TRACE") else { return };
        let src = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&src).into_owned();
        let mut workspace = Workspace::default();
        workspace.parse(&src, &PHP, &Budget::default()).unwrap();
        let nodes = &workspace.tree.nodes;
        let mut parent = vec![0usize; nodes.len()];
        let mut open: Vec<usize> = vec![0];
        for index in 1..nodes.len() {
            while open.len() > 1 && nodes[*open.last().unwrap()].subtree_end as usize <= index {
                open.pop();
            }
            parent[index] = *open.last().unwrap();
            open.push(index);
        }
        let line_of = |at: usize| text[..at.min(text.len())].bytes().filter(|byte| *byte == b'\n').count() + 1;
        let mut shown = 0;
        for (index, node) in nodes.iter().enumerate() {
            if !(node.kind == ERROR || node.flags & HAS_ERROR != 0 && node.flags & TOKEN != 0) {
                continue;
            }
            let mut ancestors = Vec::new();
            let mut at = index;
            while at != 0 {
                at = parent[at];
                ancestors.push(PHP.name(nodes[at].kind));
            }
            ancestors.reverse();
            println!(
                "line {}: {:?} in [{}]",
                line_of(node.start as usize),
                text.get(node.start as usize..node.end as usize).unwrap_or("").chars().take(120).collect::<String>(),
                ancestors.join(" > ")
            );
            shown += 1;
            if shown == 25 {
                break;
            }
        }
        println!("{shown} error nodes in {} nodes", nodes.len());
    }

    #[test]
    fn the_census_reads_an_empty_directory_as_zero() {
        assert_eq!(census(Path::new("/nonexistent/php")).1, 0);
    }
}

#[cfg(test)]
mod fixtures {
    use std::fs;
    use std::path::PathBuf;

    use super::PHP;
    use crate::deep::syntax::ERROR;
    use crate::deep::tree::{HAS_ERROR, TOKEN};
    use crate::deep::{Budget, Workspace};

    fn read(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name);
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
    }

    /// The text of every node of `kind`, in tree order.
    fn spans<'s>(src: &'s str, kind: &str) -> Vec<&'s str> {
        let mut workspace = Workspace::default();
        workspace.parse(src.as_bytes(), &PHP, &Budget::default()).unwrap();
        let nodes = &workspace.tree.nodes;
        (0..nodes.len())
            .filter(|&index| nodes[index].flags & TOKEN == 0 && PHP.name(nodes[index].kind) == kind)
            .map(|index| &src[nodes[index].start as usize..nodes[index].end as usize])
            .collect()
    }

    /// The text of every token of `kind`, in lexer order: the leaves a rule
    /// keeps for themselves, such as a heredoc's terminator or inline HTML.
    fn tokens<'s>(src: &'s str, kind: &str) -> Vec<&'s str> {
        let mut workspace = Workspace::default();
        workspace.parse(src.as_bytes(), &PHP, &Budget::default()).unwrap();
        let nodes = &workspace.tree.nodes;
        (0..nodes.len())
            .filter(|&index| nodes[index].flags & TOKEN != 0 && PHP.name(nodes[index].kind) == kind)
            .map(|index| &src[nodes[index].start as usize..nodes[index].end as usize])
            .collect()
    }

    /// Every error node and bad-token leaf, as its text.
    fn errors(src: &str) -> Vec<String> {
        let mut workspace = Workspace::default();
        workspace.parse(src.as_bytes(), &PHP, &Budget::default()).unwrap();
        let nodes = &workspace.tree.nodes;
        (0..nodes.len())
            .filter(|&index| nodes[index].kind == ERROR || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0)
            .map(|index| src[nodes[index].start as usize..nodes[index].end as usize].to_string())
            .collect()
    }

    /// An interpolated string is a tree: the literal, its text and one
    /// `interpolation` per field, with a call inside a field parsed as a call
    /// and the escaped `$` left as text.
    #[test]
    fn interpolation_is_a_tree_not_a_token() {
        let src = read("php_interpolation.php");
        assert_eq!(errors(&src), Vec::<String>::new(), "a valid file has no error node");
        assert_eq!(
            spans(&src, "interpolation"),
            ["{$obj->m()}", "{$row['k']}", "{$price}", "${name}"],
            "one node per field, braces included"
        );
        assert_eq!(spans(&src, "function_call_expression"), ["$obj->m()"], "a call in a field is a call");
        assert_eq!(spans(&src, "member_access_expression"), ["$obj->m"], "the receiver of that call is a member");
        assert_eq!(
            spans(&src, "interpolated_variable"),
            ["$name", "$obj->label", "$row[0]"],
            "the simple forms name a variable and do not run code"
        );
        assert!(
            spans(&src, "double_quoted_string").contains(&"\"call: {$obj->m()}\""),
            "the whole literal is one string node"
        );
        assert!(!spans(&src, "string_content").iter().any(|text| text.contains("{$")), "the braces are not text");
    }

    /// A heredoc body is parts; a nowdoc is one token, because nothing in it is
    /// code. The terminator keeps its own span.
    #[test]
    fn a_heredoc_is_a_tree_and_a_nowdoc_is_one_token() {
        let src = read("php_heredoc.php");
        assert_eq!(errors(&src), Vec::<String>::new(), "a valid file has no error node");
        assert_eq!(
            spans(&src, "heredoc_string"),
            [
                "<<<SQL\nSELECT * FROM t WHERE id = {$id}\nSQL",
                "<<<EOT\nhello {$user->name()}\nEOT",
                "<<<IND\n  body {$value}\n  IND",
            ],
            "the node spans the marker line to the terminator's own last byte"
        );
        assert_eq!(
            tokens(&src, "heredoc_end"),
            ["SQL", "EOT", "  IND"],
            "the terminator is a token, indentation and marker, and the `;` stays outside it"
        );
        assert_eq!(spans(&src, "interpolation"), ["{$id}", "{$user->name()}", "{$value}"]);
        assert_eq!(spans(&src, "function_call_expression"), ["$user->name()"]);
        let nowdoc = tokens(&src, "string");
        assert_eq!(nowdoc.len(), 1, "a nowdoc is one token: {nowdoc:?}");
        assert!(nowdoc[0].starts_with("<<<'TXT'") && nowdoc[0].ends_with("TXT"), "{:?}", nowdoc[0]);
    }

    /// The constructs that made real files parse: attributes, group use, trait
    /// adaptation, promoted parameters, match, arrow functions, `?>` and
    /// inline HTML.
    #[test]
    fn the_modern_syntax_fixture_keeps_its_constructs() {
        let src = read("php_modern_syntax.php");
        assert_eq!(errors(&src), Vec::<String>::new(), "a valid file has no error node");
        assert_eq!(spans(&src, "attribute_group"), ["#[Attribute(Attribute::TARGET_CLASS)]"]);
        assert_eq!(spans(&src, "use_group"), ["{User, Post as BlogPost}"]);
        assert_eq!(spans(&src, "trait_adaptations"), ["{\n        LogsActivity::log insteadof FormatsOutput;\n        FormatsOutput::log as public formatted;\n    }"]);
        assert_eq!(spans(&src, "match_expression").len(), 1);
        assert_eq!(spans(&src, "arrow_function"), ["fn(string $value): string => trim($value)"]);
        assert_eq!(spans(&src, "anonymous_function").len(), 1);
        assert_eq!(spans(&src, "enum_declaration"), Vec::<String>::new());
        assert_eq!(tokens(&src, "inline_html"), ["\n<p class=\"note\">", "</p>\n"]);
        assert_eq!(spans(&src, "echo_statement"), ["<?= $slug(\"Hello\") ?>"]);
    }

    /// Malformed input is a tree with the error inside it, never a panic and
    /// never a hang: every prefix of every fixture, and every fixture with one
    /// byte removed, still yields the language's root.
    #[test]
    fn every_truncation_and_byte_loss_still_yields_a_tree() {
        for name in ["php_interpolation.php", "php_heredoc.php", "php_modern_syntax.php"] {
            let src = read(name);
            let bytes = src.as_bytes();
            let mut inputs: Vec<Vec<u8>> = Vec::new();
            for at in (0..bytes.len()).step_by(7) {
                inputs.push(bytes[..at].to_vec());
            }
            for at in (0..bytes.len()).step_by(11) {
                let mut cut = bytes.to_vec();
                cut.remove(at);
                inputs.push(cut);
            }
            for input in inputs {
                let mut workspace = Workspace::default();
                let outcome = workspace.parse(&input, &PHP, &Budget::default()).unwrap_or_else(|_| panic!("{name}: the parse was refused"));
                assert!(outcome.complete(), "{name}: the parse did not finish on {:?}", String::from_utf8_lossy(&input));
                assert_eq!(PHP.name(workspace.tree.root().kind), "program", "{name}: the root did not survive");
            }
        }
    }

    /// The two shapes a truncation must report rather than absorb: a string
    /// that cannot close and a field that cannot close.
    #[test]
    fn an_unclosed_string_and_an_unclosed_field_report_inside_the_root() {
        for src in ["<?php\n$a = \"never closed;\n", "<?php\n$a = \"a{$b;\n", "<?php\n$a = <<<EOT\nno terminator\n"] {
            let mut workspace = Workspace::default();
            workspace.parse(src.as_bytes(), &PHP, &Budget::default()).unwrap();
            assert_eq!(PHP.name(workspace.tree.root().kind), "program");
            let nodes = &workspace.tree.nodes;
            let errors = (0..nodes.len()).filter(|&index| nodes[index].kind == ERROR || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0).count();
            assert!(errors > 0, "no error reported for {src:?}");
        }
    }
}
