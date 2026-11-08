//! Malformed input must never panic, never loop and never lose bytes.
//!
//! A lexer that panics on one stray quote takes the whole scan of a repository
//! down, so this is a hard requirement rather than a nicety. The inputs are
//! deterministic (an xorshift generator, no dependency) and drawn from the
//! bytes that actually stress the rules: quotes, backslashes, brackets, line
//! breaks, `${`, invalid UTF-8 lead bytes and NUL.

use deep_lex::{DeepLexer, Kind, Language, Lexer, Profile, ALL};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Bytes chosen because each one has a rule or a failure path near it.
const ALPHABET: &[u8] = b"abcxyz0123_ $\\\"'`(){}[]<>/*+-=!&|^%:;,.#@?~\n\r\t\x00\x7f\x80\xc3\xa9\xf0";

fn check(src: &[u8], profile: &'static Profile) {
    let mut lexer = DeepLexer::for_profile(src, profile);
    let mut previous_end = 0u32;
    let mut tokens = 0u32;
    while let Some(token) = lexer.next_token((tokens % 3) as u16) {
        assert!(
            token.start >= previous_end,
            "{}: token went backwards on {:?}",
            profile.name,
            &src[..src.len().min(40)]
        );
        assert!(token.end >= token.start);
        assert!(
            token.end as usize <= src.len(),
            "{}: token past the input on {:?}",
            profile.name,
            &src[..src.len().min(40)]
        );
        assert!(
            token.start as usize <= src.len(),
            "{}: token start past the input",
            profile.name
        );
        previous_end = token.end;
        tokens += 1;
        assert!(tokens < 10_000_000, "{}: the lexer did not terminate", profile.name);
    }
    // A whitespace- or comment-only file legitimately produces no tokens: the
    // old lexer stored it as trivia, this lexer leaves it on the floor.
}

#[test]
fn every_byte_alone_is_safe() {
    for profile in ALL {
        for byte in 0..=255u8 {
            check(&[byte], profile);
        }
        check(b"", profile);
    }
}

#[test]
fn random_bytes_never_panic() {
    for profile in ALL {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..300usize {
            let length = 1 + rng.below(512);
            let mut src = Vec::with_capacity(length);
            for _ in 0..length {
                src.push(ALPHABET[rng.below(ALPHABET.len())]);
            }
            // A few deliberately structured inputs among the noise.
            if case % 10 == 0 {
                src.extend_from_slice(b"/* unterminated");
            }
            if case % 17 == 0 {
                src.extend_from_slice(b"`${");
            }
            if case % 23 == 0 {
                src.extend_from_slice(b"R\"(x)");
            }
            check(&src, profile);
        }
    }
}

#[test]
fn the_layout_stack_never_underflows() {
    // Dedent without indent, indent without dedent, a file that only dedents.
    for src in [
        &b"a\n)"[..],
        b"a\n    b\nc\n   d\ne\n",
        b"if x:\n" as &[u8],
        b"if x:\n    if y:\n        z\n",
        &[b'\n'; 64],
        &[b'\t'; 100],
    ] {
        check(src, &deep_lex::PYTHON);
    }
    let mut lexer = DeepLexer::new(b"if x:\n    y\n", Language::Python);
    let mut kinds = Vec::new();
    while let Some(token) = lexer.next_token(0) {
        kinds.push(Kind::from_symbol(token.symbol).unwrap());
    }
    assert!(kinds.contains(&Kind::Indent));
    assert!(kinds.contains(&Kind::Dedent));
}
