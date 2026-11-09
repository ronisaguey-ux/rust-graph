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
//! `LIKE`/`ILIKE`, `IS [NOT] NULL`/`DISTINCT FROM`, `INTERVAL`, `EXISTS`,
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

    _statement: (select_statement | insert_statement | update_statement | delete_statement | create_table_statement | create_view_statement | create_index_statement | alter_statement | drop_statement | with_statement) [";"],

    with_statement: "WITH" ["RECURSIVE"] (common_table_expression %? ",") select_statement,
    common_table_expression: _entity "AS" paren_expression,

    select_statement: "SELECT" ["DISTINCT" | "ALL"] (select_item %? ",") [from_clause] [where_clause] [group_clause] [having_clause] [order_clause] [limit_clause] [set_operator select_statement],
    select_item: "*" | _expression ["AS"] [alias],
    alias: _entity,
    set_operator: "UNION" ["ALL"] | "INTERSECT" | "EXCEPT",

    from_clause: "FROM" (table_reference %? ","),
    table_reference: table_name [["AS"] alias] (join_clause)*,
    table_name: _entity [call_arguments] | paren_expression,
    join_clause: [("INNER" | "LEFT" | "RIGHT" | "FULL" | "CROSS") ["OUTER"]] "JOIN" table_name [["AS"] alias] [join_condition],
    join_condition: "ON" _expression | "USING" column_list,

    where_clause: "WHERE" _expression,
    group_clause: "GROUP" "BY" (_expression %? ","),
    having_clause: "HAVING" _expression,
    order_clause: "ORDER" "BY" (order_item %? ","),
    order_item: _expression ["ASC" | "DESC"],
    limit_clause: "LIMIT" NUMBER ["OFFSET" NUMBER],

    insert_statement: "INSERT" "INTO" table_name [column_list] ("VALUES" (value_row %? ",") | select_statement),
    value_row: "(" (_expression %? ",") ")",
    update_statement: "UPDATE" table_name "SET" (assignment %? ",") [where_clause],
    assignment: _column "=" _expression,
    delete_statement: "DELETE" "FROM" table_name [where_clause],

    create_table_statement: "CREATE" "TABLE" ["IF" "NOT" "EXISTS"] _entity [table_body | "AS" select_statement],
    create_view_statement: "CREATE" "VIEW" _entity "AS" select_statement,
    create_index_statement: "CREATE" ["UNIQUE"] "INDEX" ["IF" "NOT" "EXISTS"] _entity "ON" _entity column_list,
    table_body: "(" (column_definition %? ",") ")",
    column_definition: _column data_type (column_constraint)*,
    data_type: _type_name [type_arguments] ["WITH" "TIME" "ZONE"],
    _type_name: IDENT | QUOTED_IDENT | "TEXT",
    type_arguments: "(" (NUMBER %? ",") ")",
    check_body: "(" _expression ")",
    column_constraint:
        "PRIMARY" "KEY"
        | "NOT" "NULL"
        | "NULL"
        | "UNIQUE"
        | "REFERENCES" _entity [column_list]
        | "DEFAULT" _expression
        | "CHECK" check_body
        | "CONSTRAINT" _entity,

    alter_statement: "ALTER" "TABLE" _entity (alter_action %? ","),
    alter_action: "ADD" ["COLUMN"] column_definition | "DROP" ["COLUMN"] _column | "RENAME" ["COLUMN"] _column "TO" _column,
    drop_statement: "DROP" ("TABLE" | "VIEW" | "INDEX") ["IF" "EXISTS"] _entity ["CASCADE"],

    _expression: precedence _primary {
        left "OR" => logical_expression,
        left "AND" => logical_expression,
        left "=" | "!=" | "<>" | "<" | ">" | "<=" | ">=" => comparison,
        postfix "IS" ["NOT"] ["NULL"] => is_null,
        left "NOT" "IN" => comparison,
        left "IN" => comparison,
        left "NOT" "LIKE" => comparison,
        left "LIKE" | "BETWEEN" => comparison,
        left "||" => concatenation,
        left "+" | "-" => arithmetic,
        left "*" | "/" | "%" => arithmetic,
        prefix "-" | "+" | "NOT" => unary,
        postfix call_arguments => call,
        postfix member_expression => member,
    },

    member_expression: "." _column,

    _primary:
        literal
        | _entity
        | paren_expression
        | case_expression
        | cast_expression
        | "EXISTS" paren_expression
        | interval_expression,

    paren_expression: "(" (select_statement | _expression ("," _expression)*) ")",
    column_list: "(" (_column %? ",") ")",
    case_expression: "CASE" [ _expression] case_when+ ["ELSE" _expression] "END",
    case_when: "WHEN" _expression "THEN" _expression,
    cast_expression: "CAST" cast_body,
    cast_body: "(" _expression "AS" data_type ")",
    interval_expression: "INTERVAL" (STRING | NUMBER) _interval_unit,
    _interval_unit: IDENT,

    call_arguments: "(" ["DISTINCT"] [call_item ("," call_item)*] ")",
    call_item: "*" | _expression,
    literal: NUMBER | STRING | "NULL",
    _entity: IDENT | QUOTED_IDENT,
    _column: IDENT | QUOTED_IDENT,
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

