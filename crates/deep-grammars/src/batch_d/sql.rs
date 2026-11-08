//! SQL: the statements a scanner meets in a schema or a migration, and the
//! expressions they hold.
//!
//! Keywords are matched without regard to case: the lexer folds an ASCII word
//! to upper case before the keyword table, so `select` and `SELECT` are one
//! token. Quoted identifiers keep their case; `"user"` is an identifier, not
//! the keyword `USER`.
//!
//! What is missing, named here rather than accepted silently:
//!
//! - **Dialect syntax.** No `RETURNING`, no `ON CONFLICT`, no window frames, no
//!   `MERGE`, no procedural bodies (`BEGIN ... END`, `$$ ... $$` is one string
//!   token). A statement outside this common subset is an error inside its own
//!   subtree, not a silently shortened tree.
//! - **Types are words.** `VARCHAR(255)`, `NUMERIC(10, 2)` and `TIMESTAMP WITH
//!   TIME ZONE` are modelled as a name with optional parenthesised arguments
//!   plus the standard suffix words; an arbitrary dialect type is not.
//! - **A subquery is an expression only where a parenthesised `SELECT` is.**
//!   `IN (SELECT ...)` and `(SELECT ...) AS x` are modelled; a bare `SELECT`
//!   inside an expression is not.

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
        IDENT = "identifier",
        NUMBER = "number",
        STRING = "string",
        BAD_STRING = "bad_string" bad,
        /// A quoted identifier: `"users"`, `` `users` ``, `[users]`.
        QUOTED_IDENT = "quoted_identifier",

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
        KW_TEXT = "TEXT",
        KW_TIME = "TIME",
        KW_ZONE = "ZONE",
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
            if lex::quoted(c, b'\'') {
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
        b'[' => {
            c.bump();
            c.eat_while(|byte| byte != b']' && byte != b'\n');
            c.eat(b']');
            QUOTED_IDENT
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
            keyword(&lex::upper(&start[..length]))
        }
        _ => UNKNOWN,
    }
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
        b"ON" => KW_ON, b"USING" => KW_USING, b"UNION" => KW_UNION,
        b"INTERSECT" => KW_INTERSECT, b"EXCEPT" => KW_EXCEPT, b"INSERT" => KW_INSERT,
        b"INTO" => KW_INTO, b"VALUES" => KW_VALUES, b"UPDATE" => KW_UPDATE,
        b"SET" => KW_SET, b"DELETE" => KW_DELETE, b"CREATE" => KW_CREATE,
        b"TABLE" => KW_TABLE, b"VIEW" => KW_VIEW, b"INDEX" => KW_INDEX,
        b"ALTER" => KW_ALTER, b"DROP" => KW_DROP, b"ADD" => KW_ADD,
        b"COLUMN" => KW_COLUMN, b"RENAME" => KW_RENAME, b"TO" => KW_TO,
        b"PRIMARY" => KW_PRIMARY, b"KEY" => KW_KEY, b"FOREIGN" => KW_FOREIGN,
        b"REFERENCES" => KW_REFERENCES, b"UNIQUE" => KW_UNIQUE, b"NOT" => KW_NOT,
        b"NULL" => KW_NULL, b"DEFAULT" => KW_DEFAULT, b"CHECK" => KW_CHECK,
        b"CONSTRAINT" => KW_CONSTRAINT, b"AND" => KW_AND, b"OR" => KW_OR,
        b"IN" => KW_IN, b"IS" => KW_IS, b"LIKE" => KW_LIKE,
        b"BETWEEN" => KW_BETWEEN, b"EXISTS" => KW_EXISTS, b"CASE" => KW_CASE,
        b"WHEN" => KW_WHEN, b"THEN" => KW_THEN, b"ELSE" => KW_ELSE,
        b"END" => KW_END, b"CAST" => KW_CAST, b"INTERVAL" => KW_INTERVAL,
        b"WITH" => KW_WITH, b"RECURSIVE" => KW_RECURSIVE, b"IF" => KW_IF,
        b"CASCADE" => KW_CASCADE, b"TEXT" => KW_TEXT, b"TIME" => KW_TIME,
        b"ZONE" => KW_ZONE,
        _ => IDENT,
    }
}
