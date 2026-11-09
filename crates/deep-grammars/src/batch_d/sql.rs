//! SQL: the statements a scanner meets in a schema or a migration, and the
//! expressions they hold.
//!
//! Keywords are matched without regard to case: the lexer folds an ASCII word
//! to upper case before the keyword table, so `select` and `SELECT` are one
//! token. Quoted identifiers keep their case; `"user"` is an identifier, not
//! the keyword `USER`.
//!
//! Modelled: `WITH` (plain and recursive, several CTEs, a CTE whose body is a
//! `SELECT` or an `INSERT`/`UPDATE`/`DELETE ... RETURNING`), set operations,
//! joins with aliases and `ON`/`USING`, `CASE`, window functions (`OVER (...)`
//! with `PARTITION BY`, `ORDER BY`, named windows, `ROWS`/`RANGE`/`GROUPS`
//! frames), `FILTER (WHERE ...)`, `RETURNING`, `ON CONFLICT ... DO NOTHING` /
//! `DO UPDATE SET`, `CAST` and the `::` cast, `BETWEEN ... AND`, `IN (...)`,
//! `LIKE`/`ILIKE`, `IS [NOT] NULL`, `INTERVAL`, `EXISTS`,
//! subqueries, table and column constraints (`PRIMARY KEY`, `UNIQUE`,
//! `FOREIGN KEY ... REFERENCES ... ON DELETE CASCADE`, `CHECK`, `DEFAULT`,
//! `NOT NULL`, named `CONSTRAINT`s), `CREATE TABLE`/`VIEW`/`INDEX`/`SCHEMA`/
//! `SEQUENCE`/`TYPE ... AS ENUM`/`EXTENSION`/`FUNCTION`, `ALTER TABLE`,
//! `DROP`, `TRUNCATE`, `COMMENT ON`, `SET`, and the transaction statements.
//!
//! The lexer handles both comment styles, `''` doubling inside a string
//! literal, `$tag$ ... $tag$` strings (so a function body is one token) and
//! `$1`/`?`/`:name` placeholders.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Procedural bodies.** A `BEGIN ... END` block of a MySQL or T-SQL
//!   procedure is not a statement list here; only a dollar-quoted body is
//!   accepted, as one string token.
//! - **Types are names plus standard words.** `VARCHAR(255)`, `NUMERIC(10, 2)`,
//!   `DOUBLE PRECISION`, `TIMESTAMP WITH TIME ZONE` and `text[]` are modelled;
//!   an arbitrary dialect type is not.
//! - **`MERGE` and `UPSERT` variants other than `ON CONFLICT`** are not
//!   modelled.

use crate::batch_d::lex;
use crate::deep::lexer::Cursor;
use crate::deep::syntax::Kind;

