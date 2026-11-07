//! JSON, written by hand on the runtime: the reference that Slice 2's `grammar!`
//! must reproduce node for node. Kind names follow tree-sitter-json where one exists.
//!
//! ```text
//! document: value*
//! value:    object | array | string | number | true | false | null
//! object:   '{' (pair (',' pair)*)? '}'
//! pair:     string ':' value
//! array:    '[' (value (',' value)*)? ']'
//! ```
//! Comments (`//`, `/* */`) are trivia, as in tree-sitter-json.

use super::lexer::Cursor;
use super::parser::Parser;
use super::syntax::{Class, Kind, KindInfo, Language, COMMENT, EOF, FIRST_LANGUAGE_KIND, UNKNOWN, WHITESPACE};

pub const LBRACE: Kind = FIRST_LANGUAGE_KIND;
pub const RBRACE: Kind = LBRACE + 1;
pub const LBRACKET: Kind = LBRACE + 2;
pub const RBRACKET: Kind = LBRACE + 3;
pub const COLON: Kind = LBRACE + 4;
pub const COMMA: Kind = LBRACE + 5;
pub const STRING: Kind = LBRACE + 6;
pub const NUMBER: Kind = LBRACE + 7;
pub const TRUE: Kind = LBRACE + 8;
pub const FALSE: Kind = LBRACE + 9;
pub const NULL: Kind = LBRACE + 10;
/// A string with no closing quote on its line.
pub const BAD_STRING: Kind = LBRACE + 11;
/// A bare word that is not `true`, `false` or `null`.
pub const WORD: Kind = LBRACE + 12;
pub const DOCUMENT: Kind = LBRACE + 13;
pub const OBJECT: Kind = LBRACE + 14;
pub const ARRAY: Kind = LBRACE + 15;
pub const PAIR: Kind = LBRACE + 16;

const BRACES: u8 = 0;
const BRACKETS: u8 = 1;

const KINDS: &[KindInfo] = &[
    KindInfo { name: "{", class: Class::Open(BRACES) },
    KindInfo { name: "}", class: Class::Close(BRACES) },
    KindInfo { name: "[", class: Class::Open(BRACKETS) },
    KindInfo { name: "]", class: Class::Close(BRACKETS) },
    KindInfo { name: ":", class: Class::Token },
    KindInfo { name: ",", class: Class::Token },
    KindInfo { name: "string", class: Class::Token },
    KindInfo { name: "number", class: Class::Token },
    KindInfo { name: "true", class: Class::Token },
    KindInfo { name: "false", class: Class::Token },
    KindInfo { name: "null", class: Class::Token },
    KindInfo { name: "bad_string", class: Class::BadToken },
    KindInfo { name: "word", class: Class::BadToken },
    KindInfo { name: "document", class: Class::Node },
    KindInfo { name: "object", class: Class::Node },
    KindInfo { name: "array", class: Class::Node },
    KindInfo { name: "pair", class: Class::Node },
];

pub static JSON: Language = Language { name: "json", root: DOCUMENT, kinds: KINDS, lex, parse: document };

const VALUE_START: &[Kind] = &[LBRACE, LBRACKET, STRING, NUMBER, TRUE, FALSE, NULL];

fn lex(cursor: &mut Cursor) -> Kind {
    match cursor.peek() {
        b' ' | b'\t' | b'\n' | b'\r' => {
            cursor.eat_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
            WHITESPACE
        }
        b'{' => single(cursor, LBRACE),
        b'}' => single(cursor, RBRACE),
        b'[' => single(cursor, LBRACKET),
        b']' => single(cursor, RBRACKET),
        b':' => single(cursor, COLON),
        b',' => single(cursor, COMMA),
        b'"' => {
            cursor.bump();
            if cursor.quoted(b'"', true) { STRING } else { BAD_STRING }
        }
        b'-' | b'0'..=b'9' => number(cursor),
        b'/' if cursor.peek_at(1) == b'/' => {
            cursor.eat_while(|byte| byte != b'\n');
            COMMENT
        }
        b'/' if cursor.peek_at(1) == b'*' => {
            cursor.bump_n(2);
            while !cursor.at_end() && !cursor.starts_with(b"*/") {
                cursor.bump();
            }
            cursor.bump_n(2);
            COMMENT
        }
        byte if byte.is_ascii_alphabetic() || byte == b'_' => {
            let word = cursor.rest();
            let length = cursor.eat_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
            match &word[..length] {
                b"true" => TRUE,
                b"false" => FALSE,
                b"null" => NULL,
                _ => WORD,
            }
        }
        // Not advancing makes the core take one character as UNKNOWN.
        _ => UNKNOWN,
    }
}

