//! The operator fast path must agree with the operator tables.
//!
//! `lex_code` answers a one-byte operator from [`deep_lex::Kind`]'s match and
//! only falls back to the table scan when the next byte can continue an
//! operator. That shortcut is only safe if the tables and the shortcut describe
//! the same language, so this walks every table entry and lexes it.

use deep_lex::{tokenize, Kind, Language};

/// The kind of the first token of `bytes`, or `None` for "no token".
fn first_kind(bytes: &[u8], lang: Language) -> Option<Kind> {
    tokenize(bytes, lang).first().map(|token| {
        Kind::from_symbol(token.symbol).unwrap_or_else(|| panic!("bytes {bytes:?} lost its symbol"))
    })
}

#[test]
fn every_shared_operator_lexes_as_its_table_entry() {
    for &(bytes, kind) in deep_lex::OPERATORS {
        assert_eq!(
            first_kind(bytes, Language::Generic),
            Some(kind),
            "shared operator {:?}",
            std::str::from_utf8(bytes).unwrap()
        );
    }
}

#[test]
fn every_python_operator_lexes_as_its_table_entry() {
    for &(bytes, kind) in deep_lex::kind::PYTHON_OPERATORS {
        assert_eq!(
            first_kind(bytes, Language::Python),
            Some(kind),
            "Python operator {:?}",
            std::str::from_utf8(bytes).unwrap()
        );
    }
}

#[test]
fn every_javascript_operator_lexes_as_its_table_entry() {
    for &(bytes, kind) in deep_lex::kind::JAVASCRIPT_OPERATORS {
        assert_eq!(
            first_kind(bytes, Language::JavaScript),
            Some(kind),
            "JavaScript operator {:?}",
            std::str::from_utf8(bytes).unwrap()
        );
    }
}

#[test]
fn the_single_byte_shortcut_covers_every_single_byte_entry() {
    for &(bytes, kind) in deep_lex::OPERATORS {
        if bytes.len() == 1 {
            assert_eq!(deep_lex::kind::single(bytes[0]), Some(kind), "{bytes:?}");
            assert!(!deep_lex::kind::starts_multi(bytes[0], None));
        }
    }
    // A one-byte operator followed by a byte no operator can use takes the
    // shortcut: no table entry starts with `{`.
    for b in [b'{', b'}', b'(', b')', b'[', b']', b',', b';', b'~', b'$', b'#'] {
        assert!(!deep_lex::kind::starts_multi(b, Some(b'x')));
    }
    // And a real continuation is not missed.
    assert!(deep_lex::kind::starts_multi(b'=', Some(b'=')));
    assert!(deep_lex::kind::starts_multi(b'>', Some(b'>')));
    assert!(deep_lex::kind::starts_multi(b':', Some(b':')));
    assert!(!deep_lex::kind::starts_multi(b':', Some(b' ')));
    assert!(!deep_lex::kind::starts_multi(b'=', Some(b' ')));
}