rust_graph_deep::grammar! { SQL {
    name = "sql";
    lex = lex_sql;

    tokens {
        LPAREN = "(" open parens,
        RPAREN = ")" close parens,
        COMMA = ",",
        SEMI = ";",
        DOT = ".",
        STAR = "*",
        EQ = "=",
        NE = "!=",
        NE2 = "<>",
        LT = "<",
        GT = ">",
        LE = "<=",
        GE = ">=",
        PLUS = "+",
        MINUS = "-",
        SLASH = "/",
        PERCENT = "%",
        CONCAT = "||",
        COLON2 = "::",
        ARROW = "->",
        ARROW2 = "->>",
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,
        /// `$1`, `?` or `:name`.
        PLACEHOLDER = "placeholder",
        /// A quoted identifier: `"users"`, `` `users` ``, `[users]`.
        QUOTED_IDENT = "quoted_identifier",
        /// `[]` after a type name.
        ARRAY_SUFFIX = "[]",

        KW_SELECT = "SELECT",
        KW_DISTINCT = "DISTINCT",
        KW_ALL = "ALL",
        KW_FROM = "FROM",
        KW_WHERE = "WHERE",
        KW_GROUP = "GROUP",
        KW_BY = "BY",
        KW_HAVING = "HAVING",
        KW_ORDER = "ORDER",
        KW_ASC = "ASC",
        KW_DESC = "DESC",
        KW_LIMIT = "LIMIT",
        KW_OFFSET = "OFFSET",
        KW_AS = "AS",
        KW_JOIN = "JOIN",
        KW_INNER = "INNER",
        KW_LEFT = "LEFT",
        KW_RIGHT = "RIGHT",
        KW_FULL = "FULL",
        KW_OUTER = "OUTER",
        KW_CROSS = "CROSS",
        KW_NATURAL = "NATURAL",
        KW_LATERAL = "LATERAL",
        KW_ON = "ON",
        KW_USING = "USING",
        KW_UNION = "UNION",
        KW_INTERSECT = "INTERSECT",
        KW_EXCEPT = "EXCEPT",
        KW_INSERT = "INSERT",
        KW_INTO = "INTO",
        KW_VALUES = "VALUES",
        KW_UPDATE = "UPDATE",
        KW_SET = "SET",
        KW_DELETE = "DELETE",
        KW_CREATE = "CREATE",
        KW_TABLE = "TABLE",
        KW_VIEW = "VIEW",
        KW_INDEX = "INDEX",
        KW_ALTER = "ALTER",
        KW_DROP = "DROP",
        KW_ADD = "ADD",
        KW_COLUMN = "COLUMN",
        KW_RENAME = "RENAME",
        KW_TO = "TO",
        KW_CONCURRENTLY = "CONCURRENTLY",
        KW_PRIMARY = "PRIMARY",
        KW_KEY = "KEY",
        KW_FOREIGN = "FOREIGN",
        KW_REFERENCES = "REFERENCES",
        KW_UNIQUE = "UNIQUE",
        KW_NOT = "NOT",
        KW_NULL = "NULL",
        KW_DEFAULT = "DEFAULT",
        KW_CHECK = "CHECK",
        KW_CONSTRAINT = "CONSTRAINT",
        KW_AND = "AND",
        KW_OR = "OR",
        KW_IN = "IN",
        KW_IS = "IS",
        KW_LIKE = "LIKE",
        KW_ILIKE = "ILIKE",
        KW_BETWEEN = "BETWEEN",
        KW_EXISTS = "EXISTS",
        KW_CASE = "CASE",
        KW_WHEN = "WHEN",
        KW_THEN = "THEN",
        KW_ELSE = "ELSE",
        KW_END = "END",
        KW_CAST = "CAST",
        KW_INTERVAL = "INTERVAL",
        KW_WITH = "WITH",
        KW_RECURSIVE = "RECURSIVE",
        KW_IF = "IF",
        KW_CASCADE = "CASCADE",
        KW_RESTRICT = "RESTRICT",
        KW_TEXT = "TEXT",
        KW_TIME = "TIME",
        KW_ZONE = "ZONE",
        KW_RETURNING = "RETURNING",
        KW_CONFLICT = "CONFLICT",
        KW_DO = "DO",
        KW_NOTHING = "NOTHING",
        KW_OVER = "OVER",
        KW_PARTITION = "PARTITION",
        KW_WINDOW = "WINDOW",
        KW_FILTER = "FILTER",
        KW_ROWS = "ROWS",
        KW_RANGE = "RANGE",
        KW_GROUPS = "GROUPS",
        KW_PRECEDING = "PRECEDING",
        KW_FOLLOWING = "FOLLOWING",
        KW_UNBOUNDED = "UNBOUNDED",
        KW_CURRENT = "CURRENT",
        KW_ROW = "ROW",
        KW_NULLS = "NULLS",
        KW_FIRST = "FIRST",
        KW_LAST = "LAST",
        KW_TRUE = "TRUE",
        KW_FALSE = "FALSE",
        KW_ESCAPE = "ESCAPE",
        KW_ANY = "ANY",
        KW_SOME = "SOME",
        KW_EXTRACT = "EXTRACT",
        KW_ENUM = "ENUM",
        KW_SCHEMA = "SCHEMA",
        KW_SEQUENCE = "SEQUENCE",
        KW_EXTENSION = "EXTENSION",
        KW_TYPE = "TYPE",
        KW_FUNCTION = "FUNCTION",
        KW_PROCEDURE = "PROCEDURE",
        KW_RETURNS = "RETURNS",
        KW_LANGUAGE = "LANGUAGE",
        KW_REPLACE = "REPLACE",
        /// `OR REPLACE` folded into one token, so `CREATE OR REPLACE VIEW` and
        /// `CREATE OR REPLACE FUNCTION` are told apart by their next word.
        OR_REPLACE = "OR REPLACE",
        KW_TRUNCATE = "TRUNCATE",
        KW_COMMENT = "COMMENT",
        KW_BEGIN = "BEGIN",
        KW_COMMIT = "COMMIT",
        KW_ROLLBACK = "ROLLBACK",
        KW_START = "START",
        KW_TRANSACTION = "TRANSACTION",
        KW_PRECISION = "PRECISION",
        KW_VARYING = "VARYING",
        KW_UNSIGNED = "UNSIGNED",
        KW_SIGNED = "SIGNED",
        KW_AUTOINCREMENT = "AUTOINCREMENT",
        KW_COLLATE = "COLLATE",
        KW_STORED = "STORED",
        KW_GENERATED = "GENERATED",
        KW_ALWAYS = "ALWAYS",
        KW_IDENTITY = "IDENTITY",
        KW_NO = "NO",
        KW_DEFINER = "DEFINER",
        KW_INVOKER = "INVOKER",
        KW_IMMUTABLE = "IMMUTABLE",
        KW_STABLE = "STABLE",
        KW_VOLATILE = "VOLATILE",
        KW_STRICT = "STRICT",
        KW_SECURITY = "SECURITY",
        KW_COST = "COST",
        KW_LOGGED = "LOGGED",
        KW_UNLOGGED = "UNLOGGED",
        KW_TEMP = "TEMP",
        KW_TEMPORARY = "TEMPORARY",
        KW_SESSION = "SESSION",
        KW_LOCAL = "LOCAL",
        KW_OWNED = "OWNED",
        KW_CACHE = "CACHE",
        KW_CYCLE = "CYCLE",
        KW_INCREMENT = "INCREMENT",
        KW_ZEROFILL = "ZEROFILL",
        KW_ACTION = "ACTION",
        KW_STATISTICS = "STATISTICS",
        KW_TABLESPACE = "TABLESPACE",
        KW_AUTHORIZATION = "AUTHORIZATION",
        KW_MATERIALIZED = "MATERIALIZED",
    }

    program: _statement*,

    _statement:
        (
            with_statement
            | select_statement
            | insert_statement
            | update_statement
            | delete_statement
            | create_table_statement
            | create_view_statement
            | create_index_statement
            | create_schema_statement
            | create_sequence_statement
            | create_type_statement
            | create_extension_statement
            | create_function_statement
            | alter_statement
            | drop_statement
            | truncate_statement
            | comment_statement
            | set_statement
            | transaction_statement
        )
        [";"],

    // ---- statements ----

    with_statement: "WITH" ["RECURSIVE"] (common_table_expression % ",") _with_body,
    _with_body: select_statement | insert_statement | update_statement | delete_statement,
    common_table_expression: _entity [column_list] "AS" cte_body,
    cte_body: "(" (select_statement | insert_statement | update_statement | delete_statement) ")",

    select_statement:
        "SELECT"
        ["DISTINCT" ["ON" column_list] | "ALL"]
        (select_item % ",")
        [from_clause]
        [where_clause]
        [group_clause]
        [having_clause]
        [window_clause]
        [set_operator select_statement]
        [order_clause]
        [limit_clause],
    select_item: "*" | qualified_star | _expression ["AS"] [alias],
    qualified_star: _entity "." "*",
    alias: _entity,
    set_operator: "UNION" ["ALL"] | "INTERSECT" | "EXCEPT",

    from_clause: "FROM" (table_reference % ","),
    table_reference: table_name [["AS"] alias] (join_clause)*,
    table_name: ["LATERAL"] (_entity [call_arguments] | paren_expression) ,
    join_clause:
        ["NATURAL"] [("INNER" | "LEFT" | "RIGHT" | "FULL") ["OUTER"] | "CROSS"] "JOIN" table_name [["AS"] alias] [join_condition],
    join_condition: "ON" _expression | "USING" column_list,

    where_clause: "WHERE" _expression,
    group_clause: "GROUP" "BY" (_expression % ","),
    having_clause: "HAVING" _expression,
    order_clause: "ORDER" "BY" (order_item % ","),
    order_item: _expression ["ASC" | "DESC"] [nulls_order],
    nulls_order: "NULLS" ("FIRST" | "LAST"),
    limit_clause: "LIMIT" (_expression | "ALL") ["OFFSET" _expression],

    window_clause: "WINDOW" (window_definition % ","),
    window_definition: _entity "AS" window_specification,
    window_specification: "(" [window_name] [window_partition] [window_order] [frame_clause] ")",
    window_name: _entity,
    window_partition: "PARTITION" "BY" (_expression % ","),
    window_order: "ORDER" "BY" (order_item % ","),
    frame_clause: frame_unit ("BETWEEN" frame_bound "AND" frame_bound | frame_bound),
    frame_unit: "ROWS" | "RANGE" | "GROUPS",
    frame_bound: "UNBOUNDED" ("PRECEDING" | "FOLLOWING") | "CURRENT" "ROW" | _expression@addition ("PRECEDING" | "FOLLOWING"),

    insert_statement:
        "INSERT" "INTO" table_name [column_list] (_insert_body)
        [on_conflict_clause] [returning_clause],
    _insert_body: "VALUES" (value_row % ",") | "DEFAULT" "VALUES" | select_statement,
    value_row: "(" (_expression % ",") ")",
    on_conflict_clause:
        "ON" "CONFLICT" [conflict_target] ["WHERE" _expression] "DO" conflict_action,
    conflict_target: "(" (_expression % ",") ")",
    conflict_action: "NOTHING" | "UPDATE" "SET" (assignment % ",") ["WHERE" _expression],

    update_statement: "UPDATE" table_name "SET" (assignment % ",") [from_clause] [where_clause] [returning_clause],
    assignment: _column "=" _expression,
    delete_statement: "DELETE" "FROM" table_name [using_clause] [where_clause] [returning_clause],
    using_clause: "USING" (table_reference % ","),
    returning_clause: "RETURNING" (returning_item % ","),
    returning_item: "*" | qualified_star | _expression ["AS"] [alias],

    create_table_statement:
        "CREATE" ["TEMP" | "TEMPORARY"] "TABLE" ["IF" "NOT" "EXISTS"] _entity [table_body | "AS" select_statement],
    table_body: "(" (table_item % ",") ")",
    table_item: column_definition | table_constraint,
    table_constraint:
        ["CONSTRAINT" _entity]
        (
            "PRIMARY" "KEY" column_list
            | "UNIQUE" column_list
            | "FOREIGN" "KEY" column_list "REFERENCES" _entity [column_list] (referential_action)*
            | "CHECK" check_body
        ),
    column_definition: _column data_type (column_constraint)*,
    column_constraint:
        "PRIMARY" "KEY" ["AUTOINCREMENT"]
        | "NOT" "NULL"
        | "NULL"
        | "UNIQUE"
        | "REFERENCES" _entity [column_list] (referential_action)*
        | "DEFAULT" _expression
        | "CHECK" check_body
        | "COLLATE" _entity
        | "GENERATED" ["ALWAYS"] "AS" "IDENTITY"
        | "CONSTRAINT" _entity,
    referential_action:
        "ON" ("DELETE" | "UPDATE") ("CASCADE" | "RESTRICT" | "NO" "ACTION" | "SET" "NULL" | "SET" "DEFAULT"),
    check_body: "(" _expression ")",

    data_type: _type_name [type_arguments] (_type_word)* [array_suffixes],
    _type_name: IDENT | QUOTED_IDENT | "TEXT",
    _type_word: "WITH" "TIME" "ZONE" | "PRECISION" | "VARYING" | "UNSIGNED" | "SIGNED" | "ZEROFILL",
    type_arguments: "(" (NUMBER % ",") ")",
    array_suffixes: ARRAY_SUFFIX [array_suffixes],

    create_view_statement:
        "CREATE" [OR_REPLACE] ["TEMP" | "TEMPORARY"] "VIEW" ["IF" "NOT" "EXISTS"] _entity [column_list] "AS" select_statement,
    create_index_statement:
        "CREATE" ["UNIQUE"] "INDEX" ["CONCURRENTLY"] ["IF" "NOT" "EXISTS"] [_entity] "ON" _entity ["USING" _entity]
        index_column_list [where_clause],
    index_column_list: "(" (index_item % ",") ")",
    index_item: _expression ["ASC" | "DESC"],

    create_schema_statement: "CREATE" "SCHEMA" ["IF" "NOT" "EXISTS"] _entity,
    create_sequence_statement: "CREATE" "SEQUENCE" ["IF" "NOT" "EXISTS"] _entity (sequence_option)*,
    sequence_option: ("AS" data_type | "START" ["WITH"] NUMBER | "INCREMENT" ["BY"] NUMBER | "OWNED" "BY" _entity | "CACHE" NUMBER | "CYCLE" | "NO" "CYCLE"),
    create_type_statement: "CREATE" "TYPE" _entity "AS" "ENUM" enum_values,
    enum_values: "(" (STRING % ",") ")",
    create_extension_statement: "CREATE" "EXTENSION" ["IF" "NOT" "EXISTS"] _entity ["WITH" "SCHEMA" _entity],
    create_function_statement:
        "CREATE" [OR_REPLACE] ("FUNCTION" | "PROCEDURE") _entity [call_arguments] (function_option)*,
    function_option:
        "RETURNS" ("TABLE" table_body | data_type)
        | "LANGUAGE" _entity
        | "AS" STRING
        | "IMMUTABLE"
        | "STABLE"
        | "VOLATILE"
        | "STRICT"
        | "SECURITY" ("DEFINER" | "INVOKER")
        | "COST" NUMBER
        | "ROWS" NUMBER
        | "SET" _entity "=" _expression,

    alter_statement: "ALTER" ("TABLE" | "INDEX" | "VIEW") _entity (alter_action % ","),
    alter_action:
        add_column_action
        | add_constraint_action
        | drop_column_action
        | rename_column_action
        | rename_object_action
        | alter_column_action
        | set_action,
    add_column_action: "ADD" ["COLUMN"] ["IF" "NOT" "EXISTS"] column_definition,
    add_constraint_action: "ADD" table_constraint,
    drop_column_action: "DROP" ["COLUMN"] ["IF" "EXISTS"] _column ["CASCADE" | "RESTRICT"],
    rename_column_action: "RENAME" ["COLUMN"] _column "TO" _column,
    rename_object_action: "RENAME" "TO" _entity,
    alter_column_action:
        "ALTER" ["COLUMN"] _column
        ("SET" ("DEFAULT" _expression | "NOT" "NULL" | "TYPE" data_type | "STATISTICS" NUMBER)
            | "DROP" ("DEFAULT" | "NOT" "NULL")),
    set_action: "SET" ("SCHEMA" _entity | "TABLESPACE" _entity | "LOGGED" | "UNLOGGED"),

    drop_statement:
        "DROP"
        ("TABLE" | "VIEW" | "INDEX" | "SCHEMA" | "SEQUENCE" | "TYPE" | "FUNCTION")
        ["IF" "EXISTS"] _entity ["CASCADE" | "RESTRICT"],
    truncate_statement: "TRUNCATE" [ "TABLE"] _entity,
    comment_statement: "COMMENT" "ON" ("TABLE" | "VIEW" | "COLUMN" | "INDEX" | "SCHEMA" | "TYPE") _entity ["." _column] "IS" STRING,
    set_statement: "SET" ("TIME" "ZONE" _expression | assignment | ("SESSION" | "LOCAL") assignment),
    transaction_statement:
        "BEGIN" ["TRANSACTION"]
        | "COMMIT" ["TRANSACTION"]
        | "ROLLBACK" ["TRANSACTION"]
        | "START" "TRANSACTION",

    // ---- expressions ----

    _expression: precedence _primary {
        or: left "OR" => logical_expression,
        and: left "AND" => logical_expression,
        comparison: left "=" | "!=" | "<>" | "<" | ">" | "<=" | ">=" => comparison,
        is_null: postfix "IS" ["NOT"] ["NULL"] => is_null,
        pattern: left "LIKE" | "ILIKE" => comparison,
        negated_pattern: left "NOT" "LIKE" | "NOT" "ILIKE" => comparison,
        membership: left "IN" => comparison,
        negated_membership: left "NOT" "IN" => comparison,
        between: left "BETWEEN" => between_expression,
        negated_between: left "NOT" "BETWEEN" => between_expression,
        concatenation: left "||" => concatenation,
        addition: left "+" | "-" => arithmetic,
        multiplication: left "*" | "/" | "%" => arithmetic,
        negation: prefix "-" | "+" | "NOT" => unary,
        json_access: left "->" | "->>" => member,
        cast: postfix "::" data_type => cast_expression,
        filter: postfix filter_clause => filter_expression,
        window: postfix over_clause => window_expression,
        call: postfix call_arguments => call,
        member: postfix member_expression => member,
    },

    filter_clause: "FILTER" filter_body,
    filter_body: "(" "WHERE" _expression ")",
    over_clause: "OVER" (window_specification | window_name),

    member_expression: "." _column,

    _primary:
        literal
        | _entity
        | paren_expression
        | case_expression
        | cast_expression_call
        | "EXISTS" paren_expression
        | interval_expression
        | any_expression
        | extract_expression,

    literal: NUMBER | STRING | "NULL" | "TRUE" | "FALSE" | PLACEHOLDER,
    paren_expression: "(" (select_statement | _expression ("," _expression)*) ")",
    column_list: "(" (_column % ",") ")",
    case_expression: "CASE" [ _expression] (case_when)+ ["ELSE" _expression] "END",
    case_when: "WHEN" _expression "THEN" _expression,
    cast_expression_call: "CAST" cast_body,
    cast_body: "(" _expression "AS" data_type ")",
    interval_expression: "INTERVAL" (STRING | NUMBER) _interval_unit,
    _interval_unit: IDENT,
    any_expression: ("ANY" | "SOME" | "ALL") paren_expression,
    extract_expression: "EXTRACT" extract_body,
    extract_body: "(" _interval_unit "FROM" _expression ")",

    call_arguments: "(" ["DISTINCT"] [(_call_item) ("," _call_item)*] ")",
    _call_item: "*" | _expression,

    _entity: IDENT | QUOTED_IDENT | soft_keyword,
    _column: IDENT | QUOTED_IDENT | soft_keyword,
    /// A keyword that is also a common column name (`type`, `language`,
    /// `key`, ...). It is a keyword only where a statement or a clause starts
    /// with it, and an identifier everywhere a name can stand.
    soft_keyword:
        "TYPE" | "LANGUAGE" | "TEXT" | "TIME" | "COMMENT" | "KEY" | "ROW" | "ROWS" | "RANGE" | "GROUPS"
        | "FIRST" | "LAST" | "ACTION" | "CACHE" | "CYCLE" | "INCREMENT" | "OWNED" | "STORED"
        | "GENERATED" | "ALWAYS" | "IDENTITY" | "SECURITY" | "STRICT" | "STABLE" | "VOLATILE"
        | "IMMUTABLE" | "MATERIALIZED" | "AUTHORIZATION" | "STATISTICS" | "TABLESPACE"
        | "ZEROFILL" | "LOCAL" | "SESSION" | "LOGGED" | "UNLOGGED" | "VARYING" | "PRECISION"
        | "UNSIGNED" | "SIGNED" | "ENUM" | "EXTENSION" | "SEQUENCE" | "SCHEMA" | "VIEW" | "INDEX"
        | "TEMP" | "TEMPORARY",
} }