fn single(cursor: &mut Cursor, kind: Kind) -> Kind {
    cursor.bump();
    kind
}

/// `-? digits ('.' digits)? ([eE] [+-]? digits)?`, leniently: leading zeros pass.
/// A lone `-` is not a number, and the core takes it as UNKNOWN.
fn number(cursor: &mut Cursor) -> Kind {
    let start = cursor.pos();
    cursor.eat(b'-');
    if cursor.eat_while(|byte| byte.is_ascii_digit()) == 0 {
        debug_assert!(cursor.pos() > start, "a number starts with `-` or a digit");
        return WORD;
    }
    if cursor.peek() == b'.' && cursor.peek_at(1).is_ascii_digit() {
        cursor.bump();
        cursor.eat_while(|byte| byte.is_ascii_digit());
    }
    if matches!(cursor.peek(), b'e' | b'E') {
        let sign = usize::from(matches!(cursor.peek_at(1), b'+' | b'-'));
        if cursor.peek_at(1 + sign).is_ascii_digit() {
            cursor.bump_n(1 + sign);
            cursor.eat_while(|byte| byte.is_ascii_digit());
        }
    }
    NUMBER
}

fn document(p: &mut Parser) {
    while !p.at_end() {
        if p.at_any(VALUE_START) {
            value(p);
        } else {
            p.error_until(VALUE_START);
        }
    }
}

fn value(p: &mut Parser) {
    match p.current() {
        LBRACE => {
            p.group(OBJECT, object);
        }
        LBRACKET => {
            p.group(ARRAY, array);
        }
        STRING | NUMBER | TRUE | FALSE | NULL => p.bump(),
        _ => p.error_one(),
    }
}

/// The body of `{...}`, which sees `EOF` at its own `}`.
fn object(p: &mut Parser) {
    while !p.at_end() {
        if p.at(STRING) && p.nth(1) == COLON {
            pair(p);
        } else if p.at(COMMA) {
            // An empty member: `{,}` or `{"a":1,,"b":2}`.
            p.error_one();
            continue;
        } else {
            // Not a member: `{"b" "c"}` makes one ERROR of both strings.
            p.error_until(&[COMMA]);
        }
        if !separator(p) {
            break;
        }
    }
}

fn pair(p: &mut Parser) {
    let marker = p.open();
    p.bump();
    p.bump();
    if p.at_any(VALUE_START) {
        value(p);
    } else if p.at_end() || p.at(COMMA) {
        p.missing();
    } else {
        p.error_until(&[COMMA]);
    }
    p.close(marker, PAIR);
}

/// The body of `[...]`.
fn array(p: &mut Parser) {
    while !p.at_end() {
        if p.at_any(VALUE_START) {
            value(p);
        } else if p.at(COMMA) {
            p.error_one();
            continue;
        } else {
            p.error_until(&[COMMA]);
        }
        if !separator(p) {
            break;
        }
    }
}

/// After a member: a comma and more to come, or the end. Anything else up to the
/// next comma is an error (`[1, 2 3, 4]` makes `3` one). A trailing comma is an
/// error too. False when the group is done.
fn separator(p: &mut Parser) -> bool {
    if p.at_end() {
        return false;
    }
    if !p.at(COMMA) {
        p.error_until(&[COMMA]);
        if p.at_end() {
            return false;
        }
    }
    if p.nth(1) == EOF {
        p.error_one();
        return false;
    }
    p.bump();
    true
}