pub fn lex_sql(c: &mut Cursor) -> Kind {
    use crate::deep::syntax::{COMMENT, UNKNOWN, WHITESPACE};
    let byte = c.peek();
    match byte {
        b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => {
            c.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c'));
            WHITESPACE
        }
        b'-' if c.peek_at(1) == b'-' => {
            lex::line_comment(c);
            COMMENT
        }
        b'/' if c.peek_at(1) == b'*' => {
            lex::block_comment(c, b"/*", b"*/", true);
            COMMENT
        }
        b'\'' => {
            if sql_string(c) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'"' | b'`' => {
            c.bump();
            c.quoted(byte, true);
            QUOTED_IDENT
        }
        b'[' if c.peek_at(1) == b']' => {
            c.bump_n(2);
            ARRAY_SUFFIX
        }
        b'[' => {
            c.bump();
            c.eat_while(|byte| byte != b']' && byte != b'\n');
            c.eat(b']');
            QUOTED_IDENT
        }
        b'$' if sql_dollar_tag(c) => {
            if sql_dollar_string(c) {
                STRING
            } else {
                BAD_STRING
            }
        }
        b'$' if c.peek_at(1).is_ascii_digit() => {
            c.bump();
            c.eat_while(|byte| byte.is_ascii_digit());
            PLACEHOLDER
        }
        b'?' => {
            c.bump();
            PLACEHOLDER
        }
        b':' if c.peek_at(1).is_ascii_alphabetic() || c.peek_at(1) == b'_' => {
            c.bump();
            c.eat_while(lex::is_word_byte);
            PLACEHOLDER
        }
        b':' if c.peek_at(1) == b':' => {
            c.bump_n(2);
            COLON2
        }
        b'0'..=b'9' => {
            lex::number(c);
            NUMBER
        }
        b'(' => {
            c.bump();
            LPAREN
        }
        b')' => {
            c.bump();
            RPAREN
        }
        b',' => {
            c.bump();
            COMMA
        }
        b';' => {
            c.bump();
            SEMI
        }
        b'.' => {
            c.bump();
            DOT
        }
        b'*' => {
            c.bump();
            STAR
        }
        b'=' => {
            c.bump();
            EQ
        }
        b'!' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            NE
        }
        b'<' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            LE
        }
        b'<' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            NE2
        }
        b'<' => {
            c.bump();
            LT
        }
        b'>' if c.peek_at(1) == b'=' => {
            c.bump_n(2);
            GE
        }
        b'>' => {
            c.bump();
            GT
        }
        b'+' => {
            c.bump();
            PLUS
        }
        b'-' if c.peek_at(1) == b'>' && c.peek_at(2) == b'>' => {
            c.bump_n(3);
            ARROW2
        }
        b'-' if c.peek_at(1) == b'>' => {
            c.bump_n(2);
            ARROW
        }
        b'-' => {
            c.bump();
            MINUS
        }
        b'/' => {
            c.bump();
            SLASH
        }
        b'%' => {
            c.bump();
            PERCENT
        }
        b'|' if c.peek_at(1) == b'|' => {
            c.bump_n(2);
            CONCAT
        }
        b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
            let start = c.rest();
            let length = c.eat_while(lex::is_word_byte);
            let upper = lex::upper(&start[..length]);
            // `OR REPLACE` is one token: two boolean words in a row are not a
            // construct, and folding them keeps `CREATE OR REPLACE VIEW`
            // distinguishable from `CREATE OR REPLACE FUNCTION` earlier.
            if upper == b"OR" {
                let rest = c.rest();
                let mut index = 0;
                while index < rest.len() && matches!(rest[index], b' ' | b'\t' | b'\r' | b'\n') {
                    index += 1;
                }
                let word = &rest[index..];
                if word.len() >= 7 && word[..7].eq_ignore_ascii_case(b"REPLACE") && !word.get(7).is_some_and(|byte| lex::is_word_byte(*byte)) {
                    c.bump_n(index + 7);
                    return OR_REPLACE;
                }
            }
            keyword(&upper)
        }
        _ => UNKNOWN,
    }
}

/// A `'...'` literal with `''` as the escape for one quote, and a backslash
/// escape where the dialect uses one.
fn sql_string(c: &mut Cursor) -> bool {
    c.bump();
    while !c.at_end() {
        match c.peek() {
            b'\\' => c.bump_n(2),
            b'\'' if c.peek_at(1) == b'\'' => c.bump_n(2),
            b'\'' => {
                c.bump();
                return true;
            }
            _ => c.bump_char(),
        }
    }
    false
}

/// Whether a `$tag$` opener is at the cursor: `$$` or `$name$`.
fn sql_dollar_tag(c: &Cursor) -> bool {
    let rest = c.rest();
    if rest.len() < 2 || rest[0] != b'$' {
        return false;
    }
    let mut index = 1;
    while index < rest.len() && (rest[index].is_ascii_alphanumeric() || rest[index] == b'_') {
        index += 1;
    }
    rest.get(index) == Some(&b'$')
}

/// A `$tag$ ... $tag$` string: the body may hold anything, including `'` and
/// newlines, up to the matching tag.
fn sql_dollar_string(c: &mut Cursor) -> bool {
    let rest = c.rest();
    let mut tag_end = 1;
    while tag_end < rest.len() && rest[tag_end] != b'$' {
        tag_end += 1;
    }
    if tag_end >= rest.len() {
        return false;
    }
    let tag = rest[..=tag_end].to_vec();
    c.bump_n(tag.len());
    while !c.at_end() {
        if c.starts_with(&tag) {
            c.bump_n(tag.len());
            return true;
        }
        c.bump_char();
    }
    false
}

fn keyword(upper: &[u8]) -> Kind {
    match upper {
        b"SELECT" => KW_SELECT, b"DISTINCT" => KW_DISTINCT, b"ALL" => KW_ALL,
        b"FROM" => KW_FROM, b"WHERE" => KW_WHERE, b"GROUP" => KW_GROUP,
        b"BY" => KW_BY, b"HAVING" => KW_HAVING, b"ORDER" => KW_ORDER,
        b"ASC" => KW_ASC, b"DESC" => KW_DESC, b"LIMIT" => KW_LIMIT,
        b"OFFSET" => KW_OFFSET, b"AS" => KW_AS, b"JOIN" => KW_JOIN,
        b"INNER" => KW_INNER, b"LEFT" => KW_LEFT, b"RIGHT" => KW_RIGHT,
        b"FULL" => KW_FULL, b"OUTER" => KW_OUTER, b"CROSS" => KW_CROSS,
        b"NATURAL" => KW_NATURAL, b"LATERAL" => KW_LATERAL,
        b"ON" => KW_ON, b"USING" => KW_USING, b"UNION" => KW_UNION,
        b"INTERSECT" => KW_INTERSECT, b"EXCEPT" => KW_EXCEPT, b"INSERT" => KW_INSERT,
        b"INTO" => KW_INTO, b"VALUES" => KW_VALUES, b"UPDATE" => KW_UPDATE,
        b"SET" => KW_SET, b"DELETE" => KW_DELETE, b"CREATE" => KW_CREATE,
        b"TABLE" => KW_TABLE, b"VIEW" => KW_VIEW, b"INDEX" => KW_INDEX,
        b"ALTER" => KW_ALTER, b"DROP" => KW_DROP, b"ADD" => KW_ADD,
        b"COLUMN" => KW_COLUMN, b"RENAME" => KW_RENAME, b"TO" => KW_TO,
        b"CONCURRENTLY" => KW_CONCURRENTLY,
        b"PRIMARY" => KW_PRIMARY, b"KEY" => KW_KEY, b"FOREIGN" => KW_FOREIGN,
        b"REFERENCES" => KW_REFERENCES, b"UNIQUE" => KW_UNIQUE, b"NOT" => KW_NOT,
        b"NULL" => KW_NULL, b"DEFAULT" => KW_DEFAULT, b"CHECK" => KW_CHECK,
        b"CONSTRAINT" => KW_CONSTRAINT, b"AND" => KW_AND, b"OR" => KW_OR,
        b"IN" => KW_IN, b"IS" => KW_IS, b"LIKE" => KW_LIKE, b"ILIKE" => KW_ILIKE,
        b"BETWEEN" => KW_BETWEEN, b"EXISTS" => KW_EXISTS, b"CASE" => KW_CASE,
        b"WHEN" => KW_WHEN, b"THEN" => KW_THEN, b"ELSE" => KW_ELSE,
        b"END" => KW_END, b"CAST" => KW_CAST, b"INTERVAL" => KW_INTERVAL,
        b"WITH" => KW_WITH, b"RECURSIVE" => KW_RECURSIVE, b"IF" => KW_IF,
        b"CASCADE" => KW_CASCADE, b"RESTRICT" => KW_RESTRICT,
        b"TEXT" => KW_TEXT, b"TIME" => KW_TIME, b"ZONE" => KW_ZONE,
        b"RETURNING" => KW_RETURNING, b"CONFLICT" => KW_CONFLICT, b"DO" => KW_DO,
        b"NOTHING" => KW_NOTHING, b"OVER" => KW_OVER, b"PARTITION" => KW_PARTITION,
        b"WINDOW" => KW_WINDOW, b"FILTER" => KW_FILTER, b"ROWS" => KW_ROWS,
        b"RANGE" => KW_RANGE, b"GROUPS" => KW_GROUPS,
        b"PRECEDING" => KW_PRECEDING, b"FOLLOWING" => KW_FOLLOWING,
        b"UNBOUNDED" => KW_UNBOUNDED, b"CURRENT" => KW_CURRENT, b"ROW" => KW_ROW,
        b"NULLS" => KW_NULLS, b"FIRST" => KW_FIRST, b"LAST" => KW_LAST,
        b"TRUE" => KW_TRUE, b"FALSE" => KW_FALSE, b"ESCAPE" => KW_ESCAPE,
        b"ANY" => KW_ANY, b"SOME" => KW_SOME, b"EXTRACT" => KW_EXTRACT,
        b"ENUM" => KW_ENUM, b"SCHEMA" => KW_SCHEMA, b"SEQUENCE" => KW_SEQUENCE,
        b"EXTENSION" => KW_EXTENSION, b"TYPE" => KW_TYPE,
        b"FUNCTION" => KW_FUNCTION, b"PROCEDURE" => KW_PROCEDURE,
        b"RETURNS" => KW_RETURNS, b"LANGUAGE" => KW_LANGUAGE,
        b"REPLACE" => KW_REPLACE, b"TRUNCATE" => KW_TRUNCATE,
        b"COMMENT" => KW_COMMENT, b"BEGIN" => KW_BEGIN, b"COMMIT" => KW_COMMIT,
        b"ROLLBACK" => KW_ROLLBACK, b"START" => KW_START,
        b"TRANSACTION" => KW_TRANSACTION, b"PRECISION" => KW_PRECISION,
        b"VARYING" => KW_VARYING, b"UNSIGNED" => KW_UNSIGNED,
        b"SIGNED" => KW_SIGNED, b"AUTOINCREMENT" => KW_AUTOINCREMENT,
        b"COLLATE" => KW_COLLATE, b"STORED" => KW_STORED,
        b"GENERATED" => KW_GENERATED, b"ALWAYS" => KW_ALWAYS,
        b"IDENTITY" => KW_IDENTITY, b"NO" => KW_NO, b"DEFINER" => KW_DEFINER,
        b"INVOKER" => KW_INVOKER, b"IMMUTABLE" => KW_IMMUTABLE, b"STABLE" => KW_STABLE,
        b"VOLATILE" => KW_VOLATILE, b"STRICT" => KW_STRICT, b"SECURITY" => KW_SECURITY,
        b"COST" => KW_COST, b"LOGGED" => KW_LOGGED, b"UNLOGGED" => KW_UNLOGGED,
        b"TEMP" => KW_TEMP, b"TEMPORARY" => KW_TEMPORARY, b"SESSION" => KW_SESSION,
        b"LOCAL" => KW_LOCAL, b"OWNED" => KW_OWNED, b"CACHE" => KW_CACHE,
        b"CYCLE" => KW_CYCLE, b"INCREMENT" => KW_INCREMENT, b"ZEROFILL" => KW_ZEROFILL, b"ACTION" => KW_ACTION, b"STATISTICS" => KW_STATISTICS,
        b"TABLESPACE" => KW_TABLESPACE,
        b"AUTHORIZATION" => KW_AUTHORIZATION, b"MATERIALIZED" => KW_MATERIALIZED,
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::deep::tree::{Node, TOKEN};
    use crate::deep::{Budget, Workspace};

    struct Parsed {
        src: Vec<u8>,
        workspace: Workspace,
    }

    impl Parsed {
        fn fixture(name: &str) -> Parsed {
            let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/longtail/sql").join(name);
            let src = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            let mut workspace = Workspace::default();
            workspace.parse(&src, &super::SQL, &Budget::default()).unwrap();
            Parsed { src, workspace }
        }

        fn node(&self, index: usize) -> Node {
            self.workspace.tree.nodes[index]
        }

        fn kind(&self, index: usize) -> &'static str {
            super::SQL.name(self.node(index).kind)
        }

        fn is_token(&self, index: usize) -> bool {
            self.node(index).flags & TOKEN != 0
        }

        fn text(&self, index: usize) -> &str {
            let node = self.node(index);
            std::str::from_utf8(&self.src[node.start as usize..node.end as usize]).expect("utf8 fixture")
        }

        fn of_kind(&self, kind: &str) -> Vec<usize> {
            (1..self.workspace.tree.nodes.len())
                .filter(|&index| !self.is_token(index) && self.kind(index) == kind)
                .collect()
        }

        fn leaves_of_kind(&self, kind: &str) -> Vec<usize> {
            (1..self.workspace.tree.nodes.len())
                .filter(|&index| self.is_token(index) && self.kind(index) == kind)
                .collect()
        }

        fn errors(&self) -> usize {
            use crate::deep::syntax::ERROR;
            use crate::deep::tree::HAS_ERROR;
            (0..self.workspace.tree.nodes.len())
                .filter(|&index| self.node(index).kind == ERROR || (self.is_token(index) && self.node(index).flags & HAS_ERROR != 0))
                .count()
        }
    }

    #[test]
    fn a_cte_query_keeps_its_ctes_windows_and_between_nodes() {
        let parsed = Parsed::fixture("sql_cte_window.sql");
        assert_eq!(parsed.errors(), 0, "the fixture must parse cleanly");
        assert_eq!(parsed.of_kind("common_table_expression").len(), 2, "two CTEs, `recent` and `ranked`");
        let windows = parsed.of_kind("window_expression");
        assert_eq!(windows.len(), 3, "RANK, SUM and AVG each carry an OVER clause");
        let first = &parsed.workspace.tree.nodes[windows[0]];
        let text = &parsed.src[first.start as usize..first.end as usize];
        assert!(text.starts_with(b"RANK() OVER (PARTITION BY"), "{:?}", String::from_utf8_lossy(text));
        let between = parsed.of_kind("between_expression");
        assert_eq!(between.len(), 1, "`r.retired BETWEEN 1 AND 3` is one between_expression");
        let span = &parsed.workspace.tree.nodes[between[0]];
        assert_eq!(&parsed.src[span.start as usize..span.end as usize], b"r.retired BETWEEN 1");
    }

    #[test]
    fn a_block_comment_and_a_doubled_quote_literal_are_not_code() {
        let parsed = Parsed::fixture("sql_cte_window.sql");
        let strings: Vec<&str> = parsed.leaves_of_kind("string").iter().map(|&index| parsed.text(index)).collect();
        assert!(strings.contains(&"'it''s quoted'"), "the doubled quote stays inside one string: {strings:?}");
        assert_eq!(parsed.errors(), 0, "the quote inside the block comment is not a string");
    }

    #[test]
    fn on_conflict_and_returning_are_nodes_with_spans() {
        let parsed = Parsed::fixture("sql_upsert.sql");
        assert_eq!(parsed.errors(), 0);
        let clauses = parsed.of_kind("on_conflict_clause");
        assert_eq!(clauses.len(), 1);
        let node = parsed.workspace.tree.nodes[clauses[0]];
        let text = &parsed.src[node.start as usize..node.end as usize];
        assert!(text.starts_with(b"ON CONFLICT (login) DO UPDATE"), "{:?}", String::from_utf8_lossy(text));
        assert!(text.ends_with(b"EXCLUDED.email"), "{:?}", String::from_utf8_lossy(text));
        assert_eq!(parsed.of_kind("returning_clause").len(), 3, "one RETURNING per mutation");
    }

    #[test]
    fn table_constraints_are_nodes() {
        let parsed = Parsed::fixture("sql_constraints.sql");
        assert_eq!(parsed.errors(), 0);
        let constraints = parsed.of_kind("table_constraint");
        assert_eq!(constraints.len(), 4, "three in CREATE TABLE and one added by ALTER TABLE");
        let names: Vec<&str> = constraints.iter().map(|&index| parsed.text(index).split_whitespace().next().unwrap()).collect();
        assert_eq!(names, ["CONSTRAINT", "FOREIGN", "CHECK", "CONSTRAINT"]);
        let actions = parsed.of_kind("referential_action");
        assert_eq!(actions.len(), 3, "two ON DELETE/ON UPDATE clauses plus the named foreign key's");
    }

    #[test]
    fn every_prefix_of_the_fixtures_parses_or_errors_without_panicking() {
        for name in ["sql_cte_window.sql", "sql_upsert.sql", "sql_constraints.sql"] {
            let parsed = Parsed::fixture(name);
            for end in (0..parsed.src.len()).step_by(7).chain(std::iter::once(parsed.src.len())) {
                let mut workspace = Workspace::default();
                workspace.parse(&parsed.src[..end], &super::SQL, &Budget::default()).unwrap();
            }
        }
    }
}
